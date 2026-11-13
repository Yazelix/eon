mod legacy;

use crate::config::{Config, SlotConfig};
use crate::launch::{Failure, atomic_write, ensure_parent, io_failure};
use crate::run::random;
use crate::session::state_dir;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SECOND: u64 = 1_000_000_000;
const TEMP_PREFIX: &str = ".ticket-";
const NESTING_TOKEN: &str = "ROTA_SLOT_TOKEN";

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub enum Options<'a> {
    List { json: bool },
    Run(RunOptions<'a>),
}

pub struct RunOptions<'a> {
    class: Option<&'a str>,
    front: bool,
    quiet_exempt: bool,
    argv: &'a [String],
}

pub fn parse(args: &[String]) -> Result<Options<'_>, Failure> {
    let bad = || {
        (2, "usage: rota slot run [--class NAME] [--quiet-exempt] [--front] -- COMMAND... | rota slot ls [--json]".into())
    };
    match args {
        [ls] if ls == "ls" => return Ok(Options::List { json: false }),
        [ls, json] if ls == "ls" && json == "--json" => return Ok(Options::List { json: true }),
        [run, ..] if run == "run" => (),
        _ => return Err(bad()),
    }
    let mut rest = &args[1..];
    let mut options = RunOptions {
        class: None,
        front: false,
        quiet_exempt: false,
        argv: &[],
    };
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match arg.as_str() {
            "--" if rest.first().is_some_and(|word| !word.is_empty()) => {
                options.argv = rest;
                return Ok(Options::Run(options));
            }
            "--class" if options.class.is_none() => {
                let (name, tail) = rest.split_first().ok_or_else(bad)?;
                options.class = Some(name);
                rest = tail;
            }
            "--front" if !options.front => options.front = true,
            "--quiet-exempt" if !options.quiet_exempt => options.quiet_exempt = true,
            _ => return Err(bad()),
        }
    }
    Err(bad())
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn pid_namespace(pid: u32) -> Option<u64> {
    fs::metadata(format!("/proc/{pid}/ns/pid"))
        .ok()
        .map(|metadata| metadata.ino())
}

fn proc_start(pid: u32) -> io::Result<Option<u64>> {
    let stat = match fs::read(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(e) if e.kind() == ErrorKind::NotFound || e.raw_os_error() == Some(libc::ESRCH) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    // The comm field can contain spaces and parentheses.
    let fields = stat.rsplit(|byte| *byte == b')').next().unwrap_or_default();
    let mut fields = std::str::from_utf8(fields)
        .map_err(io::Error::other)?
        .split_ascii_whitespace();
    if matches!(fields.next(), Some("Z" | "X" | "x")) {
        return Ok(None);
    }
    fields
        .nth(18)
        .and_then(|field| field.parse().ok())
        .map(Some)
        .ok_or_else(|| io::Error::other("invalid process start time"))
}

fn nix_option_tail<'a>(option: &str, rest: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let values = match option {
        "--option" | "--arg" | "--argstr" => 2,
        "--store"
        | "--log-format"
        | "--extra-experimental-features"
        | "--experimental-features"
        | "--max-jobs"
        | "-j"
        | "--cores"
        | "--out-link"
        | "-o"
        | "--file"
        | "-f"
        | "--expr" => 1,
        "-v" | "--verbose" | "-q" | "--quiet" | "--debug" | "--json" | "-L"
        | "--print-build-logs" | "--no-link" | "--impure" | "--offline" | "--refresh" => 0,
        _ => return None,
    };
    rest.get(values..)
}

