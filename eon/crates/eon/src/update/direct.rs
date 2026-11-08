//! Read-only discovery for installer-owned payloads. Metadata never authorizes bytes.
use super::*;
use std::os::unix::fs::MetadataExt;

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
    if helper.is_some() {
        unavailable(
            &mut report,
            "trust-unconfigured",
            "Discovery has not verified any signature or archive. Provision an independently trusted allowed_signers file as docs/INSTALLER.md describes; direct Install is pending.",
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
    if !report.proved || report.capability.reason == Some("unsupported-platform") {
        return report;
    }
    match releases(host, Instant::now() + Duration::from_secs(30)) {
        Err((reason, message)) => {
            report.availability = unknown(Some(reason));
            report.fail("remote-unavailable", "check", message);
        }
        Ok(None) => unavailable(
            &mut report,
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
                    &mut report,
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
                    &mut report,
                    "source-unverified",
                    "The same release version names different archive bytes; verify the source manually.",
                ),
            }
            report.candidate_release = Some(candidate);
        }
    }
    report
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::tests::{BUILD, Fake, Tree, at, invoked};
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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
