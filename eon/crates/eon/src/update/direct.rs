//! Discovery and installer invocation for owned payloads. Metadata never authorizes bytes.
use super::*;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    prefix: PathBuf,
    root: PathBuf,
    helper: Option<PathBuf>,
    identity: Vec<(u64, u64)>,
    source: Source,
    build: Build,
}

#[derive(Debug, PartialEq)]
pub(super) struct Release {
    pub(super) version: String,
    pub(super) archive_url: String,
    sha256: String,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn version(value: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

fn archive_version(url: &str) -> Option<&str> {
    let path = url
        .strip_prefix(REPOSITORY)?
        .strip_prefix("/releases/download/v")?;
    let (number, file) = path.split_once('/')?;
    version(number)?;
    (file == format!("eon-{number}-x86_64-linux.tar.zst")).then_some(number)
}

fn private_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0
    })
}

fn text(path: &Path) -> Option<String> {
    String::from_utf8(bounded_file(path)?).ok()
}

fn unavailable(report: &mut Report, reason: &'static str, message: impl Into<String>) {
    report.availability = unknown(Some(reason));
    report.capability = Capability {
        install: false,
        reason: Some(reason),
        next_step: Some(message.into()),
    };
}

pub(super) fn inspect(
    prefix: Option<&Path>,
    invoked: Build,
    places: &Places,
    now: SystemTime,
) -> Option<Report> {
    let explicit = prefix.is_some();
    let prefix = prefix.map(Path::to_path_buf).or_else(|| {
        places
            .invoked
            .as_ref()?
            .ancestors()
            .find(|path| fs::symlink_metadata(path.join(".eon-install")).is_ok())
            .map(Path::to_path_buf)
    })?;
    let mut report = Report::checking(invoked.clone(), now);
    report.prefix = Some(prefix.clone());
    report.owner = Some(format!("managed prefix {}", prefix.display()));
    if explicit && !prefix.exists() {
        report.fail(
            "unknown-target",
            "resolve",
            format!("{} is not an existing installation", prefix.display()),
        );
        return Some(report);
    }
    unavailable(
        &mut report,
        "provenance-invalid",
        "This prefix lacks a valid installer-owned selector and receipt chain; inspect it manually.",
    );
    if !private_directory(&prefix)
        || !private_directory(&prefix.join("versions"))
        || !private_directory(&prefix.join("bin"))
        || text(&prefix.join(".eon-install")).as_deref() != Some("eon-install-v1\n")
        || fs::read_link(prefix.join("bin/eon")).ok().as_deref()
            != Some(Path::new("../current/eon"))
    {
        return Some(report);
    }
    let selected = fs::read_link(prefix.join("current")).ok();
    let Some(hash) = selected
        .as_deref()
        .and_then(|path| path.to_str())
        .and_then(|path| path.strip_prefix("versions/"))
        .filter(|hash| digest(hash))
    else {
        return Some(report);
    };
    let root = prefix.join("versions").join(hash);
    if !private_directory(&root) {
        return Some(report);
    }
    let Ok(root) = fs::canonicalize(root) else {
        return Some(report);
    };
    let Some(source) = text(&root.join(".eon-source")) else {
        return Some(report);
    };
    let lines: Vec<_> = source.lines().collect();
    let ["eon-install-v1", recorded_hash, url] = lines.as_slice() else {
        return Some(report);
    };
    if *recorded_hash != hash {
        return Some(report);
    }
    let Some(number) = archive_version(url) else {
        return Some(report);
    };
    let entry = root.join("eon");
    if !fs::symlink_metadata(&entry).is_ok_and(|meta| meta.is_symlink()) {
        return Some(report);
    }
    let executable = fs::canonicalize(&entry).ok();
    let Some(executable) = executable.filter(|path| {
        path.starts_with(&root)
            && fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.mode() & 0o111 != 0)
    }) else {
        return Some(report);
    };
    let Some(package) = executable
        .parent()
        .and_then(Path::parent)
        .filter(|path| path.starts_with(&root))
    else {
        return Some(report);
    };
    let Some(share) = fs::canonicalize(package.join("share"))
        .ok()
        .filter(|path| path.starts_with(&root))
    else {
        return Some(report);
    };
    let Some(receipt) = bounded_file(&share.join("eon-source-receipt.json"))
        .and_then(|bytes| parse_receipt(&bytes))
        .filter(|receipt| receipt.attribute.is_none())
    else {
        return Some(report);
    };
    let invoked_here = places.invoked.as_deref() == executable.parent();
    if !explicit && !invoked_here {
        unavailable(
            &mut report,
            "install-ambiguous",
            "The running eon is a retained payload, not this prefix's current selection; name --prefix explicitly.",
        );
        return Some(report);
    }
    if invoked_here
        && (invoked.build.as_deref() != Some(&receipt.build)
            || invoked.version.as_deref() != Some(number)
            || invoked.platform.as_deref() != Some(&receipt.platform))
    {
        return Some(report);
    }
    report.install_kind = "eon-install";
    report.launcher = Some(prefix.join("bin/eon"));
    report.proved = true;
    report.installed = Some(Build {
        version: Some(number.into()),
        build: Some(receipt.build),
        revision: receipt.revision.clone(),
        platform: Some(receipt.platform.clone()),
    });
    report.source = Some(Source {
        repository: Some(REPOSITORY.into()),
        policy: "release",
        original_ref: None,
        locked_ref: None,
        attribute: None,
        revision: receipt.revision,
        archive_url: Some((*url).into()),
        archive_sha256: Some(hash.into()),
    });
    if receipt.platform != "x86_64-linux" || !cfg!(all(target_os = "linux", target_arch = "x86_64"))
    {
        unavailable(
            &mut report,
            "unsupported-platform",
            "Direct release discovery supports only x86_64 Linux; use your platform's Nix installation route.",
        );
        return Some(report);
    }
    let helper = places
        .installer
        .as_ref()
        .and_then(|path| fs::canonicalize(path).ok())
        .filter(|path| {
            invoked_here
                && path.starts_with(&root)
                && fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.mode() & 0o111 != 0)
        });
    let Ok(prefix) = fs::canonicalize(&prefix) else {
        return Some(report);
    };
    let mut identities = vec![
        prefix.clone(),
        prefix.join("versions"),
        prefix.join("bin"),
        prefix.join(".eon-install"),
        root.clone(),
    ];
    if let Some(helper) = &helper {
        identities.push(helper.clone());
    }
    let Ok(identity) = identities
        .iter()
        .map(|path| fs::metadata(path).map(|meta| (meta.dev(), meta.ino())))
        .collect::<Result<_, _>>()
    else {
        return Some(report);
    };
    report.direct = Some(Snapshot {
        prefix,
        root,
        helper: helper.clone(),
        identity,
        source: report.source.clone().unwrap(),
        build: report.installed.clone().unwrap(),
    });
    let prefix = &report.direct.as_ref().unwrap().prefix;
    let signer_ready = places.signers.as_ref().is_some_and(|path| {
        let outside =
            |path: &Path| fs::canonicalize(path).is_ok_and(|path| !path.starts_with(prefix));
        if !path.ancestors().all(outside) {
            return false;
        }
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .is_ok_and(|file| {
                file.metadata()
                    .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
            })
    });
    if helper.is_some() && signer_ready {
        report.capability = Capability { install: true, reason: None, next_step: Some("Run eon update --install for this prefix; the installed helper verifies signed checksums with your independently trusted signer.".into()) };
    } else if helper.is_some() {
        unavailable(
            &mut report,
            "trust-unconfigured",
            "Provision an independently trusted allowed_signers file outside the managed prefix as docs/INSTALLER.md describes and set updates.allowed_signers to its absolute path. Eon never supplies or fetches a signer.",
        );
    } else {
        unavailable(
            &mut report,
            "installer-unavailable",
            "No bundled installer belongs to this running eon's selected payload; verify and install an official release manually with your independently trusted signer.",
        );
    }
    Some(report)
}