fn literal_shell_words(script: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = None::<String>;
    let mut quote = None;
    for character in script.trim().chars() {
        match (quote, character) {
            (Some(active), c) if active == c => quote = None,
            (Some('"'), '$' | '`' | '\\') => return None,
            (Some(_), c) => word.get_or_insert_with(String::new).push(c),
            (None, '\'' | '"') => {
                quote = Some(character);
                word.get_or_insert_with(String::new);
            }
            (
                None,
                '\n' | '\r' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '$' | '`' | '\\' | '*'
                | '?' | '[' | ']' | '{' | '}' | '~',
            ) => return None,
            (None, '#') if word.is_none() => return None,
            (None, c) if c.is_ascii_whitespace() => words.extend(word.take()),
            (None, c) => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    quote.is_none().then_some(words)
}

fn nix_without_slot(argv: &[&str]) -> bool {
    let Some((executable, mut args)) = argv.split_first() else {
        return false;
    };
    match Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some("timeout") => {
            return matches!(args, [duration, tail @ ..]
            if crate::run::parse_timeout(duration).is_some() && nix_without_slot(tail));
        }
        Some("sh" | "bash") => {
            let ["-c" | "-ec", script, ..] = args else {
                return false;
            };
            let words = literal_shell_words(script).or_else(|| {
                let (before, after) = script.split_once("&&")?;
                let prefix = literal_shell_words(before)?;
                if prefix.len() == 2 && prefix[0] == "cd" {
                    literal_shell_words(after)
                } else {
                    None
                }
            });
            return words.is_some_and(|words| {
                let words: Vec<_> = words.iter().map(String::as_str).collect();
                nix_without_slot(words.strip_prefix(&["exec"]).unwrap_or(&words))
            });
        }
        Some("nix") => (),
        _ => return false,
    }
    while let Some((&option, rest)) = args.split_first().filter(|(arg, _)| arg.starts_with('-')) {
        let Some(rest) = nix_option_tail(option, rest) else {
            return false;
        };
        args = rest;
    }
    match args {
        ["eval" | "path-info", ..] => true,
        ["build", rest @ ..] => {
            let mut rest = rest;
            while let Some((&arg, tail)) = rest.split_first() {
                if arg == "--dry-run" {
                    return true;
                }
                if arg.starts_with('-') {
                    let Some(tail) = nix_option_tail(arg, tail) else {
                        return false;
                    };
                    rest = tail;
                } else {
                    rest = tail;
                }
            }
            false
        }
        _ => false,
    }
}

fn argv_matches(words: &[&str], pattern: &[&str]) -> bool {
    words.windows(pattern.len()).any(|window| {
        let [executable, args @ ..] = window else {
            return false;
        };
        (*executable == pattern[0]
            || (!pattern[0].contains('/')
                && Path::new(executable)
                    .file_name()
                    .is_some_and(|name| name == pattern[0])))
            && args == &pattern[1..]
    })
}

fn class_for<'a>(config: &'a SlotConfig, options: &RunOptions<'_>) -> Result<&'a str, Failure> {
    if let Some(class) = options.class {
        if config.classes.is_empty() && matches!(class, "run" | "nix") {
            return Ok(if class == "run" { "run" } else { "nix" });
        }
        return config
            .classes
            .iter()
            .find(|entry| entry.name == class)
            .map(|entry| entry.name.as_str())
            .ok_or_else(|| (2, format!("unknown slot class {class:?}")));
    }
    let argv: Vec<_> = options.argv.iter().map(String::as_str).collect();
    let joined = options.argv.join(" ");
    let words: Vec<_> = joined
        .split(|c: char| c.is_whitespace() || ";&|()<>\x27\"".contains(c))
        .filter(|word| !word.is_empty())
        .collect();
    let matches = |pattern: &[&str]| argv_matches(&argv, pattern) || argv_matches(&words, pattern);
    Ok(if config.classes.is_empty() {
        if matches(&["nix", "build"]) || matches(&["nix", "profile"]) {
            "nix"
        } else {
            "run"
        }
    } else {
        config
            .classes
            .iter()
            .find(|entry| {
                entry.match_argv.is_empty()
                    || entry.match_argv.iter().any(|pattern| {
                        let pattern: Vec<_> = pattern.iter().map(String::as_str).collect();
                        matches(&pattern)
                    })
            })
            .map(|entry| entry.name.as_str())
            .unwrap_or("run")
    })
}

#[derive(Deserialize, Serialize)]
struct Ticket {
    queued_ns: u64,
    pid: u32,
    class: String,
    cwd: Option<String>,
    argv: Vec<String>,
    #[serde(default)]
    front: bool,
    #[serde(default)]
    quiet_exempt: bool,
    #[serde(skip_deserializing, skip_serializing_if = "is_false")]
    legacy: bool,
    #[serde(skip_deserializing, skip_serializing_if = "is_false")]
    malformed: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Ticket {
    fn key(&self) -> (u64, u32) {
        (self.queued_ns, self.pid)
    }
}

#[derive(Deserialize, Serialize)]
struct Holder {
    slot: u16,
    granted_ns: u64,
    process_start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid_namespace: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nesting_token_hash: Option<String>,
    #[serde(flatten)]
    ticket: Ticket,
}

impl Holder {
    fn matches_process(&self, pid: u32) -> bool {
        let local_pid = match self.pid_namespace {
            None => Some(pid),
            Some(namespace) if pid_namespace(pid) == Some(namespace) => {
                fs::read_to_string(format!("/proc/{pid}/status"))
                    .ok()
                    .and_then(|status| {
                        status.lines().find_map(|line| {
                            line.strip_prefix("NSpid:")?
                                .split_ascii_whitespace()
                                .last()?
                                .parse()
                                .ok()
                        })
                    })
            }
            Some(_) => None,
        };
        local_pid == Some(self.ticket.pid)
            && proc_start(pid).ok().flatten() == Some(self.process_start)
    }
}

struct Ranked<'a> {
    ticket: &'a Ticket,
    priority: u8,
    slots: Vec<u16>,
}

struct Policy<'a> {
    config: &'a SlotConfig,
    extra_until: u64,
    one_until: u64,
    quiet_until: u64,
    stalled: bool,
    yield_roots: Vec<PathBuf>,
    yield_ns: u64,
}

