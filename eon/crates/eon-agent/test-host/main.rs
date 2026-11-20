//! A test-only rota host with a slot route, for tests that need one. It is the `rota`
//! host plus Boat (by default) or a fake route (`ROTA_TEST_ROUTE=fake`). Its source sits
//! outside the package's `include`, so `cargo package` drops this target.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod fake {
    use rota::slots::{Fallback, Kind, Listing, Local, Placement, Remote, Request, Route, Stage};
    use serde_json::Value;
    use std::io::{self, Write};
    use std::path::Path;
    use std::process::ExitStatus;

    /// A canonical source (sorted keys, no spaces) with nested values, escapes,
    /// non-ASCII text and the largest u64; the tests hold the same literal.
    pub const SOURCE: &str =
        r#"{"a":[1,{"b":"é\n\"q\""}],"n":18446744073709551615,"z":{"y":true}}"#;

    /// Behaves per `ROTA_TEST_ROUTE_MODE` (`declined`, `kept`, `ungranted`, `null`,
    /// `null-exit`, or empty for a returned fallback with `SOURCE`) and appends one line per call that receives a
    /// source to `ROTA_TEST_ROUTE_LOG`: `<call> <host|exec> <source>`.
    pub struct Fake {
        pub mode: String,
    }

    fn fallback(reason: &str, kind: Kind) -> Fallback {
        Fallback {
            reason: reason.into(),
            kind,
        }
    }

    fn record(call: &str, source: Option<&Value>) {
        let Some(path) = std::env::var_os("ROTA_TEST_ROUTE_LOG") else {
            return;
        };
        let process = if std::env::args().any(|arg| arg == "_slot-local-exec") {
            "exec"
        } else {
            "host"
        };
        let source = source.map_or_else(|| "none".into(), Value::to_string);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .expect("route log");
        writeln!(file, "{call} {process} {source}").expect("route log write");
    }

    impl Route for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn place(&self, _: &Request<'_>, stage: Stage) -> Placement {
            match (self.mode.as_str(), stage) {
                ("declined", _) => {
                    Placement::Local(Some(fallback("fake-declined", Kind::Declined)))
                }
                ("kept", _) => Placement::Local(Some(fallback("fake-kept", Kind::Kept))),
                _ => Placement::Remote,
            }
        }

        fn run_remote(
            &self,
            _: &Request<'_>,
            granted: &mut dyn FnMut(),
            source: &mut Option<Value>,
        ) -> Remote {
            if self.mode != "ungranted" {
                granted();
            }
            *source = Some(if self.mode.starts_with("null") {
                Value::Null
            } else {
                serde_json::from_str(SOURCE).expect("source literal")
            });
            if self.mode == "null-exit" {
                return Remote::Exit {
                    code: 0,
                    memory: None,
                };
            }
            Remote::Local(fallback("fake-returned", Kind::Returned))
        }

        fn run_local(&self, local: Local<'_>) -> io::Result<ExitStatus> {
            record("run_local", local.source);
            local.command.status()
        }

        fn verify(&self, source: &Value) -> Result<(), String> {
            record("verify", Some(source));
            Ok(())
        }

        fn listing(
            &self,
            _: &Path,
            _: &dyn Fn(&std::fs::Metadata) -> Option<Option<u32>>,
        ) -> Result<Listing, String> {
            Ok(Listing::default())
        }
    }
}

#[cfg(target_os = "linux")]
static BOAT: rota::slots::boat::Boat = rota::slots::boat::Boat::with_patience(20);

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    let session = |_: std::time::Instant| Err("native quota requires eon agent".into());
    let fake;
    let route: &dyn rota::slots::Route = match std::env::var("ROTA_TEST_ROUTE").as_deref() {
        Ok("fake") => {
            fake = fake::Fake {
                mode: std::env::var("ROTA_TEST_ROUTE_MODE").unwrap_or_default(),
            };
            &fake
        }
        // Boat with a 20 s patience, so lost-attach tests need not wait ten minutes.
        _ => &BOAT,
    };
    rota::cli::main(
        std::env::args_os().skip(1),
        &rota::cli::Host {
            route: Some(route),
            ..rota::cli::Host::rota(&session)
        },
    )
}

#[cfg(not(target_os = "linux"))]
fn main() {}