pub(super) fn install(
    request: &Request,
    mut report: Report,
    invoked: Build,
    places: &Places,
    system: &System,
) -> Report {
    report.operation = Some("install");
    let before = report.direct.clone();
    transact(request, &mut report, &invoked, places, system);
    let interrupted = INTERRUPTED.swap(false, Ordering::Relaxed);
    if let Some(prefix) = report.prefix.clone() {
        let after = inspect(Some(&prefix), invoked, places, SystemTime::now()).unwrap();
        let state = match (&before, &after.direct) {
            (Some(before), Some(after))
                if before.prefix == after.prefix
                    && before.identity.first() == after.identity.first() =>
            {
                if before.root == after.root
                    && before.source == after.source
                    && before.build == after.build
                {
                    "unchanged"
                } else {
                    "changed"
                }
            }
            _ => "unknown",
        };
        report.launcher = after.launcher;
        report.selection = Some(Selection {
            state,
            selected: after.installed,
            source: after.source,
        });
    }
    if report.selection.is_none() {
        report.selection = Some(Selection {
            state: "unknown",
            selected: None,
            source: None,
        });
    }
    if report.error.is_none() && report.outcome != "cancelled" {
        let selected = report.selection.as_ref().unwrap();
        if selected.state == "unknown" || selected.selected.is_none() {
            report.fail(
                "selection-failed",
                "inspect-result",
                "Cannot verify the selected payload after Install".into(),
            );
        } else if report.candidate_release.as_ref().is_none_or(|candidate| {
            selected.source.as_ref().is_none_or(|source| {
                source.archive_url.as_deref() != Some(&candidate.archive_url)
                    || source.archive_sha256.as_deref() != Some(&candidate.sha256)
            })
        }) {
            report.fail("selection-failed", "inspect-result", "The observed selection differs from the requested release; reporting its actual digest".into());
        } else {
            report.outcome = if selected.state == "changed" {
                "installed"
            } else {
                "unchanged"
            };
        }
    }
    if interrupted {
        let stage = report
            .error
            .as_ref()
            .map_or("fetch-build", |error| error.stage);
        report.fail(
            "interrupted",
            stage,
            "Install interrupted; reporting the actual selection, cancellation does not roll back"
                .into(),
        );
    }
    report
}