fn rank<'a>(
    tickets: impl IntoIterator<Item = &'a Ticket>,
    policy: &Policy<'_>,
    now: u64,
) -> Vec<Ranked<'a>> {
    let config = policy.config;
    let seconds = now / SECOND;
    let quiet = seconds < policy.quiet_until;
    let narrowed = policy.stalled || seconds < policy.one_until;
    let extra = seconds < policy.extra_until;
    let mut ranked: Vec<_> = tickets
        .into_iter()
        .map(|ticket| {
            let priority = if ticket.front || ticket.quiet_exempt {
                0
            } else if now.saturating_sub(ticket.queued_ns) < policy.yield_ns
                && ticket.cwd.as_ref().is_some_and(|cwd| {
                    policy
                        .yield_roots
                        .iter()
                        .any(|root| Path::new(cwd).starts_with(root))
                })
            {
                2
            } else {
                1
            };
            let mut slots = if quiet && !(ticket.quiet_exempt || ticket.malformed) {
                Vec::new()
            } else if narrowed {
                vec![1]
            } else if ticket.front || ticket.malformed {
                (1..=config.count).collect()
            } else if let Some(class) = config
                .classes
                .iter()
                .find(|class| class.name == ticket.class)
            {
                class.slots.clone()
            } else if ticket.class == "nix" {
                vec![2.min(config.count)]
            } else {
                (1..=config.count).collect()
            };
            if !narrowed && extra && !slots.is_empty() {
                slots.push(config.count + 1);
            }
            Ranked {
                ticket,
                priority,
                slots,
            }
        })
        .collect();
    ranked.sort_by_key(|item| (item.priority, item.ticket.key()));
    ranked
}

fn first_for_slot<'a>(ranked: &'a [Ranked<'a>], slot: u16) -> Option<&'a Ticket> {
    ranked
        .iter()
        .find(|item| item.slots.contains(&slot))
        .map(|item| item.ticket)
}

fn try_lock(file: &File) -> io::Result<bool> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(false),
        Err(e) => Err(e),
    }
}

type LockId = (libc::c_uint, libc::c_uint, u64);
type ObservedFlocks = HashMap<LockId, Option<u32>>;

fn observed_flocks() -> io::Result<ObservedFlocks> {
    Ok(fs::read_to_string("/proc/locks")?
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            if fields.nth(1)? != "FLOCK" {
                return None;
            }
            let owner = fields.nth(2)?.parse().ok();
            let mut identity = fields.next()?.split(':');
            Some((
                (
                    u32::from_str_radix(identity.next()?, 16).ok()?,
                    u32::from_str_radix(identity.next()?, 16).ok()?,
                    identity.next()?.parse().ok()?,
                ),
                owner,
            ))
        })
        .collect())
}

