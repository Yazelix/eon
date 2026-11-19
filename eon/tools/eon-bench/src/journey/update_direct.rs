//! The Nix-free direct update (eon-rjiyl.3). Archive A installs itself with
//! its own bundled `eon-install` from a signed fake release, configures a
//! throwaway signer, discovers B through `eon update --check`, refuses three
//! bad releases without touching the prefix, then installs B, and the next
//! launch runs B.
//!
//! The release network is `eon-fake-curl` (eon/tools/fake-curl): the jail
//! binds it at `/usr/bin/curl`, where the installer finds curl, and over each
//! selected payload's own curl, where the bundle launcher points `EON_CURL`.
//! The journey sandbox has no network, so nothing reaches GitHub. Every other
//! piece is the candidate's own: its launcher, `eon update`, its installer,
//! and the host's OpenSSH `ssh-keygen` and coreutils `timeout`, the
//! installer's declared host boundary (docs/INSTALLER.md).

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

use super::{
    ARCHIVE, BUNDLE_ROOT, CALL_LIMIT, HOST_SET, Input, NEXT, NEXT_SOURCE, Options, Receipt, Result,
    Value, describe, error, object, path_text, succeeded, survivors, text,
};
use crate::{
    bundle::{self, Bundle, Expect, HostSet},
    launcher::{self, Headless, WindowMode},
    sha256::Sha256,
};

/// The installer's host programs besides curl, which the fake replaces.
pub(super) const HOST_TOOLS: &[&str] = &["ssh-keygen", "timeout"];
/// B's extraction, removed once its identity is read.
pub(super) const NEXT_ROOT: &str = "next";
/// The managed prefix the case installs into.
pub(super) const PREFIX_ROOT: &str = "prefix";
/// The tampered copy of B's archive, removed after its refusal.
pub(super) const TAMPERED: &str = "net/tampered.tar.zst";
const NET_ROOT: &str = "net";
const TRUST_ROOT: &str = "trust";
/// Private keys live in the sandbox's own tmpfs and never in the evidence.
const KEYS: &str = "/tmp/eon-journey-keys";
const FAKE_CURL: &str = "/usr/bin/curl";
const REPOSITORY: &str = "https://github.com/Yazelix/eon";
const LISTING: &str = "https://api.github.com/repos/Yazelix/eon/releases?per_page=100&page=1";
const LOCK: &str = ".eon-update.lock";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// A zstd skippable frame: decoders skip it, so the archive's tree is
/// unchanged while its bytes are not.
const SKIPPABLE_FRAME: [u8; 12] = [0x50, 0x2a, 0x4d, 0x18, 4, 0, 0, 0, b'e', b'o', b'n', b'!'];
const EXTRACT_LIMIT: Duration = Duration::from_secs(120);
/// An install downloads, hashes, extracts and checks a whole payload.
const INSTALL_LIMIT: Duration = Duration::from_secs(180);
/// Release discovery has its own 30-second deadline inside `eon update`.
const CHECK_LIMIT: Duration = Duration::from_secs(45);

/// What the case knows about an input archive before installing it.
#[derive(Clone, Debug, PartialEq)]
struct Identity {
    version: String,
    build: String,
    revision: Option<String>,
    sha256: String,
    bytes: u64,
}

impl Identity {
    fn of(bundle: &Bundle) -> Result<Self> {
        let name = bundle
            .tree
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let version = name
            .strip_prefix("eon-")
            .and_then(|rest| rest.strip_suffix("-x86_64-linux"))
            .filter(|version| numbers(version).is_some())
            .ok_or(format!(
                "the archive's top-level directory {name} is not eon-X.Y.Z-x86_64-linux"
            ))?;
        let field = |key: &str| bundle.receipt.get(key).and_then(Value::as_str);
        Ok(Self {
            version: version.into(),
            build: field("build")
                .ok_or("the bundle's source receipt has no build")?
                .into(),
            revision: field("revision").map(str::to_owned),
            sha256: bundle.sha256.clone(),
            bytes: bundle.bytes,
        })
    }

    fn archive(&self) -> String {
        format!("eon-{}-x86_64-linux.tar.zst", self.version)
    }

    fn url(&self, name: &str) -> String {
        format!("{REPOSITORY}/releases/download/v{}/{name}", self.version)
    }

    fn archive_url(&self) -> String {
        self.url(&self.archive())
    }

    /// The installer-format checksum list: one line for the archive.
    fn sums(&self) -> String {
        format!("{}  {}\n", self.sha256, self.archive())
    }

    fn to_json(&self) -> Value {
        object([
            ("version", self.version.as_str().into()),
            ("build", self.build.as_str().into()),
            (
                "revision",
                self.revision.as_deref().map_or(Value::Null, Value::from),
            ),
            ("sha256", self.sha256.as_str().into()),
            ("bytes", self.bytes.into()),
        ])
    }
}