fn transact(
    request: &Request,
    report: &mut Report,
    invoked: &Build,
    places: &Places,
    system: &System,
) {
    if report.error.is_some() {
        return;
    }
    if request.prefix.is_none() && (request.json || request.yes || !std::io::stdin().is_terminal())
    {
        report.refuse(
            "install-ambiguous",
            "confirm",
            "--yes and noninteractive Install require an explicit or configured prefix".into(),
        );
        return;
    }
    if let Some(reason) = report.capability.reason {
        report.refuse(
            reason,
            "resolve",
            report
                .capability
                .next_step
                .clone()
                .unwrap_or_else(|| reason.into()),
        );
        return;
    }
    let before = report.direct.clone().unwrap();
    let writable = [
        &before.prefix,
        &before.prefix.join("bin"),
        &before.prefix.join("versions"),
    ]
    .iter()
    .all(|path| {
        CString::new(path.as_os_str().as_bytes())
            .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
    });
    if !writable {
        report.refuse(
            "target-unwritable",
            "resolve",
            "The managed prefix is not writable".into(),
        );
        return;
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(before.prefix.join(".eon-update.lock"));
    let _lock = match lock {
        Ok(lock) => {
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    report.refuse(
                        "operation-busy",
                        "resolve",
                        "Another Eon update owns this prefix".into(),
                    );
                } else {
                    report.fail(
                        "selection-failed",
                        "resolve",
                        format!("Cannot lock prefix: {error}"),
                    );
                }
                return;
            }
            lock
        }
        Err(error) => {
            report.fail(
                "selection-failed",
                "resolve",
                format!("Cannot open prefix update lock: {error}"),
            );
            return;
        }
    };
    // Metadata discovers the pinned URL; only the installed helper verifies its bytes.
    refresh(report, system);
    if report.error.is_some() {
        return;
    }
    if let Some(reason) = report.capability.reason {
        report.refuse(
            reason,
            "check",
            report
                .capability
                .next_step
                .clone()
                .unwrap_or_else(|| reason.into()),
        );
        return;
    }
    let candidate = report.candidate_release.as_ref().unwrap();
    eprintln!(
        "eon update: install under {} from {} using signer {}",
        before.prefix.display(),
        candidate.archive_url,
        places.signers.as_ref().unwrap().display()
    );
    let already_current = report.availability.state == "current";
    let needs_consent = !already_current || request.json || !std::io::stdin().is_terminal();
    if needs_consent && !consent(request, report) {
        return;
    }
    if inspect(
        report.prefix.as_deref(),
        invoked.clone(),
        places,
        SystemTime::now(),
    )
    .and_then(|after| after.direct)
    .as_ref()
        != Some(&before)
    {
        report.refuse(
            "target-changed",
            "select",
            "Prefix identity, selection, receipts or installer changed since consent was displayed"
                .into(),
        );
        return;
    }
    if INTERRUPTED.load(Ordering::Relaxed) {
        return;
    }
    if already_current {
        return;
    }
    eprintln!(
        "eon update: downloading, verifying and selecting with the installed helper; Ctrl+C cancels"
    );
    let mut command = Command::new(before.helper.as_ref().unwrap());
    let child = owned_child(&mut command)
        .env("EON_INSTALLER_FOREGROUND_CHECK", "1")
        .arg("install")
        .arg(&report.candidate_release.as_ref().unwrap().archive_url)
        .arg("--signers")
        .arg(places.signers.as_ref().unwrap())
        .arg("--prefix")
        .arg(&before.prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .stderr(Stdio::inherit())
        .spawn();
    let result = child
        .map_err(Bounded::Spawn)
        .and_then(|mut child| wait_child(&mut child, None, true));
    match result {
        Ok(status) if status.success() => (),
        Ok(status) => match status.code() {
            Some(10) => report.fail(
                "verification-failed",
                "verify",
                "Installer signature, archive or URL/version verification failed; see diagnostics"
                    .into(),
            ),
            Some(11) => report.fail(
                "preflight-failed",
                "verify",
                "Packaged eon preflight failed; see diagnostics".into(),
            ),
            Some(12) => report.refuse(
                "operation-busy",
                "select",
                "The installer prefix is busy; retry after its owner exits".into(),
            ),
            Some(13) => report.fail(
                "remote-unavailable",
                "fetch-build",
                "Installer download failed; see diagnostics".into(),
            ),
            Some(14) => report.refuse(
                "trust-unconfigured",
                "verify",
                "Provision an independently trusted allowed_signers file as docs/INSTALLER.md describes; see installer diagnostics".into(),
            ),
            _ => report.fail(
                "selection-failed",
                "select",
                format!("Installer exited {status}; see diagnostics"),
            ),
        },
        Err(error) => {
            let (code, message) = error.trouble(["selection-failed"; 3], "eon-install");
            report.fail(code, "fetch-build", message);
        }
    }
}