fn lock_id(metadata: &fs::Metadata) -> LockId {
    let dev = metadata.dev();
    (libc::major(dev), libc::minor(dev), metadata.ino())
}

struct Pending {
    path: PathBuf,
    _file: File,
}

impl Drop for Pending {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct Slots<'a> {
    config: &'a SlotConfig,
    root: PathBuf,
    queue: PathBuf,
    warned: HashSet<PathBuf>,
}

impl Slots<'_> {
    fn queue_lock(&self, create: bool) -> io::Result<Option<File>> {
        let path = self.queue.join(".lock");
        let lock = if create {
            ensure_parent(&path)?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)?
        } else {
            match File::open(path) {
                Ok(file) => file,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
        };
        FileExt::lock_exclusive(&lock)?;
        Ok(Some(lock))
    }

    fn refuse_queue(&self, error: impl std::fmt::Display) -> Failure {
        (
            3,
            format!(
                "cannot register a ticket in {}: {error}. Give rota write access to this queue directory",
                self.queue.display()
            ),
        )
    }

    fn enqueue(&self, ticket: &Ticket) -> io::Result<Pending> {
        let path = self
            .queue
            .join(format!("{}-{}.json", ticket.queued_ns, ticket.pid));
        let mut temp = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempfile_in(&self.queue)?;
        FileExt::lock_exclusive(temp.as_file())?;
        serde_json::to_writer(&mut temp, ticket)?;
        let file = temp.persist_noclobber(&path).map_err(|e| e.error)?;
        Ok(Pending { path, _file: file })
    }

    fn tickets(&mut self, observed: Option<&ObservedFlocks>) -> Vec<Ticket> {
        let mut tickets = legacy::tickets(self.config.legacy_dir.as_deref());
        let Ok(entries) = fs::read_dir(&self.queue) else {
            return tickets;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let key = name
                .strip_suffix(".json")
                .and_then(|stem| stem.split_once('-'))
                .and_then(|(ns, pid)| Some((ns.parse::<u64>().ok()?, pid.parse::<u32>().ok()?)));
            if (key.is_none() && (observed.is_some() || !name.starts_with(TEMP_PREFIX)))
                || !entry.file_type().is_ok_and(|kind| kind.is_file())
            {
                continue;
            }
            let path = entry.path();
            let Ok(file) = File::open(&path) else {
                continue;
            };
            let live = match observed {
                Some(locks) => file
                    .metadata()
                    .is_ok_and(|metadata| locks.contains_key(&lock_id(&metadata))),
                None => match try_lock(&file) {
                    Ok(false) => true,
                    Ok(true) => {
                        let _ = fs::remove_file(&path);
                        false
                    }
                    Err(_) => false,
                },
            };
            if !live {
                continue;
            }
            if let Some((queued_ns, pid)) = key {
                match serde_json::from_reader::<_, Ticket>(io::BufReader::new(file)) {
                    Ok(ticket) if ticket.key() == (queued_ns, pid) => tickets.push(ticket),
                    Err(e) if e.is_io() => continue,
                    _ => {
                        if self.warned.insert(path.clone()) {
                            let _ = writeln!(
                                io::stderr(),
                                "rota slot: malformed live ticket {}",
                                path.display()
                            );
                        }
                        tickets.push(Ticket {
                            queued_ns,
                            pid,
                            class: "unknown".into(),
                            cwd: None,
                            argv: Vec::new(),
                            front: false,
                            quiet_exempt: false,
                            legacy: false,
                            malformed: true,
                        });
                    }
                }
            }
        }
        tickets
    }

    fn slot_path(&self, slot: u16) -> Option<PathBuf> {
        if self.config.lock_files.is_empty() {
            Some(self.root.join(format!("slot-{slot}.lock")))
        } else {
            self.config.lock_files.get(usize::from(slot - 1)).cloned()
        }
    }

    fn open_slot(&self, slot: u16) -> io::Result<File> {
        let path = self
            .slot_path(slot)
            .ok_or_else(|| io::Error::other(format!("no configured lock file for slot {slot}")))?;
        match File::open(&path) {
            Err(e) if self.config.lock_files.is_empty() && e.kind() == ErrorKind::NotFound => {
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                {
                    Ok(_) => (),
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(e),
                }
                File::open(path)
            }
            result => result,
        }
    }

    fn holder_path(&self, slot: u16) -> PathBuf {
        self.root.join("holders").join(format!("{slot}.json"))
    }

    fn holder(&self, slot: u16) -> Option<(Holder, File)> {
        let file = File::open(self.holder_path(slot)).ok()?;
        let holder: Holder = serde_json::from_reader(io::BufReader::new(&file)).ok()?;
        (holder.slot == slot).then_some((holder, file))
    }

    fn nested(&self) -> bool {
        let Some((holder, file)) = std::env::var("ROTA_SLOT")
            .ok()
            .and_then(|slot| slot.parse::<u16>().ok())
            .and_then(|slot| self.holder(slot))
        else {
            return false;
        };
        std::env::var(NESTING_TOKEN).ok().is_some_and(|token| {
            holder.nesting_token_hash.as_deref() == Some(token_hash(&token).as_str())
                && FileExt::try_lock_shared(&file).is_err_and(|e| e.kind() == ErrorKind::WouldBlock)
        })
    }

    fn log(&self, event: &str, ticket: &Ticket, slot: Option<u16>, code: Option<u8>) {
        if let Err(e) = self.append_event(event, ticket, slot, code) {
            let _ = writeln!(io::stderr(), "rota slot: cannot log {event}: {e}");
        }
    }

    fn append_event(
        &self,
        event: &str,
        ticket: &Ticket,
        slot: Option<u16>,
        code: Option<u8>,
    ) -> io::Result<()> {
        let mut entry = serde_json::json!({
            "time": now_ns() / SECOND,
            "event": event,
            "pid": ticket.pid,
            "class": ticket.class,
            "folder": ticket.cwd,
            "command": ticket.argv.join(" ").chars().take(80).collect::<String>(),
            "front": ticket.front,
            "quiet_exempt": ticket.quiet_exempt,
        });
        if let Some(slot) = slot {
            entry["slot"] = slot.into();
        }
        if let Some(code) = code {
            entry["exit_code"] = code.into();
        }
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.root.join("log.jsonl"))?;
        let end = file.metadata()?.len();
        if let Err(error) = file.write_all(&line) {
            let _ = file.set_len(end);
            return Err(error);
        }
        Ok(())
    }
}

