#![forbid(unsafe_code)]
//! Runs `.github/scripts/pin-latest-release.sh` against real Git tags on a local
//! `origin` and a stub `cachix` that records its arguments: only the newest
//! vX.Y.Z release pins, with exactly the expected `cachix pin` call.
//!
//! Usage: pin-latest-release-check SCRIPT (needs `git`, `bash` and `awk` on PATH).

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const RUNTIME: &str = "/nix/store/00000000000000000000000000000000-eon";

struct Fixture {
    root: PathBuf,
    script: PathBuf,
    path: String,
}

impl Fixture {
    fn new(script: PathBuf) -> Fixture {
        let root = env::temp_dir().join("pin-latest-release-check");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let cachix = root.join("bin/cachix");
        fs::write(
            &cachix,
            "#!/bin/sh\n{ printf '[%s]' \"$@\"; echo; } >> \"$CACHIX_LOG\"\nexit \"${CACHIX_EXIT:-0}\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&cachix).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&cachix, permissions).unwrap();
        let path = format!(
            "{}:{}",
            root.join("bin").display(),
            env::var("PATH").unwrap()
        );
        let fixture = Fixture { root, script, path };
        let source = fixture.root.join("source");
        fixture.git(&fixture.root, &["init", "-q", "source"]);
        fixture.git(&source, &["commit", "-q", "--allow-empty", "-m", "release"]);
        fixture
    }

    fn git(&self, dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .env("HOME", self.root.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "user.name=Check",
                "-c",
                "user.email=check@example.invalid",
            ])
            .args(["-c", "init.defaultBranch=edge"])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    /// A fresh `origin` holding exactly `tags`, and a checkout whose `origin` is it.
    fn origin(&self, case: &str, tags: &[&str]) -> PathBuf {
        let case_dir = self.root.join(case);
        fs::create_dir_all(&case_dir).unwrap();
        self.git(&case_dir, &["init", "-q", "--bare", "origin.git"]);
        let source = self.root.join("source");
        for tag in tags {
            self.git(&source, &["tag", "-f", tag]);
        }
        let origin = case_dir.join("origin.git");
        let origin = origin.to_str().unwrap();
        let mut push = vec!["push", "-q", origin];
        let refs: Vec<String> = tags.iter().map(|tag| format!("refs/tags/{tag}")).collect();
        push.extend(refs.iter().map(String::as_str));
        if !tags.is_empty() {
            self.git(&source, &push);
        }
        for tag in tags {
            self.git(&source, &["tag", "-d", tag]);
        }
        self.git(&case_dir, &["init", "-q", "checkout"]);
        let checkout = case_dir.join("checkout");
        self.git(&checkout, &["remote", "add", "origin", origin]);
        checkout
    }

    /// Runs the script from `checkout` and returns its output and the `cachix` calls.
    fn run(&self, checkout: &Path, case: &Case) -> (Output, String) {
        let log = checkout.join("cachix.log");
        let output = Command::new("bash")
            .arg(&self.script)
            .args([case.platform, case.runtime])
            .current_dir(checkout)
            .env("PATH", &self.path)
            .env("HOME", self.root.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GITHUB_REF", case.git_ref)
            .env("CACHIX_LOG", &log)
            .env("CACHIX_EXIT", if case.cachix_fails { "1" } else { "0" })
            .output()
            .unwrap();
        let calls = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        (output, calls)
    }
}

/// One run of the script and what it must do.
struct Case {
    platform: &'static str,
    git_ref: &'static str,
    runtime: &'static str,
    cachix_fails: bool,
    exit: i32,
    /// Whether the script calls `cachix pin`, with exactly the expected arguments.
    pins: bool,
}

const fn case(git_ref: &'static str, exit: i32, pins: bool) -> Case {
    Case {
        platform: "x86_64-linux",
        git_ref,
        runtime: RUNTIME,
        cachix_fails: false,
        exit,
        pins,
    }
}

fn check(fixture: &Fixture, checkout: &Path, cases: &[Case]) {
    for case in cases {
        let (output, calls) = fixture.run(checkout, case);
        // One bracketed argument each, so joined or split arguments never match.
        let expected = if case.pins {
            format!(
                "[pin][eon-yazelix][latest-release-{}][{}][--keep-revisions][1]\n",
                case.platform, case.runtime
            )
        } else {
            String::new()
        };
        let shown = format!(
            "{} {} {}: exit {:?}, cachix calls {calls:?}\nstdout: {}\nstderr: {}",
            case.platform,
            case.git_ref,
            case.runtime,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(output.status.code(), Some(case.exit), "{shown}");
        assert_eq!(calls, expected, "{shown}");
        println!(
            "ok: {} {} {}: exit {}",
            case.platform, case.git_ref, case.runtime, case.exit
        );
    }
}

fn main() {
    let script = env::args_os()
        .nth(1)
        .expect("usage: pin-latest-release-check SCRIPT");
    let fixture = Fixture::new(fs::canonicalize(script).unwrap());

    // Release candidates, vendor tags and nested names never outrank a release.
    let mixed = fixture.origin(
        "mixed",
        &[
            "v0.1.9",
            "v0.2.0",
            "v0.3.0-rc1",
            "vendor-x",
            "nested/v9.9.9",
        ],
    );
    check(
        &fixture,
        &mixed,
        &[
            case("refs/tags/v0.2.0", 0, true),
            // Each platform pins its own name.
            Case {
                platform: "aarch64-darwin",
                ..case("refs/tags/v0.2.0", 0, true)
            },
            case("refs/tags/v0.1.9", 0, false),
            case("refs/tags/v0.3.0-rc1", 0, false),
            case("refs/heads/edge", 0, false),
            // A runtime outside the store, or a failing pin, fails the run.
            Case {
                runtime: "result",
                ..case("refs/tags/v0.2.0", 1, false)
            },
            Case {
                cachix_fails: true,
                ..case("refs/tags/v0.2.0", 1, true)
            },
        ],
    );

    // Version order, not string order.
    let ordered = fixture.origin("ordered", &["v0.9.0", "v0.10.0"]);
    check(
        &fixture,
        &ordered,
        &[
            case("refs/tags/v0.10.0", 0, true),
            case("refs/tags/v0.9.0", 0, false),
        ],
    );

    // Without any release tag on origin the run cannot tell, so it fails.
    let unreleased = fixture.origin("unreleased", &["v0.3.0-rc1", "vendor-x"]);
    check(&fixture, &unreleased, &[case("refs/tags/v0.2.0", 1, false)]);
}