fn metadata(message: &str) -> Trouble {
    ("metadata-invalid", message.into())
}

fn releases(host: &dyn Host, deadline: Instant) -> Result<Option<Release>, Trouble> {
    let mut latest: Option<([u64; 3], Value)> = None;
    let mut seen = std::collections::HashSet::new();
    // Fixed network safety budget; reaching it cannot establish currentness.
    for page in 1..=10 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|limit| !limit.is_zero())
            .ok_or((
                "offline",
                "Release discovery exceeded its total 30-second deadline".into(),
            ))?;
        let body = response(host.get(
            &format!("{API}/releases?per_page=100&page={page}"),
            "application/vnd.github+json",
            remaining,
        )?)?
        .ok_or_else(|| metadata("GitHub returned no release listing"))?;
        if Instant::now() >= deadline {
            return Err((
                "offline",
                "Release discovery exceeded its total 30-second deadline".into(),
            ));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| metadata("GitHub returned malformed release JSON"))?;
        let entries = value
            .as_array()
            .ok_or_else(|| metadata("GitHub returned no release list"))?;
        if entries.len() > 100 {
            return Err(metadata("GitHub exceeded the release page size"));
        }
        for entry in entries {
            let draft = entry["draft"]
                .as_bool()
                .ok_or_else(|| metadata("Release lacks draft status"))?;
            let prerelease = entry["prerelease"]
                .as_bool()
                .ok_or_else(|| metadata("Release lacks prerelease status"))?;
            if draft || prerelease {
                continue;
            }
            let tag = entry["tag_name"]
                .as_str()
                .ok_or_else(|| metadata("Release lacks a tag"))?;
            let Some(number) = tag.strip_prefix('v').and_then(version) else {
                continue;
            };
            if entry["published_at"].as_str().is_none_or(str::is_empty) {
                return Err(metadata("Release lacks publication status"));
            }
            if !seen.insert(number) {
                return Err(metadata("Duplicate numbered release"));
            }
            match &latest {
                Some((previous, _)) if previous > &number => (),
                _ => latest = Some((number, entry.clone())),
            }
        }
        if entries.len() < 100 {
            break;
        }
        if page == 10 {
            return Err(metadata("Release discovery exceeded its page budget"));
        }
    }
    let Some((_, entry)) = latest else {
        return Ok(None);
    };
    let number = entry["tag_name"]
        .as_str()
        .unwrap()
        .strip_prefix('v')
        .unwrap();
    let assets = entry["assets"]
        .as_array()
        .ok_or_else(|| metadata("Release lacks assets"))?;
    let archive = format!("eon-{number}-x86_64-linux.tar.zst");
    let mut hash = None;
    for name in [&archive, "SHA256SUMS", "SHA256SUMS.sig"] {
        let matches: Vec<_> = assets
            .iter()
            .filter(|asset| asset["name"].as_str() == Some(name))
            .collect();
        if matches.is_empty() {
            return Ok(None);
        }
        if matches.len() != 1 {
            return Err(metadata("Release has duplicate assets"));
        }
        let asset = matches[0];
        let url = format!("{REPOSITORY}/releases/download/v{number}/{name}");
        if asset["browser_download_url"].as_str() != Some(&url) {
            return Err(metadata(
                "Release asset URL does not match its official numbered release",
            ));
        }
        if asset["state"] != "uploaded" || asset["size"].as_u64().is_none_or(|size| size == 0) {
            return Ok(None);
        }
        if name == archive {
            hash = Some(
                asset["digest"]
                    .as_str()
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .filter(|value| digest(value))
                    .ok_or_else(|| metadata("Release archive lacks SHA-256 metadata"))?
                    .to_owned(),
            );
        }
    }
    if Instant::now() >= deadline {
        return Err((
            "offline",
            "Release discovery exceeded its total 30-second deadline".into(),
        ));
    }
    Ok(Some(Release {
        version: number.into(),
        archive_url: format!("{REPOSITORY}/releases/download/v{number}/{archive}"),
        sha256: hash.unwrap(),
    }))
}