fn numbers(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// B must be a newer, different build of the same source as A. A source copy
/// has no git metadata, so B's revision may be absent.
fn successor(a: &Identity, b: &Identity) -> Result {
    if numbers(&b.version) <= numbers(&a.version) {
        return Err(format!(
            "--next {} is not newer than --bundle {}",
            b.version, a.version
        ));
    }
    if a.build == b.build || a.sha256 == b.sha256 {
        return Err("--next is the same build as --bundle".into());
    }
    if let (Some(a), Some(b)) = (&a.revision, &b.revision)
        && a != b
    {
        return Err(format!(
            "--bundle names revision {a} but --next names {b}: build both from one source"
        ));
    }
    Ok(())
}

/// The GitHub release listing `eon update` reads, newest first.
fn listing(releases: &[(&Identity, u64, u64)]) -> Value {
    Value::Array(
        releases
            .iter()
            .map(|(identity, sums, signature)| {
                let asset = |name: &str, size: u64| {
                    object([
                        ("name", name.into()),
                        ("browser_download_url", identity.url(name).into()),
                        ("state", "uploaded".into()),
                        ("size", size.into()),
                    ])
                };
                let mut archive = asset(&identity.archive(), identity.bytes);
                if let Value::Object(fields) = &mut archive {
                    fields.push((
                        "digest".into(),
                        format!("sha256:{}", identity.sha256).into(),
                    ));
                }
                object([
                    ("tag_name", format!("v{}", identity.version).into()),
                    ("draft", false.into()),
                    ("prerelease", false.into()),
                    ("published_at", "2026-11-19T00:00:00Z".into()),
                    (
                        "assets",
                        Value::Array(vec![
                            archive,
                            asset("SHA256SUMS", *sums),
                            asset("SHA256SUMS.sig", *signature),
                        ]),
                    ),
                ])
            })
            .collect(),
    )
}

/// One fake release: the files its three URLs serve.
struct Served<'a> {
    identity: &'a Identity,
    sums: PathBuf,
    signature: PathBuf,
    archive: PathBuf,
}

