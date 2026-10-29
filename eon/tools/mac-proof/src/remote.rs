//! `remote`: sends a tree to the Mac, runs `run` there and fetches the evidence.

use crate::step::{self, Log, POLL, format_duration, progress};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TRANSFER: Duration = Duration::from_secs(30 * 60);
/// What the host gets beyond the run's own limit: building the tool there,
/// stopping its last step and writing the summary.
const MARGIN: Duration = Duration::from_secs(30 * 60);
const NIX_PROFILE: &str = "/nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh";

pub enum Host {
    Ssh(String),
    /// This machine, with the directory standing in for the Mac's HOME.
    Local(PathBuf),
}

pub struct Options {
    pub host: Host,
    pub out: PathBuf,
    pub rev: Option<String>,
    pub timeout: Duration,
    pub dry_run: bool,
}

impl Host {
    /// A command that runs `script` from the host's HOME, in the SSH user's
    /// login shell (zsh by default on macOS) or, for a local host, in sh.
    fn shell(&self, script: &str) -> Command {
        match self {
            Host::Ssh(host) => {
                let mut command = Command::new("ssh");
                // Keepalives stop an idle NAT from dropping a long silent step.
                command.args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ServerAliveInterval=60",
                    host,
                    script,
                ]);
                command
            }
            Host::Local(home) => {
                let mut command = Command::new("sh");
                command.args(["-c", script]).current_dir(home);
                command
            }
        }
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn git(root: &Path, log: &mut Log, arguments: &[&str]) -> Result<String, String> {
    let output = step::succeed(
        log,
        Command::new("git").arg("-C").arg(root).args(arguments),
        Instant::now() + Duration::from_secs(600),
        true,
    )
    .map_err(|_| format!("git {} failed; see the log", arguments.join(" ")))?;
    Ok(String::from_utf8_lossy(&output).trim().to_owned())
}