fn command(config: &SlotConfig, options: &RunOptions<'_>, slot: u16, token: &str) -> Command {
    let mut command = Command::new(&options.argv[0]);
    command
        .args(&options.argv[1..])
        .envs(&config.env)
        .env("ROTA_SLOT", slot.to_string())
        .env(NESTING_TOKEN, token);
    let increment = config.nice;
    if increment != 0 {
        // Apply nice in the child so the queue wrapper keeps its original priority.
        unsafe {
            command.pre_exec(move || {
                *libc::__errno_location() = 0;
                if libc::nice(increment) == -1 && *libc::__errno_location() != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command
}

fn run_queued(mut slots: Slots<'_>, options: &RunOptions<'_>) -> Result<u8, Failure> {
    let class = class_for(slots.config, options)?;
    let cwd = std::env::current_dir()
        .map_err(io_failure)?
        .to_string_lossy()
        .into_owned();
    let pid = process::id();
    let process_start = proc_start(pid)
        .map_err(io_failure)?
        .ok_or_else(|| io_failure("cannot read rota process identity"))?;
    let nesting_token = random()?;
    let (ticket, pending) = {
        let _queue = slots.queue_lock(true).map_err(|e| slots.refuse_queue(e))?;
        let ticket = Ticket {
            queued_ns: now_ns(),
            pid,
            class: class.into(),
            cwd: Some(cwd),
            argv: options.argv.to_vec(),
            front: options.front,
            quiet_exempt: options.quiet_exempt,
            legacy: false,
            malformed: false,
        };
        let pending = slots.enqueue(&ticket).map_err(|e| slots.refuse_queue(e))?;
        slots.log("queue", &ticket, None, None);
        (ticket, pending)
    };
    let poll = std::env::var("ROTA_SLOT_POLL_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .filter(|ms| (1..=5_000).contains(ms))
        .map_or(Duration::from_secs(5), Duration::from_millis);
    let (holder, slot_lock, holder_lock) = loop {
        {
            let _queue = slots.queue_lock(true).map_err(|e| slots.refuse_queue(e))?;
            let now = now_ns();
            let policy = legacy::policy(slots.config);
            let tickets = slots.tickets(None);
            let competitors = tickets
                .iter()
                .filter(|other| other.legacy || other.key() != ticket.key());
            let ranked = rank(competitors.chain(std::iter::once(&ticket)), &policy, now);
            let own = ranked
                .iter()
                .find(|item| std::ptr::eq(item.ticket, &ticket))
                .expect("rank includes the in-memory ticket");
            let mut grant = None;
            for &slot in &own.slots {
                if !first_for_slot(&ranked, slot)
                    .is_some_and(|first| std::ptr::eq(first, own.ticket))
                {
                    continue;
                }
                let lock = slots
                    .open_slot(slot)
                    .map_err(|e| io_failure(format!("open slot {slot}: {e}")))?;
                if try_lock(&lock).map_err(io_failure)? {
                    grant = Some((slot, lock));
                    break;
                }
            }
            if let Some((slot, lock)) = grant {
                let holder = Holder {
                    slot,
                    granted_ns: now_ns(),
                    process_start,
                    pid_namespace: pid_namespace(pid),
                    nesting_token_hash: Some(token_hash(&nesting_token)),
                    ticket,
                };
                let path = slots.holder_path(slot);
                let record = serde_json::to_vec(&holder).map_err(io_failure)?;
                let holder_lock = (|| {
                    atomic_write(&path, &record)?;
                    let file = File::open(&path)?;
                    FileExt::try_lock_exclusive(&file)?;
                    fs::remove_file(&pending.path)?;
                    Ok::<_, io::Error>(file)
                })()
                .map_err(|e| {
                    let _ = fs::remove_file(&path);
                    io_failure(format!("grant slot {slot}: {e}"))
                })?;
                slots.log("grant", &holder.ticket, Some(slot), None);
                drop(pending);
                break (holder, lock, holder_lock);
            }
        }
        std::thread::sleep(poll);
    };
    let slot = holder.slot;
    let _ = writeln!(io::stderr(), "rota slot: slot {slot}, pid {pid}");
    let status = command(slots.config, options, slot, &nesting_token)
        .status()
        .map(|status| {
            status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or_default()) as u8
        })
        .map_err(|e| io_failure(format!("cannot start {}: {e}", options.argv[0])));
    let cleanup = (|| {
        let _queue = slots.queue_lock(true)?;
        slots.log(
            "exit",
            &holder.ticket,
            Some(slot),
            Some(status.as_ref().copied().unwrap_or(1)),
        );
        let removed = fs::remove_file(slots.holder_path(slot));
        drop(holder_lock);
        drop(slot_lock);
        removed
    })();
    if let Err(e) = cleanup {
        let _ = writeln!(
            io::stderr(),
            "rota slot: cannot remove holder for slot {slot}: {e}"
        );
    }
    status
}

fn list(
    mut slots: Slots<'_>,
    json: bool,
    observe: impl FnOnce() -> io::Result<ObservedFlocks>,
    mut out: impl Write,
) -> Result<u8, Failure> {
    let queue = slots.queue_lock(false).ok().flatten();
    let locks = observe().map_err(|e| io_failure(format!("cannot read /proc/locks: {e}")))?;
    let now = now_ns();
    let policy = legacy::policy(slots.config);
    let tickets = slots.tickets(Some(&locks));
    let ranked = rank(&tickets, &policy, now);
    let mut holders = Vec::new();
    for slot in 1..=slots.config.count + 1 {
        if let Some(owner) = slots
            .slot_path(slot)
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|metadata| locks.get(&lock_id(&metadata)))
        {
            let holder = owner.and_then(|pid| {
                let (mut holder, _) = slots.holder(slot)?;
                if !holder.matches_process(pid) {
                    return None;
                }
                holder.ticket.pid = pid;
                holder.nesting_token_hash = None;
                Some(holder)
            });
            holders.push((slot, holder));
        }
    }
    drop(queue);
    if json {
        let holders: Vec<_> = holders
            .into_iter()
            .map(|(slot, holder)| {
                holder.map_or_else(
                    || Ok(serde_json::json!({"slot": slot})),
                    serde_json::to_value,
                )
            })
            .collect::<Result<_, _>>()
            .map_err(io_failure)?;
        let waiters: Vec<_> = ranked
            .iter()
            .map(|item| {
                let mut entry = serde_json::to_value(item.ticket)?;
                entry["slots"] = serde_json::to_value(&item.slots)?;
                Ok(entry)
            })
            .collect::<Result<_, serde_json::Error>>()
            .map_err(io_failure)?;
        writeln!(
            out,
            "{}",
            serde_json::json!({"holders": holders, "waiters": waiters})
        )
        .map_err(io_failure)?;
    } else {
        for (slot, holder) in holders {
            match holder {
                Some(holder) => writeln!(
                    out,
                    "slot {slot} held {}",
                    describe(&holder.ticket, now.saturating_sub(holder.granted_ns))
                ),
                None => writeln!(out, "slot {slot} occupied"),
            }
            .map_err(io_failure)?;
        }
        for item in ranked {
            writeln!(
                out,
                "waiting {} slots={:?}",
                describe(item.ticket, now.saturating_sub(item.ticket.queued_ns)),
                item.slots
            )
            .map_err(io_failure)?;
        }
    }
    Ok(0)
}

fn describe(ticket: &Ticket, age_ns: u64) -> String {
    format!(
        "pid={} class={} age={}s {} {}",
        ticket.pid,
        ticket.class,
        age_ns / SECOND,
        ticket.cwd.as_deref().unwrap_or("?"),
        ticket.argv.join(" ")
    )
}

pub fn run(config: &Config, options: Options<'_>) -> Result<u8, Failure> {
    let root = state_dir(&config.state_file).join("slots");
    let slots = Slots {
        config: &config.slots,
        queue: root.join("queue"),
        root,
        warned: HashSet::new(),
    };
    match options {
        Options::Run(options)
            if slots.nested()
                || (options.class.is_none()
                    && nix_without_slot(
                        &options.argv.iter().map(String::as_str).collect::<Vec<_>>(),
                    )) =>
        {
            Err(io_failure(
                Command::new(&options.argv[0])
                    .args(&options.argv[1..])
                    .exec(),
            ))
        }
        Options::Run(options) => run_queued(slots, &options),
        Options::List { json } => list(slots, json, observed_flocks, io::stdout().lock()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_fails_without_output_when_lock_table_is_unreadable() {
        let directory = tempfile::tempdir().unwrap();
        let config = SlotConfig::default();
        let slots = Slots {
            config: &config,
            root: directory.path().to_path_buf(),
            queue: directory.path().join("queue"),
            warned: HashSet::new(),
        };
        let mut out = Vec::new();
        let error = list(
            slots,
            true,
            || Err(ErrorKind::PermissionDenied.into()),
            &mut out,
        )
        .unwrap_err();
        assert_eq!(error.0, 1);
        assert!(error.1.contains("cannot read /proc/locks"));
        assert!(out.is_empty());
    }

    #[test]
    fn dry_run_word_must_be_an_option_to_a_nix_build() {
        for args in [
            &["nix", "build", "--out-link", "--dry-run"][..],
            &["nix", "build", "--", "--dry-run"],
            &["sh", "-c", "nix build --dry-run; cargo test"],
            &["sh", "-c", "cd . && nix build --dry-run && cargo test"],
            &["sh", "-c", "nix --option test 'value --dry-run' build"],
            &["bash", "-c", "nix build --dry-run $(cargo test)"],
        ] {
            assert!(!nix_without_slot(args));
        }
    }

    fn ticket(queued_ns: u64, class: &str, cwd: &str) -> Ticket {
        Ticket {
            queued_ns,
            pid: 42,
            class: class.into(),
            cwd: Some(cwd.into()),
            argv: Vec::new(),
            front: false,
            quiet_exempt: false,
            legacy: false,
            malformed: false,
        }
    }

    #[test]
    fn yield_priority_expires_at_the_default_cap() {
        let config = SlotConfig::default();
        let mut policy = legacy::policy(&config);
        policy.yield_roots.push("/projects/lower".into());
        let tickets = [
            ticket(SECOND, "run", "/projects/lower/subdir"),
            ticket(2 * SECOND, "run", "/projects/lower-neighbor"),
        ];
        let cap = tickets[0].queued_ns + 30 * 60 * SECOND;
        assert_eq!(
            first_for_slot(&rank(&tickets, &policy, cap - 1), 1)
                .unwrap()
                .queued_ns,
            tickets[1].queued_ns
        );
        assert_eq!(
            first_for_slot(&rank(&tickets, &policy, cap), 1)
                .unwrap()
                .queued_ns,
            tickets[0].queued_ns
        );
    }

    #[test]
    fn yielding_waiters_keep_fifo_without_a_competitor() {
        let config = SlotConfig::default();
        let mut policy = legacy::policy(&config);
        policy.yield_roots.push("/projects/lower".into());
        let tickets = [
            ticket(2 * SECOND, "run", "/projects/lower/a"),
            ticket(SECOND, "run", "/projects/lower/b"),
        ];
        let ranked = rank(&tickets, &policy, 3 * SECOND);
        assert_eq!(first_for_slot(&ranked, 1).unwrap().queued_ns, SECOND);
        assert_eq!(
            ranked.iter().map(|item| item.priority).collect::<Vec<_>>(),
            [2, 2]
        );
    }

    #[test]
    fn configured_yield_cap_and_empty_default() {
        let directory = tempfile::tempdir().unwrap();
        let config = SlotConfig {
            legacy_dir: Some(directory.path().into()),
            ..SlotConfig::default()
        };
        fs::write(directory.path().join("yield-roots"), "/projects/lower").unwrap();
        let cap_file = directory.path().join("yield-max-minutes");
        let tickets = [
            ticket(SECOND, "run", "/projects/lower"),
            ticket(2 * SECOND, "run", "/projects/higher"),
        ];
        for (text, minutes) in [("2\n", 2), ("\n", 30)] {
            fs::write(&cap_file, text).unwrap();
            let policy = legacy::policy(&config);
            let cap = SECOND + minutes * 60 * SECOND;
            assert_eq!(
                first_for_slot(&rank(&tickets, &policy, cap - 1), 1)
                    .unwrap()
                    .queued_ns,
                2 * SECOND
            );
            assert_eq!(
                first_for_slot(&rank(&tickets, &policy, cap), 1)
                    .unwrap()
                    .queued_ns,
                SECOND
            );
        }
    }

    #[test]
    fn quiet_exemption_jumps_older_eligible_waiter() {
        let config = SlotConfig::default();
        let policy = legacy::policy(&config);
        let mut tickets = [
            ticket(SECOND, "nix", "/projects/normal"),
            ticket(2 * SECOND, "nix", "/projects/normal"),
        ];
        tickets[1].quiet_exempt = true;
        let ranked = rank(&tickets, &policy, 3 * SECOND);
        assert!(first_for_slot(&ranked, 1).is_none());
        assert!(first_for_slot(&ranked, 2).unwrap().quiet_exempt);
    }

    #[test]
    fn policy_expires_at_the_epoch_second() {
        let config = SlotConfig::default();
        let mut policy = legacy::policy(&config);
        policy.quiet_until = 2;
        policy.one_until = 3;
        policy.extra_until = 4;
        let tickets = [ticket(0, "nix", "/projects/normal")];
        assert!(rank(&tickets, &policy, 2 * SECOND - 1)[0].slots.is_empty());
        assert_eq!(rank(&tickets, &policy, 2 * SECOND)[0].slots, [1]);
        assert_eq!(rank(&tickets, &policy, 3 * SECOND)[0].slots, [2, 3]);
        assert_eq!(rank(&tickets, &policy, 4 * SECOND)[0].slots, [2]);
    }
}