/// The fake network's routes: the listing, and each release's three assets.
fn routes(listing: &Path, releases: &[&Served<'_>]) -> String {
    let mut text = format!("{LISTING}\t{}\n", listing.display());
    for served in releases {
        let identity = served.identity;
        for (url, file) in [
            (identity.url("SHA256SUMS"), &served.sums),
            (identity.url("SHA256SUMS.sig"), &served.signature),
            (identity.archive_url(), &served.archive),
        ] {
            text.push_str(&format!("{url}\t{}\n", file.display()));
        }
    }
    text
}

/// Serves `releases` from now on, with a fresh request log.
fn serve(net: &Path, listing: &Path, releases: &[&Served<'_>]) -> Result {
    let staged = net.join("routes.new");
    fs::write(&staged, routes(listing, releases)).map_err(error)?;
    fs::rename(&staged, net.join("routes")).map_err(error)?;
    match fs::remove_file(net.join("requests.log")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// The fake network's log: each request's URL and status, in order.
fn requests(net: &Path) -> Result<Vec<(String, String)>> {
    let log = match fs::read_to_string(net.join("requests.log")) {
        Ok(log) => log,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    log.lines()
        .map(|line| {
            line.split_once('\t')
                .map(|(url, status)| (url.into(), status.into()))
                .ok_or(format!("malformed request log line {line:?}"))
        })
        .collect()
}

fn requests_json(requests: &[(String, String)]) -> Value {
    Value::Array(
        requests
            .iter()
            .map(|(url, status)| {
                object([
                    ("url", url.as_str().into()),
                    ("status", status.as_str().into()),
                ])
            })
            .collect(),
    )
}

/// Each URL in `delivered` answered 200 and no URL in `absent` was asked.
fn fetched(requests: &[(String, String)], delivered: &[String], absent: &[String]) -> Result {
    for url in delivered {
        if !requests
            .iter()
            .any(|(asked, status)| asked == url && status == "200")
        {
            return Err(format!("{url} was not delivered; requests: {requests:?}"));
        }
    }
    if let Some((url, _)) = requests.iter().find(|(asked, _)| absent.contains(asked)) {
        return Err(format!("{url} was requested; requests: {requests:?}"));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).map_err(|reason| format!("{}: {reason}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => return Ok(hasher.hex()),
            Ok(read) => hasher.update(&buffer[..read]),
            Err(reason) => return Err(format!("{}: {reason}", path.display())),
        }
    }
}

/// One prefix entry, read without following links.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Link(PathBuf),
    Dir(u32),
    /// Mode and the contents' SHA-256.
    File(u32, String),
    Other(u32),
}

/// Every entry of a prefix, the prefix itself included (as the empty path),
/// by its exact path.
type Snapshot = BTreeMap<PathBuf, Entry>;

fn snapshot(root: &Path) -> Result<Snapshot> {
    fn entry(path: &Path) -> Result<Entry> {
        let meta = fs::symlink_metadata(path).map_err(error)?;
        let mode = meta.mode() & 0o7777;
        Ok(if meta.file_type().is_symlink() {
            Entry::Link(fs::read_link(path).map_err(error)?)
        } else if meta.is_dir() {
            Entry::Dir(mode)
        } else if meta.is_file() {
            Entry::File(mode, sha256_file(path)?)
        } else {
            Entry::Other(mode)
        })
    }
    fn walk(root: &Path, dir: &Path, entries: &mut Snapshot) -> Result {
        for child in fs::read_dir(dir).map_err(error)? {
            let path = child.map_err(error)?.path();
            let found = entry(&path)?;
            if matches!(found, Entry::Dir(_)) {
                walk(root, &path, entries)?;
            }
            entries.insert(path.strip_prefix(root).map_err(error)?.into(), found);
        }
        Ok(())
    }
    let mut entries = Snapshot::from([(PathBuf::new(), entry(root)?)]);
    walk(root, root, &mut entries)?;
    Ok(entries)
}

/// `after` must equal `baseline`, except that `eon update --install` may have
/// created its empty 0600 lock (update/direct.rs) before verifying; from then
/// on the lock is part of the baseline.
fn unchanged(baseline: &mut Snapshot, after: Snapshot) -> Result<Value> {
    let lock = Path::new(LOCK);
    let created = !baseline.contains_key(lock)
        && after.get(lock) == Some(&Entry::File(0o600, EMPTY_SHA256.into()));
    let changed: Vec<_> = baseline
        .keys()
        .chain(after.keys())
        .filter(|key| baseline.get(*key) != after.get(*key))
        .filter(|key| !(created && key.as_path() == lock))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(20)
        .map(|key| format!("{key:?}: {:?} -> {:?}", baseline.get(key), after.get(key)))
        .collect();
    if !changed.is_empty() {
        return Err(format!(
            "the refused install changed the prefix: {changed:?}"
        ));
    }
    let entries = after.len() as u64;
    *baseline = after;
    Ok(object([
        ("entries", entries.into()),
        ("lock_created", created.into()),
    ]))
}

/// The one file under `tree/nix/store/*/bin/NAME` whose store entry's name
/// contains `component`.
fn store_program(tree: &Path, component: &str, name: &str) -> Result<PathBuf> {
    let found = store_programs(tree, name)?
        .into_iter()
        .filter(|path| {
            path.components()
                .any(|part| part.as_os_str().to_string_lossy().contains(component))
        })
        .collect::<Vec<_>>();
    match found.as_slice() {
        [program] => Ok(program.clone()),
        _ => Err(format!(
            "expected one {component} {name} under {}, found {found:?}",
            tree.display()
        )),
    }
}

/// Every regular file `tree/nix/store/*/bin/NAME`.
fn store_programs(tree: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let store = tree.join("nix/store");
    let mut found = Vec::new();
    for entry in fs::read_dir(&store).map_err(|reason| format!("{}: {reason}", store.display()))? {
        let program = entry.map_err(error)?.path().join("bin").join(name);
        if fs::symlink_metadata(&program).is_ok_and(|meta| meta.is_file()) {
            found.push(program);
        }
    }
    found.sort();
    Ok(found)
}

/// `eon versions`' first line names `identity`'s version and build.
fn runs(versions: &str, identity: &Identity) -> Result {
    let line = versions.lines().next().unwrap_or_default();
    let words: Vec<_> = line.split_whitespace().collect();
    if words.get(..3) != Some(&["eon", identity.version.as_str(), identity.build.as_str()][..]) {
        return Err(format!(
            "eon versions runs {line:?}, not eon {} {}",
            identity.version, identity.build
        ));
    }
    Ok(())
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

fn string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    field(value, path).and_then(Value::as_str)
}

/// The fixed expectations of one `eon update --json` report.
fn expect(report: &Value, checks: &[(&[&str], &str)]) -> Result {
    for (path, wanted) in checks {
        if string(report, path) != Some(wanted) {
            return Err(format!(
                "eon update reported {} = {:?}, expected {wanted:?}: {}",
                path.join("."),
                field(report, path),
                report.to_pretty().trim()
            ));
        }
    }
    Ok(())
}

/// A refused release: what it serves and whether the archive is fetched.
struct Refusal<'a> {
    name: &'static str,
    served: Served<'a>,
    archive_fetched: bool,
}

struct Case {
    launch: Headless,
    out: PathBuf,
    prefix: PathBuf,
    net: PathBuf,
    listing: PathBuf,
}

impl Case {
    fn output(&self, limit: Duration, program: &Path, args: &[&str]) -> Result<Output> {
        self.launch
            .candidate_output_until(Instant::now() + limit, program, args)
    }

    fn succeed(&self, limit: Duration, program: &Path, args: &[&str]) -> Result<Vec<u8>> {
        succeeded(program, args, self.output(limit, program, args)?)
    }

    fn eon(&self) -> PathBuf {
        self.prefix.join("bin/eon")
    }

    fn versions(&self, identity: &Identity) -> Result<Value> {
        let versions = String::from_utf8(self.succeed(CALL_LIMIT, &self.eon(), &["versions"])?)
            .map_err(error)?;
        runs(&versions, identity)?;
        Ok(object([("stdout", versions.trim().into())]))
    }

    /// `eon update ARGS --json`: its exit status and report.
    fn update(&self, limit: Duration, flags: &[&str]) -> Result<(Option<i32>, Value, Output)> {
        let mut args = vec!["update"];
        args.extend_from_slice(flags);
        args.push("--json");
        let output = self.output(limit, &self.eon(), &args)?;
        let report = std::str::from_utf8(&output.stdout)
            .map_err(error)
            .and_then(Value::parse)
            .map_err(|reason| format!("eon {args:?}: {reason}; {}", describe(&output)))?;
        Ok((output.status.code(), report, output))
    }

    /// Points every curl in the selected payload at the fake network.
    fn fake_payload_curls(&mut self, identity: &Identity) -> Result<Value> {
        let payload = self.prefix.join("versions").join(&identity.sha256);
        let curls = store_programs(&payload, "curl")?;
        if curls.is_empty() {
            return Err(format!("no curl under {}", payload.display()));
        }
        let fake = PathBuf::from(
            std::env::var_os("EON_BENCH_FAKE_CURL").ok_or("missing EON_BENCH_FAKE_CURL")?,
        );
        for curl in &curls {
            self.launch.jail_bind(&fake, curl)?;
        }
        Ok(Value::Array(curls.iter().map(|curl| text(curl)).collect()))
    }

    fn selected(&self) -> Result<String> {
        let current = fs::read_link(self.prefix.join("current")).map_err(error)?;
        current
            .strip_prefix("versions")
            .ok()
            .and_then(|hash| hash.to_str())
            .map(str::to_owned)
            .ok_or(format!("current names {}", current.display()))
    }
}

fn tool(key: &str) -> Result<std::ffi::OsString> {
    std::env::var_os(key).ok_or(format!("missing {key} (use the Nix bench package)"))
}

pub(super) fn run(options: &Options) -> Result {
    let Input::Bundle(archive) = &options.input else {
        return Err("update-direct needs --bundle".into());
    };
    let next = options.next.as_ref().ok_or("update-direct needs --next")?;
    let mut receipt = Receipt::new(options, archive, None, Instant::now());
    let out = options.out.clone();
    let mut launch = None;
    let result = steps(archive, next, &out, &mut receipt, &mut launch);
    let cleaned = match launch.as_mut() {
        Some(case) => {
            let cleaned = case.launch.processes.cleanup();
            if !cleaned {
                let _ = receipt.record("cleanup", "cleanup", || {
                    Err("private process cleanup failed".into())
                });
            }
            cleaned
        }
        None => true,
    };
    if let Err(reason) = &result
        && !receipt.failed
    {
        let _ = receipt.record("setup", "setup", || Err(reason.clone()));
    }
    // The extractions and the prefix are large and the archives reproduce
    // them; keep one only while a process may still run from it. The outer
    // run removes what is kept once the journey sandbox is gone.
    for (row, name) in [
        ("remove-bundle", BUNDLE_ROOT),
        ("remove-next", NEXT_ROOT),
        ("remove-prefix", PREFIX_ROOT),
        ("remove-tampered-archive", TAMPERED),
    ] {
        let root = out.join(name);
        if fs::symlink_metadata(&root).is_ok() {
            let _ = receipt.remove_root(row, &root, cleaned);
        }
    }
    receipt.finish(&out, result)
}

fn steps(
    archive: &Path,
    next: &Path,
    out: &Path,
    receipt: &mut Receipt<'_>,
    slot: &mut Option<Case>,
) -> Result {
    if receipt.bench.get("error").is_some() {
        return Err(format!("could not identify bench: {:?}", receipt.bench));
    }
    let host = std::env::var(HOST_SET).map_err(|_| "missing bundle host set")?;
    let host = HostSet::from_json(&Value::parse(&host)?)?;
    let fake = PathBuf::from(tool("EON_BENCH_FAKE_CURL")?);
    let (tar, zstd) = (tool("EON_BENCH_TAR")?, tool("EON_BENCH_ZSTD")?);
    let extract = |archive: &Path, root: &Path| {
        let opened = Bundle::extract(archive, root, (&tar, &zstd), Instant::now() + EXTRACT_LIMIT)?;
        let identity = Identity::of(&opened)?;
        Ok::<_, String>((opened, identity))
    };

    let (a_bundle, a) = receipt.produce("bundle", "setup", || {
        let (opened, identity) = extract(archive, &out.join(BUNDLE_ROOT))?;
        let evidence = object([
            ("bundle", opened.to_json()),
            ("identity", identity.to_json()),
        ]);
        Ok(((opened, identity), evidence))
    })?;
    receipt.eon = text(&a_bundle.entry);
    receipt.product_root = text(&a_bundle.product);
    receipt.source_receipt = a_bundle.receipt.clone();
    let b = receipt.produce("next", "setup", || {
        let root = out.join(NEXT_ROOT);
        let (opened, identity) = extract(next, &root)?;
        let evidence = object([
            ("bundle", opened.to_json()),
            ("identity", identity.to_json()),
            (
                "source",
                std::env::var(NEXT_SOURCE).map_or(Value::Null, Value::from),
            ),
            ("removed", bundle::remove(&root)?),
        ]);
        successor(&a, &identity)?;
        Ok((identity, evidence))
    })?;

    let net = out.join(NET_ROOT);
    let case = receipt.produce("host", "setup", || {
        let mut launch =
            Headless::bundle(out, &a_bundle.entry, WindowMode::Fullscreen, host.clone())?;
        launch.jail_bind(&fake, Path::new(FAKE_CURL))?;
        for archive in [ARCHIVE, NEXT] {
            launch.jail_bind(Path::new(archive), Path::new(archive))?;
        }
        launch
            .env
            .insert("EON_BENCH_NET".into(), net.display().to_string());
        let probe = launch.candidate_output_until(
            Instant::now() + CALL_LIMIT,
            Path::new("/bin/sh"),
            &[
                "-c",
                bundle::PROBE,
                "sh",
                bundle::LOADER,
                bundle::ICD,
                "",
                "",
            ],
        )?;
        let tools: Vec<&str> = HOST_TOOLS.iter().copied().chain(["curl"]).collect();
        let evidence = bundle::host_probe(
            &String::from_utf8_lossy(&probe.stdout),
            &a_bundle.glibc_floor,
            &Expect {
                tools: &tools,
                display: false,
            },
        )
        .map_err(|reason| format!("{reason}; probe: {}", describe(&probe)))?;
        let case = Case {
            launch,
            out: out.to_path_buf(),
            prefix: out.join(PREFIX_ROOT),
            listing: net.join("releases.json"),
            net: net.clone(),
        };
        Ok((
            case,
            object([("jail", evidence), ("host_set", host.to_json())]),
        ))
    })?;
    let case = slot.insert(case);
    let baseline_processes = launcher::namespace_processes()?;

    let trust = out.join(TRUST_ROOT).join("allowed_signers");
    let releases = receipt.produce("release", "setup", || release(case, &a, &b, &trust))?;
    let [a_served, b_served, foreign, tampered_sums, tampered_archive] = releases;

    let installer = store_program(&a_bundle.tree, "-eon-install-", "eon-install")?;
    let prefix = path_text(&case.prefix)?.to_owned();
    let signers = path_text(&trust)?.to_owned();
    receipt.record("install-a", "cli", || {
        serve(&case.net, &case.listing, &[&a_served])?;
        let url = a.archive_url();
        let stdout = case.succeed(
            INSTALL_LIMIT,
            &installer,
            &["install", &url, "--signers", &signers, "--prefix", &prefix],
        )?;
        let selected = case.selected()?;
        if selected != a.sha256 {
            return Err(format!("A's install selected {selected}, not {}", a.sha256));
        }
        Ok(object([
            ("installer", text(&installer)),
            ("stdout", String::from_utf8_lossy(&stdout).trim().into()),
            ("requests", requests_json(&requests(&case.net)?)),
        ]))
    })?;
    // A's installer has exited, so nothing runs from A's extraction.
    receipt.remove_root("remove-installer-bundle", &out.join(BUNDLE_ROOT), true)?;
    receipt.record("versions-a", "cli", || case.versions(&a))?;
    // Every later step but the real install must leave the prefix as A's
    // own installer left it.
    let mut baseline = receipt.produce("baseline", "check", || {
        let snapshot = snapshot(&case.prefix)?;
        let entries = snapshot.len() as u64;
        Ok((snapshot, object([("entries", entries.into())])))
    })?;
    receipt.record("configure", "setup", || {
        let config = case.out.join("config/config.toml");
        fs::write(
            &config,
            format!("[updates]\nbundle_prefix = \"{prefix}\"\nallowed_signers = \"{signers}\"\n"),
        )
        .map_err(error)?;
        Ok(object([
            ("config", text(&config)),
            ("faked_curls", case.fake_payload_curls(&a)?),
        ]))
    })?;
    receipt.record("check", "cli", || {
        serve(&case.net, &case.listing, &[&a_served, &b_served])?;
        let (status, report, output) = case.update(CHECK_LIMIT, &["--check"])?;
        let candidate = ["availability", "candidate"];
        let b_url = b.archive_url();
        expect(
            &report,
            &[
                (&["availability", "state"], "available"),
                (&[candidate[0], candidate[1], "version"], b.version.as_str()),
                (&[candidate[0], candidate[1], "archive_url"], b_url.as_str()),
                (&["installed", "version"], a.version.as_str()),
                (&["installed", "build"], a.build.as_str()),
            ],
        )?;
        if status != Some(0)
            || field(&report, &["capability", "install"]).and_then(Value::as_bool) != Some(true)
        {
            return Err(format!(
                "eon update --check offers no install (exit {status:?}): {}",
                describe(&output)
            ));
        }
        Ok(report)
    })?;

    let archive_url = b.archive_url();
    let checksums = [b.url("SHA256SUMS"), b.url("SHA256SUMS.sig")];
    for refusal in [
        Refusal {
            name: "refuse-foreign-signer",
            served: foreign,
            archive_fetched: false,
        },
        Refusal {
            name: "refuse-tampered-checksum",
            served: tampered_sums,
            archive_fetched: false,
        },
        Refusal {
            name: "refuse-tampered-archive",
            served: tampered_archive,
            archive_fetched: true,
        },
    ] {
        receipt.record(refusal.name, "refusal", || {
            serve(&case.net, &case.listing, &[&a_served, &refusal.served])?;
            let (status, report, output) = case.update(INSTALL_LIMIT, &["--install", "--yes"])?;
            expect(
                &report,
                &[
                    (&["error", "code"], "verification-failed"),
                    (&["selection", "state"], "unchanged"),
                ],
            )?;
            // EON-C41: verification-failed exits 1; anything else (124 from
            // the harness's timeout, say) is not this refusal.
            if status != Some(1) {
                return Err(format!("a refused install exited {status:?}, not 1"));
            }
            let requests = requests(&case.net)?;
            let (mut delivered, mut absent) = (checksums.to_vec(), Vec::new());
            if refusal.archive_fetched {
                delivered.push(archive_url.clone());
            } else {
                absent.push(archive_url.clone());
            }
            fetched(&requests, &delivered, &absent)?;
            let prefix = unchanged(&mut baseline, snapshot(&case.prefix)?)?;
            let versions = case.versions(&a)?;
            Ok(object([
                (
                    "exit",
                    status.map_or(Value::Null, |code| u64::from(code as u32).into()),
                ),
                ("report", report),
                (
                    "stderr",
                    String::from_utf8_lossy(&output.stderr).trim().into(),
                ),
                ("requests", requests_json(&requests)),
                ("prefix", prefix),
                ("versions", versions),
            ]))
        })?;
    }
    receipt.remove_root("remove-tampered-archive", &out.join(TAMPERED), true)?;

    receipt.record("install-b", "cli", || {
        let target = case.prefix.join("versions").join(&b.sha256);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(format!("{} exists before B's install", target.display()));
        }
        serve(&case.net, &case.listing, &[&a_served, &b_served])?;
        let (status, report, output) = case.update(INSTALL_LIMIT, &["--install", "--yes"])?;
        expect(
            &report,
            &[
                (&["outcome"], "installed"),
                (&["selection", "state"], "changed"),
                (&["selection", "selected", "version"], b.version.as_str()),
                (&["selection", "selected", "build"], b.build.as_str()),
            ],
        )?;
        let selected = case.selected()?;
        if status != Some(0) || selected != b.sha256 {
            return Err(format!(
                "install exited {status:?} with current at {selected}: {}",
                describe(&output)
            ));
        }
        if !case.prefix.join("versions").join(&a.sha256).is_dir() {
            return Err("installing B did not retain A's payload".into());
        }
        let requests = requests(&case.net)?;
        fetched(&requests, std::slice::from_ref(&archive_url), &[])?;
        Ok(object([
            ("report", report),
            ("requests", requests_json(&requests)),
        ]))
    })?;
    receipt.record("versions-b", "cli", || case.versions(&b))?;
    receipt.record("check-current", "cli", || {
        let faked = case.fake_payload_curls(&b)?;
        let (status, report, _) = case.update(CHECK_LIMIT, &["--check"])?;
        expect(&report, &[(&["availability", "state"], "current")])?;
        if status != Some(0) {
            return Err(format!("eon update --check exited {status:?}"));
        }
        Ok(object([("faked_curls", faked), ("report", report)]))
    })?;
    receipt.record("survivors", "cleanup", || {
        let survivors = survivors(&baseline_processes)?;
        if !survivors.is_empty() {
            return Err(format!(
                "live candidate processes: {}",
                survivors.join(", ")
            ));
        }
        Ok(object([("live_candidate_processes", 0u64.into())]))
    })?;
    Ok(())
}

/// Makes the fake releases: two throwaway keys, the signer record, A's and
/// B's signed checksums, the three refused variants of B and the listing.
/// The private keys are removed once everything is signed.
fn release<'a>(
    case: &Case,
    a: &'a Identity,
    b: &'a Identity,
    trust: &Path,
) -> Result<([Served<'a>; 5], Value)> {
    let keygen = Path::new("/usr/bin/ssh-keygen");
    let keys = Path::new(KEYS);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(keys)
        .map_err(error)?;
    let signed = (|| {
        for name in ["release", "foreign"] {
            let key = keys.join(name);
            case.succeed(
                CALL_LIMIT,
                keygen,
                &[
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-C",
                    "eon-journey",
                    "-f",
                    path_text(&key)?,
                ],
            )?;
        }
        let public = fs::read_to_string(keys.join("release.pub")).map_err(error)?;
        let key: Vec<_> = public.split_whitespace().take(2).collect();
        fs::create_dir_all(trust.parent().expect("trust has a directory")).map_err(error)?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(trust)
            .and_then(|mut file| {
                writeln!(
                    file,
                    "eon-release namespaces=\"eon-bundle\" {}",
                    key.join(" ")
                )
            })
            .map_err(error)?;
        let sign = |key: &str, dir: &str, sums: &str| -> Result<(PathBuf, PathBuf)> {
            let dir = case.net.join(dir);
            fs::create_dir_all(&dir).map_err(error)?;
            let file = dir.join("SHA256SUMS");
            fs::write(&file, sums).map_err(error)?;
            case.succeed(
                CALL_LIMIT,
                keygen,
                &[
                    "-Y",
                    "sign",
                    "-f",
                    path_text(&keys.join(key))?,
                    "-n",
                    "eon-bundle",
                    path_text(&file)?,
                ],
            )?;
            Ok((file.clone(), dir.join("SHA256SUMS.sig")))
        };
        Ok::<_, String>([
            sign("release", "a", &a.sums())?,
            sign("release", "b", &b.sums())?,
            sign("foreign", "foreign", &b.sums())?,
        ])
    })();
    let removed = bundle::remove(keys);
    let [(a_sums, a_sig), (b_sums, b_sig), (_, foreign_sig)] = signed?;
    removed?;

    // B's checksum edited after signing: its archive line names other bytes.
    let tampered_dir = case.net.join("tampered-checksum");
    fs::create_dir_all(&tampered_dir).map_err(error)?;
    let tampered_sums = tampered_dir.join("SHA256SUMS");
    let mut edited = b.sums();
    let flipped = if edited.starts_with('0') { "1" } else { "0" };
    edited.replace_range(..1, flipped);
    fs::write(&tampered_sums, edited).map_err(error)?;

    // B's archive with a skippable frame appended: the same tree, other bytes.
    let tampered_archive = case.out.join(TAMPERED);
    // Streamed rather than fs::copy, which would carry the store's read-only mode.
    fs::File::open(NEXT)
        .and_then(|mut input| {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tampered_archive)?;
            std::io::copy(&mut input, &mut output)?;
            output.write_all(&SKIPPABLE_FRAME)
        })
        .map_err(|reason| format!("{}: {reason}", tampered_archive.display()))?;
    let zstd = PathBuf::from(tool("EON_BENCH_ZSTD")?);
    let tested = case.launch.output_until(
        Instant::now() + EXTRACT_LIMIT,
        &zstd,
        &["-t", "-q", path_text(&tampered_archive)?],
    )?;
    if !tested.status.success() {
        return Err(format!(
            "the appended-frame archive does not decode: {}",
            describe(&tested)
        ));
    }
    let tampered_sha256 = sha256_file(&tampered_archive)?;

    let size = |path: &Path| fs::metadata(path).map(|meta| meta.len()).map_err(error);
    let listing = listing(&[
        (b, size(&b_sums)?, size(&b_sig)?),
        (a, size(&a_sums)?, size(&a_sig)?),
    ]);
    fs::write(&case.listing, listing.to_pretty()).map_err(error)?;
    let served = |identity, sums: &Path, signature: &Path, archive: &Path| Served {
        identity,
        sums: sums.into(),
        signature: signature.into(),
        archive: archive.into(),
    };
    let next = Path::new(NEXT);
    let evidence = object([
        ("allowed_signers", text(trust)),
        ("listing", text(&case.listing)),
        ("tampered_archive_sha256", tampered_sha256.into()),
        ("tampered_archive_decodes", true.into()),
    ]);
    Ok((
        [
            served(a, &a_sums, &a_sig, Path::new(ARCHIVE)),
            served(b, &b_sums, &b_sig, next),
            served(b, &b_sums, &foreign_sig, next),
            served(b, &tampered_sums, &b_sig, next),
            served(b, &b_sums, &b_sig, &tampered_archive),
        ],
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn identity(version: &str, build: &str, revision: Option<&str>) -> Identity {
        Identity {
            version: version.into(),
            build: build.into(),
            revision: revision.map(str::to_owned),
            sha256: format!("{build:0>64}"),
            bytes: 9,
        }
    }

    #[test]
    fn next_must_be_a_newer_build_of_the_same_source() {
        let a = identity("0.1.28", "g1-a", Some("r1"));
        assert!(successor(&a, &identity("0.1.29", "g1-b", Some("r1"))).is_ok());
        assert!(successor(&a, &identity("0.1.29", "g1-b", None)).is_ok());
        assert!(successor(&a, &identity("0.2.0", "g1-b", None)).is_ok());
        for bad in [
            identity("0.1.28", "g1-b", None),
            identity("0.1.27", "g1-b", None),
            identity("0.1.29", "g1-a", None),
            identity("0.1.29", "g1-b", Some("r2")),
        ] {
            assert!(successor(&a, &bad).is_err(), "{bad:?}");
        }
        assert!(numbers("0.1.10") > numbers("0.1.9"));
    }

    #[test]
    fn the_listing_names_each_release_and_its_official_assets() {
        let b = identity("0.1.29", "g1-b", None);
        let listing = listing(&[(&b, 80, 300)]);
        let release = &listing.as_array().unwrap()[0];
        assert_eq!(string(release, &["tag_name"]), Some("v0.1.29"));
        let assets = release.get("assets").and_then(Value::as_array).unwrap();
        let archive = &assets[0];
        assert_eq!(
            string(archive, &["browser_download_url"]),
            Some(
                "https://github.com/Yazelix/eon/releases/download/v0.1.29/eon-0.1.29-x86_64-linux.tar.zst"
            )
        );
        assert_eq!(
            string(archive, &["digest"]),
            Some(format!("sha256:{}", b.sha256).as_str())
        );
        assert_eq!(string(&assets[2], &["name"]), Some("SHA256SUMS.sig"));
        assert_eq!(
            b.sums(),
            format!("{}  eon-0.1.29-x86_64-linux.tar.zst\n", b.sha256)
        );
    }

    #[test]
    fn eon_versions_must_name_the_version_and_build() {
        let b = identity("0.1.29", "g1-b", None);
        assert!(runs("eon 0.1.29 g1-b\neonw 47\n", &b).is_ok());
        assert!(runs("eon 0.1.28 g1-b\n", &b).is_err());
        assert!(runs("eon 0.1.29 g1-a\n", &b).is_err());
        assert!(runs("", &b).is_err());
    }

    #[test]
    fn requests_must_deliver_the_named_urls_and_skip_the_rest() {
        let log = vec![
            ("u1".to_owned(), "200".to_owned()),
            ("u2".to_owned(), "404".to_owned()),
        ];
        assert!(fetched(&log, &["u1".into()], &["u3".into()]).is_ok());
        assert!(fetched(&log, &["u2".into()], &[]).is_err());
        assert!(fetched(&log, &[], &["u1".into()]).is_err());
    }

    #[test]
    fn a_refusal_may_only_create_the_empty_lock() {
        let dir = std::env::temp_dir().join(format!("bench-snapshot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("versions/aa")).unwrap();
        fs::write(dir.join("versions/aa/eon"), "payload").unwrap();
        std::os::unix::fs::symlink("versions/aa", dir.join("current")).unwrap();
        let mut baseline = snapshot(&dir).unwrap();
        let lock = || {
            fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(dir.join(LOCK))
                .unwrap()
        };
        lock();
        let created = unchanged(&mut baseline, snapshot(&dir).unwrap()).unwrap();
        assert_eq!(
            created.get("lock_created").and_then(Value::as_bool),
            Some(true)
        );
        assert!(unchanged(&mut baseline, snapshot(&dir).unwrap()).is_ok());
        // Same size, other contents: only a content hash sees it.
        fs::write(dir.join("versions/aa/eon"), "PAYLOAD").unwrap();
        assert!(unchanged(&mut baseline.clone(), snapshot(&dir).unwrap()).is_err());
        fs::write(dir.join("versions/aa/eon"), "payload").unwrap();
        writeln!(lock(), "held").unwrap();
        assert!(unchanged(&mut baseline.clone(), snapshot(&dir).unwrap()).is_err());
        fs::write(dir.join(LOCK), "").unwrap();
        fs::remove_file(dir.join("current")).unwrap();
        std::os::unix::fs::symlink("versions/bb", dir.join("current")).unwrap();
        assert!(unchanged(&mut baseline.clone(), snapshot(&dir).unwrap()).is_err());
        fs::remove_file(dir.join("current")).unwrap();
        std::os::unix::fs::symlink("versions/aa", dir.join("current")).unwrap();
        assert!(unchanged(&mut baseline.clone(), snapshot(&dir).unwrap()).is_ok());
        // The prefix's own mode counts.
        let mode = fs::metadata(&dir).unwrap().permissions().mode();
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode ^ 0o020)).unwrap();
        assert!(unchanged(&mut baseline.clone(), snapshot(&dir).unwrap()).is_err());
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
        // Two link targets that print alike are still different targets.
        use std::os::unix::ffi::OsStrExt;
        let lossy = |byte: u8| std::ffi::OsStr::from_bytes(&[b'x', byte]).to_owned();
        std::os::unix::fs::symlink(lossy(0xff), dir.join("odd")).unwrap();
        let mut odd = snapshot(&dir).unwrap();
        fs::remove_file(dir.join("odd")).unwrap();
        std::os::unix::fs::symlink(lossy(0xfe), dir.join("odd")).unwrap();
        assert!(unchanged(&mut odd, snapshot(&dir).unwrap()).is_err());
        fs::remove_file(dir.join("odd")).unwrap();
        fs::create_dir(dir.join(".eon-stage-1")).unwrap();
        assert!(unchanged(&mut baseline, snapshot(&dir).unwrap()).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