/// The tree of `rev`, or without one, HEAD's tree for a clean checkout and
/// else the candidate tree with untracked files, written through a private
/// index. Returns (commit, tree, dirty).
fn identify(
    root: &Path,
    log: &mut Log,
    rev: Option<&str>,
) -> Result<(String, String, bool), String> {
    let commit = git(
        root,
        log,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", rev.unwrap_or("HEAD")),
        ],
    )?;
    let tree = git(root, log, &["rev-parse", &format!("{commit}^{{tree}}")])?;
    if rev.is_some() || git(root, log, &["status", "--porcelain"])?.is_empty() {
        return Ok((commit, tree, false));
    }
    let index = PathBuf::from(git(
        root,
        log,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    let private = std::env::temp_dir().join(format!("mac-proof-index-{}", std::process::id()));
    fs::copy(&index, &private).map_err(|error| format!("{}: {error}", index.display()))?;
    let written = (|| {
        let mut add = Command::new("git");
        add.arg("-C")
            .arg(root)
            .args(["add", "--all"])
            .env("GIT_INDEX_FILE", &private);
        step::succeed(
            log,
            &mut add,
            Instant::now() + Duration::from_secs(600),
            false,
        )
        .map_err(|_| "git add --all into a private index failed".to_owned())?;
        let mut write = Command::new("git");
        write
            .arg("-C")
            .arg(root)
            .arg("write-tree")
            .env("GIT_INDEX_FILE", &private);
        step::succeed(
            log,
            &mut write,
            Instant::now() + Duration::from_secs(600),
            true,
        )
        .map_err(|_| "git write-tree failed".to_owned())
    })();
    let _ = fs::remove_file(&private);
    Ok((
        commit,
        String::from_utf8_lossy(&written?).trim().to_owned(),
        true,
    ))
}

/// Pipes `from`'s stdout into `to` until both exit, one fails or the
/// deadline passes.
fn pipe(
    log: &mut Log,
    from: &mut Command,
    to: &mut Command,
    deadline: Instant,
) -> Result<(), String> {
    log.line(&format!("$ {from:?} | {to:?}"));
    let file = |log: &Log| log.stdio().map_err(|error| error.to_string());
    let mut source = from
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(file(log)?)
        .spawn()
        .map_err(|error| format!("cannot start {from:?}: {error}"))?;
    let stdout = source.stdout.take().expect("piped stdout");
    let sink = to
        .stdin(stdout)
        .stdout(file(log)?)
        .stderr(file(log)?)
        .spawn();
    // The command keeps the pipe's read end; closing it lets a sink that
    // fails early end the source with EPIPE instead of a full pipe.
    to.stdin(Stdio::null());
    let mut sink = match sink {
        Ok(sink) => sink,
        Err(error) => {
            step::stop(&mut source, false);
            return Err(format!("cannot start {to:?}: {error}"));
        }
    };
    let (mut sent, mut received): (Option<ExitStatus>, Option<ExitStatus>) = (None, None);
    loop {
        sent = sent.or_else(|| source.try_wait().ok().flatten());
        received = received.or_else(|| sink.try_wait().ok().flatten());
        match (sent, received) {
            (Some(a), Some(b)) if a.success() && b.success() => return Ok(()),
            (Some(a), _) | (_, Some(a)) if !a.success() => {
                step::stop(&mut source, false);
                step::stop(&mut sink, false);
                return Err(format!("transfer failed ({a}); see remote.log"));
            }
            _ if Instant::now() >= deadline => {
                step::stop(&mut source, false);
                step::stop(&mut sink, false);
                return Err("transfer timed out".to_owned());
            }
            _ => thread::sleep(POLL),
        }
    }
}

/// Runs the whole round trip and returns whether the Mac's run passed.
pub fn remote(options: Options) -> Result<bool, String> {
    if options.out.exists() {
        return Err(format!(
            "{} exists; give a fresh --out",
            options.out.display()
        ));
    }
    let root = PathBuf::from(
        String::from_utf8_lossy(
            &Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .map_err(|error| format!("git: {error}"))?
                .stdout,
        )
        .trim(),
    );
    if root.as_os_str().is_empty() {
        return Err("remote runs inside the Eon checkout".to_owned());
    }
    fs::create_dir_all(&options.out)
        .map_err(|error| format!("{}: {error}", options.out.display()))?;
    // Evidence inside the checkout would change the candidate tree.
    let out = fs::canonicalize(&options.out)
        .map_err(|error| format!("{}: {error}", options.out.display()))?;
    if out.starts_with(&root) {
        let _ = fs::remove_dir(&out);
        return Err("give an --out outside the checkout".to_owned());
    }
    let mut log = Log::new(
        File::create(out.join("remote.log")).map_err(|error| format!("remote.log: {error}"))?,
    );
    let (commit, tree, dirty) = identify(&root, &mut log, options.rev.as_deref())?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let run = format!("eon-proof/{}-{seconds}", &tree[..12]);
    let host = match &options.host {
        Host::Ssh(host) => host.clone(),
        Host::Local(home) => format!("local {}", home.display()),
    };
    for line in [
        format!("host: {host}"),
        format!("run directory: {run}"),
        format!("commit: {commit}"),
        format!("tree: {tree}"),
        format!("candidate with uncommitted changes: {dirty}"),
    ] {
        log.line(&line);
        progress(&line);
    }

    progress("sending the source");
    let mut archive = Command::new("git");
    archive
        .arg("-C")
        .arg(&root)
        .args(["archive", "--format=tar", &tree]);
    pipe(
        &mut log,
        &mut archive,
        &mut options.host.shell(&format!(
            "mkdir -p eon-proof && mkdir {run} {run}/src && tar -x -f - -C {run}/src"
        )),
        Instant::now() + TRANSFER,
    )?;

    let tool = match &options.host {
        // caffeinate keeps the Mac awake for the run.
        Host::Ssh(_) => format!(
            "if [ -e {NIX_PROFILE} ]; then . {NIX_PROFILE}; fi; exec /usr/bin/caffeinate -i nix --extra-experimental-features 'nix-command flakes' --accept-flake-config run path:.#mac-proof --"
        ),
        Host::Local(_) => {
            let exe = std::env::current_exe().map_err(|error| format!("current exe: {error}"))?;
            format!("exec {}", quote(&exe.to_string_lossy()))
        }
    };
    let dry_run = if options.dry_run { " --dry-run" } else { "" };
    progress(&format!(
        "running on the host for up to {}; follow {}",
        format_duration(options.timeout),
        out.join("remote.log").display()
    ));
    let ran = step::execute(
        &mut log,
        &mut options.host.shell(&format!(
            "cd {run}/src || exit 2; {tool} run --out ../evidence --expect-tree {tree} --timeout {}s{dry_run}",
            options.timeout.as_secs()
        )),
        Instant::now() + options.timeout + MARGIN,
        false,
        false,
    );
    // ssh exits 255 when the connection fails or drops.
    let finished = match ran {
        Ok((status, _)) => status.code().filter(|code| *code != 255),
        Err(_) => None,
    };

    progress("fetching the evidence");
    let fetch = format!("COPYFILE_DISABLE=1 tar -c -f - -C {run}/evidence .");
    let mut unpack = Command::new("tar");
    unpack.args(["-x", "-f", "-", "-C"]).arg(&out);
    // A failed fetch still prints the commands below: after a dropped
    // connection they are the way back to the evidence.
    let fetched = pipe(
        &mut log,
        &mut options.host.shell(&fetch),
        &mut unpack,
        Instant::now() + TRANSFER,
    );
    if let Ok(summary) = fs::read_to_string(out.join("summary.txt")) {
        progress(&summary);
    }
    let (fetch, cleanup) = match &options.host {
        Host::Ssh(host) => (
            format!(
                "ssh {host} {} | tar -x -f - -C {}",
                quote(&fetch),
                quote(&out.to_string_lossy())
            ),
            format!("ssh {host} rm -rf {run}"),
        ),
        Host::Local(home) => (
            format!(
                "(cd {} && {fetch}) | tar -x -f - -C {}",
                quote(&home.to_string_lossy()),
                quote(&out.to_string_lossy())
            ),
            format!("rm -rf {}", quote(&home.join(&run).to_string_lossy())),
        ),
    };
    match (finished, &fetched) {
        (Some(_), Ok(())) => progress(&format!(
            "evidence: {}\nclean up the host with: {cleanup}",
            out.display()
        )),
        (Some(_), Err(_)) => progress(&format!(
            "evidence so far: {}\nfetch it again with: {fetch}\nclean up the host with: {cleanup}",
            out.display()
        )),
        (None, _) => progress(&format!(
            "the connection or the time limit ended before the run did, which may still be going on the host\n\
             evidence so far: {}\nfetch it again later with: {fetch}\nonce it has finished, clean up the host with: {cleanup}",
            out.display()
        )),
    }
    // A run that could not start leaves no evidence to fetch; its exit names
    // the cause better than the failed fetch.
    let passed = outcome(finished)?;
    fetched?;
    Ok(passed)
}

/// The host run's verdict, including steps left for Yu: 0 for pass and 1 for
/// fail. Its 2, a failure of the tool itself, and no code at all are errors.
fn outcome(code: Option<i32>) -> Result<bool, String> {
    match code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => Err(format!(
            "the run on the host failed with exit {code}; see remote.log"
        )),
        None => Err("the run's outcome is unknown; see above".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::outcome;

    #[test]
    fn only_the_run_s_own_verdicts_are_results() {
        assert_eq!(outcome(Some(0)), Ok(true));
        assert_eq!(outcome(Some(1)), Ok(false));
        assert!(outcome(Some(2)).is_err());
        assert!(outcome(None).is_err());
    }
}