pub(super) fn check_remote(mut report: Report, host: &dyn Host) -> Report {
    refresh(&mut report, host);
    report
}

fn refresh(report: &mut Report, host: &dyn Host) {
    if !report.proved || report.capability.reason == Some("unsupported-platform") {
        return;
    }
    match releases(host, Instant::now() + Duration::from_secs(30)) {
        Err((reason, message)) => {
            report.availability = unknown(Some(reason));
            report.fail("remote-unavailable", "check", message);
        }
        Ok(None) => unavailable(
            report,
            "release-unavailable",
            "No published numbered Linux release with its archive, signed-checksum companions and digest metadata is available; check again later.",
        ),
        Ok(Some(candidate)) => {
            let installed = report
                .installed
                .as_ref()
                .unwrap()
                .version
                .as_deref()
                .unwrap();
            let source = report.source.as_ref().unwrap();
            let ordering = version(&candidate.version)
                .unwrap()
                .cmp(&version(installed).unwrap());
            match ordering {
                std::cmp::Ordering::Greater => {
                    report.availability = Availability {
                        state: "available",
                        candidate: None,
                        reason: None,
                    }
                }
                std::cmp::Ordering::Less => unavailable(
                    report,
                    "no-newer-release",
                    "The official release is older than this installation; no downgrade is offered.",
                ),
                std::cmp::Ordering::Equal
                    if source.archive_url.as_deref() == Some(&candidate.archive_url)
                        && source.archive_sha256.as_deref() == Some(&candidate.sha256) =>
                {
                    report.outcome = "unchanged";
                    report.availability = Availability {
                        state: "current",
                        candidate: None,
                        reason: None,
                    };
                }
                _ => unavailable(
                    report,
                    "source-unverified",
                    "The same release version names different archive bytes; verify the source manually.",
                ),
            }
            report.candidate_release = Some(candidate);
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::tests::{BUILD, Fake, Tree, at, invoked};
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn human_install_reports_the_actual_archive_after_interruption() {
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        let mut report = inspect(Some(&prefix), invoked(), &places, at(0)).unwrap();
        report.operation = Some("install");
        report.fail(
            "interrupted",
            "select",
            "interrupted after selection".into(),
        );
        let mut source = report.source.clone().unwrap();
        source.archive_url = Some(format!(
            "{REPOSITORY}/releases/download/v0.1.22/eon-0.1.22-x86_64-linux.tar.zst"
        ));
        source.archive_sha256 = Some("b".repeat(64));
        let mut selected = report.installed.clone().unwrap();
        selected.version = Some("0.1.22".into());
        selected.build = Some("g1-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
        report.selection = Some(Selection {
            state: "changed",
            selected: Some(selected),
            source: Some(source.clone()),
        });
        let human = report.human();
        assert!(
            human.contains(&format!("Selected source: {}", source.archive_url.unwrap())),
            "{human}"
        );
        assert!(
            human.contains(&format!("Selected archive SHA-256: {}", "b".repeat(64))),
            "{human}"
        );
    }

    fn payload(tree: &Tree) -> (PathBuf, Places) {
        let prefix = tree.0.join("install with ' spaces");
        let root = prefix.join("versions").join(HASH);
        let package = root.join("nix/store/product");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::create_dir_all(package.join("share")).unwrap();
        fs::create_dir(prefix.join("bin")).unwrap();
        for dir in [
            &prefix,
            &prefix.join("versions"),
            &prefix.join("bin"),
            &root,
        ] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(prefix.join(".eon-install"), "eon-install-v1\n").unwrap();
        fs::write(package.join("bin/eon"), "").unwrap();
        fs::set_permissions(package.join("bin/eon"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            package.join(RECEIPT),
            json!({"schema_version":1,"repository":REPOSITORY,
            "revision":"1".repeat(40),"build":BUILD,"platform":"x86_64-linux","attribute":null})
            .to_string(),
        )
        .unwrap();
        symlink("nix/store/product/bin/eon", root.join("eon")).unwrap();
        symlink(format!("versions/{HASH}"), prefix.join("current")).unwrap();
        symlink("../current/eon", prefix.join("bin/eon")).unwrap();
        fs::write(root.join(".eon-source"), format!("eon-install-v1\n{HASH}\n{REPOSITORY}/releases/download/v0.1.21/eon-0.1.21-x86_64-linux.tar.zst\n")).unwrap();
        let places = tree.places(&prefix.join("bin/eon"));
        (prefix, places)
    }

    fn release(version: &str, hash: &str) -> Value {
        let mut assets: Vec<_> = [
            format!("eon-{version}-x86_64-linux.tar.zst"),
            "SHA256SUMS".into(),
            "SHA256SUMS.sig".into(),
        ]
        .iter()
        .map(|name| {
            json!({"name":name,"state":"uploaded","size":123,
                "browser_download_url":format!("{REPOSITORY}/releases/download/v{version}/{name}")})
        })
        .collect();
        assets[0]["digest"] = format!("sha256:{hash}").into();
        json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":false,
            "published_at":"2026-11-08T12:00:00Z","assets":assets})
    }

    fn remote_report(prefix: &Path, places: &Places, body: Value) -> Report {
        let fake = Fake {
            responses: vec![("releases?", Ok((200, body.to_string().into_bytes())))],
            ..Fake::default()
        };
        check_remote(
            inspect(Some(prefix), invoked(), places, UNIX_EPOCH).unwrap(),
            &fake,
        )
    }

    #[test]
    fn validates_one_owned_chain_and_keeps_trust_separate_from_discovery() {
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        let before = fs::read_link(prefix.join("current")).unwrap();
        let report = remote_report(&prefix, &places, json!([release("0.1.22", HASH)]));
        assert_eq!(report.status(), 0);
        assert_eq!(report.json()["target"], json!({"prefix":prefix}));
        assert_eq!(report.json()["source"]["archive_sha256"], HASH);
        assert_eq!(
            report.json()["availability"]["candidate"]["version"],
            "0.1.22"
        );
        assert_eq!(report.availability.state, "available");
        assert_eq!(report.capability.reason, Some("installer-unavailable"));
        assert_eq!(before, fs::read_link(prefix.join("current")).unwrap());
        assert!(inspect(None, invoked(), &places, at(0)).is_some());
        let report = remote_report(&prefix, &places, json!([release("0.1.21", HASH)]));
        assert_eq!(
            (report.outcome, report.availability.state),
            ("unchanged", "current")
        );
        let report = remote_report(
            &prefix,
            &places,
            json!([release("0.1.21", &"b".repeat(64))]),
        );
        assert_eq!(report.availability.reason, Some("source-unverified"));
        let report = remote_report(&prefix, &places, json!([release("0.1.20", HASH)]));
        assert_eq!(report.availability.reason, Some("no-newer-release"));
    }

    #[test]
    fn inherited_installers_and_retained_payloads_do_not_own_the_selection() {
        let tree = Tree::new();
        let (prefix, mut places) = payload(&tree);
        let helper = prefix.join("current/nix/store/product/bin/eon-install");
        fs::write(&helper, "").unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        places.installer = Some(helper);
        assert_eq!(
            inspect(None, invoked(), &places, at(0))
                .unwrap()
                .capability
                .reason,
            Some("trust-unconfigured")
        );
        let foreign = tree.0.join("foreign/eon");
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&foreign, "").unwrap();
        places.invoked = Some(foreign.parent().unwrap().to_path_buf());
        assert!(inspect(None, invoked(), &places, at(0)).is_none());
        assert_eq!(
            inspect(Some(&prefix), invoked(), &places, at(0))
                .unwrap()
                .capability
                .reason,
            Some("installer-unavailable")
        );
        places.invoked = Some(prefix.join("versions/retained/bin"));
        let report = inspect(None, invoked(), &places, at(0)).unwrap();
        assert_eq!(report.capability.reason, Some("install-ambiguous"));
        assert!(report.source.is_none());
    }

    #[test]
    fn signers_must_be_nonempty_files_outside_the_managed_prefix() {
        let tree = Tree::new();
        let (prefix, mut places) = payload(&tree);
        let helper = prefix.join("current/nix/store/product/bin/eon-install");
        fs::write(&helper, "").unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        places.installer = Some(helper);
        let empty = tree.0.join("empty-signers");
        fs::write(&empty, "").unwrap();
        let inside = prefix.join("signers");
        fs::write(&inside, "fixture independently provisioned signer").unwrap();
        let outside = tree.0.join("signers");
        fs::write(&outside, "fixture independently provisioned signer").unwrap();
        let outside_link = tree.0.join("link-into-prefix");
        symlink(&inside, &outside_link).unwrap();
        let inside_link = prefix.join("link-outside");
        symlink(&outside, &inside_link).unwrap();
        let through_prefix = prefix.join("directory-outside");
        symlink(&tree.0, &through_prefix).unwrap();
        for path in [
            tree.0.join("missing-signers"),
            empty,
            inside,
            outside_link,
            inside_link,
            through_prefix.join("signers"),
        ] {
            places.signers = Some(path.clone());
            let report = inspect(Some(&prefix), invoked(), &places, at(0)).unwrap();
            assert_eq!(
                report.capability.reason,
                Some("trust-unconfigured"),
                "{}",
                path.display()
            );
            assert!(!report.capability.install);
        }
        places.signers = Some(outside);
        assert!(
            inspect(Some(&prefix), invoked(), &places, at(0))
                .unwrap()
                .capability
                .install
        );
    }

    #[test]
    fn pagination_must_finish_and_versions_have_numeric_order() {
        assert!(version("0.01.1").is_none());
        assert!(version("0.1.2-rc.1").is_none());
        assert!(version("0.1.18446744073709551616").is_none());
        assert!(version("0.1.9") < version("0.1.22"));
        let mut draft = release("1.0.0", HASH);
        draft["draft"] = true.into();
        let page = serde_json::to_vec(&vec![draft; 100]).unwrap();
        let fake = Fake {
            responses: vec![
                ("releases?per_page=100&page=1", Ok((200, page.clone()))),
                (
                    "releases?per_page=100&page=2",
                    Ok((
                        200,
                        serde_json::to_vec(&json!([release("0.1.22", HASH)])).unwrap(),
                    )),
                ),
            ],
            ..Fake::default()
        };
        assert_eq!(
            releases(&fake, Instant::now() + Duration::from_secs(30))
                .unwrap()
                .unwrap()
                .version,
            "0.1.22"
        );
        assert_eq!(fake.requests.borrow().len(), 2);
        assert!(fake.limits.borrow()[1] < fake.limits.borrow()[0]);
        let mut first_page: Value = serde_json::from_slice(&page).unwrap();
        first_page[99] = release("0.1.22", HASH);
        let broken = Fake {
            responses: vec![
                (
                    "releases?per_page=100&page=1",
                    Ok((200, serde_json::to_vec(&first_page).unwrap())),
                ),
                ("releases?per_page=100&page=2", Ok((404, vec![]))),
            ],
            ..Fake::default()
        };
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        let report = check_remote(
            inspect(Some(&prefix), invoked(), &places, at(0)).unwrap(),
            &broken,
        );
        assert_eq!(report.status(), 1);
        assert_eq!(report.availability.state, "unknown");
        assert_eq!(report.error.unwrap().code, "remote-unavailable");
        let expired = Fake::default();
        assert_eq!(releases(&expired, Instant::now()).unwrap_err().0, "offline");
        assert!(expired.requests.borrow().is_empty());
        let fake = Fake {
            responses: vec![("releases?", Ok((200, page)))],
            ..Fake::default()
        };
        assert_eq!(
            releases(&fake, Instant::now() + Duration::from_secs(30))
                .unwrap_err()
                .0,
            "metadata-invalid"
        );
        assert_eq!(fake.requests.borrow().len(), 10);
    }

    #[test]
    fn receipt_reads_refuse_links_and_fifos_without_waiting() {
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        let file = prefix.join("current/nix/store/product").join(RECEIPT);
        let foreign = tree.0.join("foreign-receipt");
        fs::rename(&file, &foreign).unwrap();
        symlink(&foreign, &file).unwrap();
        assert_eq!(
            inspect(Some(&prefix), invoked(), &places, at(0))
                .unwrap()
                .capability
                .reason,
            Some("provenance-invalid")
        );
        fs::remove_file(&file).unwrap();
        let name = CString::new(file.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || sender.send(bounded_file(&file)).unwrap());
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("receipt read blocked on FIFO"),
            None
        );
    }

    #[test]
    fn every_required_asset_and_publication_state_is_needed() {
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        for missing in 0..3 {
            let mut value = release("0.1.22", HASH);
            value["assets"].as_array_mut().unwrap().remove(missing);
            let report = remote_report(&prefix, &places, json!([value]));
            assert_eq!(report.status(), 0);
            assert_eq!(report.availability.state, "unknown");
            assert_eq!(report.availability.reason, Some("release-unavailable"));
            assert!(report.candidate_release.is_none());
        }
        let mut value = release("0.1.22", HASH);
        value["assets"][0]["state"] = "new".into();
        let report = remote_report(&prefix, &places, json!([value]));
        assert_eq!(report.availability.state, "unknown");
        assert_eq!(report.availability.reason, Some("release-unavailable"));
        assert!(report.candidate_release.is_none());
        let mut value = release("0.1.22", HASH);
        value.as_object_mut().unwrap().remove("published_at");
        let report = remote_report(&prefix, &places, json!([value]));
        assert_eq!(report.status(), 1);
        assert_eq!(report.availability.reason, Some("metadata-invalid"));
        assert_eq!(report.error.unwrap().code, "remote-unavailable");
        let report = remote_report(
            &prefix,
            &places,
            json!([
                release("0.1.22", HASH),
                release("0.1.20", HASH),
                release("0.1.20", HASH)
            ]),
        );
        assert_eq!(report.status(), 1);
        assert_eq!(report.availability.reason, Some("metadata-invalid"));
    }

    #[test]
    fn altered_prefix_and_receipts_cannot_become_current() {
        for fault in [
            "marker", "bin", "current", "private", "receipt", "source", "build", "platform",
        ] {
            let tree = Tree::new();
            let (prefix, places) = payload(&tree);
            let root = prefix.join("versions").join(HASH);
            match fault {
                "marker" => fs::write(prefix.join(".eon-install"), "foreign\n").unwrap(),
                "bin" => {
                    fs::remove_file(prefix.join("bin/eon")).unwrap();
                    symlink(root.join("eon"), prefix.join("bin/eon")).unwrap();
                }
                "current" => {
                    fs::remove_file(prefix.join("current")).unwrap();
                    symlink(&root, prefix.join("current")).unwrap();
                }
                "private" => {
                    fs::set_permissions(prefix.join("versions"), fs::Permissions::from_mode(0o755))
                        .unwrap()
                }
                "source" => fs::write(
                    root.join(".eon-source"),
                    format!("eon-install-v1\n{HASH}\nhttps://evil.test/eon.tar.zst\n"),
                )
                .unwrap(),
                _ => {
                    let file = root.join("nix/store/product").join(RECEIPT);
                    let mut value: Value =
                        serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
                    match fault {
                        "build" => value["build"] = "g1-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                        "platform" => value["platform"] = "aarch64-darwin".into(),
                        _ => value["schema_version"] = 2.into(),
                    }
                    fs::write(file, value.to_string()).unwrap();
                }
            }
            let report = remote_report(&prefix, &places, json!([release("0.1.21", HASH)]));
            assert_ne!(report.availability.state, "current", "{fault}");
            assert!(!report.capability.install, "{fault}");
        }
    }

    #[test]
    fn stable_numbered_assets_and_remote_failures_have_honest_results() {
        let tree = Tree::new();
        let (prefix, places) = payload(&tree);
        let mut draft = release("1.0.0", HASH);
        draft["draft"] = true.into();
        let mut pre = release("2.0.0", HASH);
        pre["prerelease"] = true.into();
        let report = remote_report(
            &prefix,
            &places,
            json!([draft, release("0.1.9", HASH), pre, release("0.1.22", HASH)]),
        );
        assert_eq!(report.candidate_release.unwrap().version, "0.1.22");
        let report = remote_report(&prefix, &places, json!([]));
        assert_eq!(report.availability.reason, Some("release-unavailable"));
        for fault in ["foreign", "duplicate", "missing", "digest", "metadata"] {
            let mut value = release("0.1.22", HASH);
            match fault {
                "foreign" => {
                    value["assets"][0]["browser_download_url"] =
                        "https://evil.test/eon.tar.zst".into()
                }
                "duplicate" => {
                    let asset = value["assets"][0].clone();
                    value["assets"].as_array_mut().unwrap().push(asset);
                }
                "missing" => value["assets"] = json!([]),
                "digest" => value["assets"][0]["digest"] = "sha256:invalid".into(),
                _ => value["draft"] = Value::Null,
            }
            let report = remote_report(&prefix, &places, json!([value]));
            assert_ne!(report.availability.state, "current", "{fault}");
            assert!(!report.capability.install, "{fault}");
            if fault != "missing" {
                assert_eq!(report.status(), 1, "{fault}");
            } else {
                assert_eq!(report.availability.state, "unknown");
                assert_eq!(report.availability.reason, Some("release-unavailable"));
                assert!(report.candidate_release.is_none());
            }
        }
        for response in [
            Ok((404, vec![])),
            Ok((422, vec![])),
            Ok((429, vec![])),
            Ok((503, vec![])),
            Ok((200, b"broken".to_vec())),
            Err(("offline", "offline".into())),
        ] {
            let fake = Fake {
                responses: vec![("releases?", response)],
                ..Fake::default()
            };
            let report = check_remote(
                inspect(Some(&prefix), invoked(), &places, at(0)).unwrap(),
                &fake,
            );
            assert_eq!(report.status(), 1);
            assert_eq!(report.availability.state, "unknown");
            assert_eq!(report.error.unwrap().code, "remote-unavailable");
        }
    }
}
