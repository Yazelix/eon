//! Read-only local checks shared by the standalone command and the bug report.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    io::Write,
    os::unix::fs::FileTypeExt,
    path::Path,
    process::{Command, ExitStatus},
    time::Duration,
};

use eon_runtime::{CallerBuild, DiagnosticConfiguration, DiagnosticWindow, Inputs};
use serde_json::json;

use crate::{
    launch,
    update::{self, Bounded, Build, LocalInstallation, LocalUpdate},
};

pub(crate) const LOCAL_PROBE_LIMIT: Duration = Duration::from_secs(5);

#[derive(PartialEq, Eq)]
enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn label(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

struct Check {
    id: &'static str,
    status: Status,
    message: String,
    fix: Option<&'static str>,
}

impl Check {
    fn pass(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Pass,
            message: message.into(),
            fix: None,
        }
    }

    fn warn(id: &'static str, message: impl Into<String>, fix: &'static str) -> Self {
        Self {
            id,
            status: Status::Warn,
            message: message.into(),
            fix: Some(fix),
        }
    }

    fn fail(id: &'static str, message: impl Into<String>, fix: &'static str) -> Self {
        Self {
            id,
            status: Status::Fail,
            message: message.into(),
            fix: Some(fix),
        }
    }
}

pub(crate) struct Diagnostics {
    checks: [Check; 7],
    pub installation: LocalInstallation,
    pub configuration_warnings: Result<Vec<String>, String>,
    pub window: DiagnosticWindow,
    pub compositor: String,
}

impl Diagnostics {
    pub(crate) fn collect(program: &OsStr, invoked: Build, inputs: &Inputs, report: bool) -> Self {
        let window = eon_runtime::diagnostic_window(inputs, report);
        let mut config = eon_runtime::diagnostic_configuration(inputs);
        let (installation, route) =
            update::local_installations(program, invoked.clone(), config.updates.take());
        let programs = launch::diagnostic_programs(inputs.launch.as_ref().ok());
        let display = Display::current();
        let checks = [
            install_check(
                &installation,
                &window.caller_build,
                invoked.build.as_deref(),
            ),
            config_check(&config),
            terminal_check(inputs, &window.caller_build, programs.infocmp.as_deref()),
            display.check(),
            Check::warn(
                "gpu",
                "Unknown GPU adapter",
                "Include the GPU and driver when reporting rendering trouble.",
            ),
            editor_check(inputs, config.editor_available, programs.helix.as_deref()),
            update_check(&route),
        ];
        Self {
            checks,
            installation,
            configuration_warnings: config.warnings,
            window,
            compositor: display.description(),
        }
    }

    pub(crate) fn human(&self) -> String {
        let mut text = String::new();
        for check in &self.checks {
            use std::fmt::Write as _;
            writeln!(
                text,
                "{} {} — {}",
                check.status.label(),
                check.id,
                check.message
            )
            .unwrap();
            if let Some(fix) = check.fix {
                writeln!(text, "  Fix  {fix}").unwrap();
            }
        }
        text
    }

    fn json(&self) -> String {
        let checks = self
            .checks
            .iter()
            .map(|check| {
                json!({
                    "id": check.id,
                    "status": check.status.label(),
                    "message": check.message,
                    "fix": check.fix,
                })
            })
            .collect::<Vec<_>>();
        format!("{}\n", json!({"checks": checks}))
    }

    fn exit_status(&self) -> u8 {
        u8::from(self.checks.iter().any(|check| check.status == Status::Fail))
    }
}

pub(crate) fn run(arguments: &[OsString], invoked: Build, inputs: &Inputs) -> u8 {
    let json = match &arguments[2..] {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("eon: invalid doctor arguments\nRun eon --help for commands and examples.");
            return 2;
        }
    };
    let diagnostics = Diagnostics::collect(&arguments[0], invoked, inputs, false);
    let output = if json {
        diagnostics.json()
    } else {
        diagnostics.human()
    };
    write_stdout(&output, diagnostics.exit_status())
}

pub(crate) fn write_stdout(output: &str, status: u8) -> u8 {
    match io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => status,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => status,
        Err(error) => {
            eprintln!("eon: cannot write stdout: {error}");
            1
        }
    }
}

fn broken_provenance(installation: &LocalInstallation) -> bool {
    installation.variant_mismatch
        || installation.reason == Some("provenance-invalid")
        || (installation.reason == Some("pinned-install")
            && installation.kind == "nix-profile"
            && !installation.proved)
}

fn install_check(
    installation: &LocalInstallation,
    caller: &CallerBuild,
    invoked: Option<&str>,
) -> Check {
    let id = "install";
    let build = match (caller, invoked) {
        (CallerBuild::Known(active), Some(invoked)) if active != invoked => {
            return Check::fail(
                id,
                "This CLI and the calling window use different Eon builds.",
                "Use the window's own launcher for its work or open a separate window with the intended build.",
            );
        }
        (CallerBuild::Known(_), Some(_)) => "The CLI and calling window use the same build.",
        (CallerBuild::Outside, _) => "No calling window outside Eon.",
        _ => "Could not verify the calling window's active build.",
    };
    let route = match installation.kind {
        "nix-profile" => "Nix profile",
        "eon-install" => "Eon installer",
        "externally-managed" => "Declarative installation (provenance managed externally)",
        "unmanaged" => "Unmanaged installation",
        _ => "Unknown install route",
    };
    if broken_provenance(installation) {
        return Check::fail(
            id,
            format!("{route} has invalid provenance. {build}"),
            "Verify or reinstall the package through the original installation route.",
        );
    }
    if !installation.proved && installation.kind != "externally-managed" {
        return Check::warn(
            id,
            format!("{route}. Could not verify installation provenance. {build}"),
            "Check through your external installed Eon launcher or verify the artifact with your package manager.",
        );
    }
    if !matches!(caller, CallerBuild::Outside)
        && (invoked.is_none() || matches!(caller, CallerBuild::Unknown))
    {
        return Check::warn(
            id,
            format!("{route}. {build}"),
            "Run eon doctor from a terminal in the affected window using that window's launcher.",
        );
    }
    Check::pass(id, format!("{route}. {build}"))
}

fn config_check(config: &DiagnosticConfiguration) -> Check {
    if !config.valid {
        Check::fail(
            "config",
            "Eon configuration is unreadable or invalid.",
            "Run eon config check to see the configuration error.",
        )
    } else if config
        .warnings
        .as_ref()
        .is_ok_and(|warnings| !warnings.is_empty())
    {
        Check::warn(
            "config",
            "Eon accepted the configuration with warnings.",
            "Review Configuration warnings in eon report and remove or replace the named settings.",
        )
    } else {
        Check::pass("config", "Eon configuration is valid.")
    }
}

fn terminal_check(inputs: &Inputs, caller: &CallerBuild, infocmp: Option<&Path>) -> Check {
    let inside = !matches!(caller, CallerBuild::Outside);
    let status = infocmp.map(|program| {
        let mut command = Command::new(program);
        command.args(["-x", "eon"]);
        if inside && let Ok(launch) = &inputs.launch {
            match launch.caller_terminfo_dirs() {
                Some(dirs) => {
                    command.env("TERMINFO_DIRS", dirs);
                }
                None => {
                    command.env_remove("TERMINFO_DIRS");
                }
            }
        }
        update::bounded_status(command, LOCAL_PROBE_LIMIT)
    });
    match status {
        Some(Ok(status)) if status.success() => {
            if inside && env::var_os("TERM").as_deref() != Some(OsStr::new("eon")) {
                Check::warn(
                    "terminal",
                    "The Eon terminfo entry resolves but this terminal does not use TERM=eon.",
                    "Retry from a plain Eon pane and remove any shell override of TERM.",
                )
            } else {
                Check::pass("terminal", "The Eon terminfo entry resolves.")
            }
        }
        Some(Ok(status)) if status.code().is_some() => Check::fail(
            "terminal",
            "The Eon terminfo entry did not resolve.",
            "Check the package's terminfo and the shell settings for TERMINFO and TERMINFO_DIRS.",
        ),
        _ => Check::warn(
            "terminal",
            "Could not check the Eon terminfo entry with the packaged infocmp.",
            "Run doctor through the installed Eon launcher and verify the package if the check remains unavailable.",
        ),
    }
}

struct Display {
    macos: bool,
    desktop: Option<String>,
    session: Option<String>,
    wayland_display: Option<OsString>,
    wayland_socket: bool,
    runtime: Option<OsString>,
}

impl Display {
    fn current() -> Self {
        Self {
            macos: cfg!(target_os = "macos"),
            desktop: env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .filter(|name| !name.trim().is_empty()),
            session: env::var("XDG_SESSION_TYPE").ok(),
            wayland_display: env::var_os("WAYLAND_DISPLAY").filter(|value| !value.is_empty()),
            wayland_socket: env::var_os("WAYLAND_SOCKET").is_some_and(|value| !value.is_empty()),
            runtime: env::var_os("XDG_RUNTIME_DIR"),
        }
    }

    fn description(&self) -> String {
        compositor(self.macos, self.desktop.as_deref(), self.session.as_deref())
    }

    fn check(&self) -> Check {
        let id = "display";
        let fix =
            "Run Eon from the affected native Wayland desktop with its inherited environment.";
        if self.macos {
            return Check::warn(
                id,
                "The macOS desktop is in preview.",
                "Include the macOS version and display setup if filing a report.",
            );
        }
        if self.wayland_socket {
            return Check::warn(
                id,
                "Could not verify the inherited Wayland display descriptor.",
                fix,
            );
        }
        let socket = self.wayland_display.as_deref().and_then(|name| {
            let path = Path::new(name);
            if path.is_absolute() {
                return Some(path.to_path_buf());
            }
            self.runtime
                .as_deref()
                .map(Path::new)
                .filter(|root| root.is_absolute())
                .map(|root| root.join(path))
        });
        match socket {
            Some(socket) => match fs::metadata(socket) {
                Ok(metadata) if metadata.file_type().is_socket() => {
                    match self.desktop.as_deref().filter(|desktop| {
                        desktop.split(':').all(|name| {
                            !name.is_empty()
                                && name.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || b"-_".contains(&byte)
                                })
                        })
                    }) {
                        Some(desktop) => {
                            Check::pass(id, format!("Wayland socket present for {desktop}."))
                        }
                        None => Check::warn(
                            id,
                            "Wayland socket present. The compositor name is unknown.",
                            "Include the compositor name if filing a report.",
                        ),
                    }
                }
                Ok(_) => Check::fail(id, "The advertised Wayland path is not a socket.", fix),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Check::fail(id, "The advertised Wayland socket is missing.", fix)
                }
                Err(_) => Check::warn(id, "Could not inspect the advertised Wayland socket.", fix),
            },
            None if self.wayland_display.is_none() && self.session.as_deref() == Some("x11") => {
                Check::fail(
                    id,
                    "This environment identifies an X11 session with no Wayland display.",
                    fix,
                )
            }
            None => Check::warn(
                id,
                "Could not identify a Wayland socket in this environment.",
                fix,
            ),
        }
    }
}

/// The desktop and session type as the report names them.
pub(crate) fn compositor(macos: bool, desktop: Option<&str>, session: Option<&str>) -> String {
    if macos {
        return "macOS".into();
    }
    match (desktop.filter(|desktop| !desktop.is_empty()), session) {
        (Some(desktop), Some(session)) if !session.is_empty() => format!("{desktop} ({session})"),
        (Some(desktop), _) => desktop.into(),
        (None, _) => "unknown (XDG_CURRENT_DESKTOP is unset)".into(),
    }
}

fn editor_check(inputs: &Inputs, available: Option<bool>, helix: Option<&Path>) -> Check {
    let id = "editor";
    if available == Some(false) {
        return Check::fail(
            id,
            "No executable editor is selected.",
            "Install the selected editor or set [editor] command to an executable in config.toml.",
        );
    }

    if inputs.managed.helix.is_some() {
        let Some(helix) = helix else {
            return Check::warn(
                id,
                "Could not verify the packaged Helix executable.",
                "Run eon doctor through the installed Eon launcher.",
            );
        };
        let mut command = Command::new(helix);
        command.args(["--config", "/dev/null", "--log", "/dev/null", "--version"]);
        if let Some(check) =
            helix_version_finding(update::bounded_status(command, LOCAL_PROBE_LIMIT))
        {
            return check;
        }
    }

    if available.is_none() {
        return Check::warn(
            id,
            "Could not determine whether the effective editor resolves.",
            "Check eon config check and the editor setting in the calling environment.",
        );
    }
    Check::pass(
        id,
        if inputs.managed.helix.is_some() {
            "The selected editor resolves and packaged Helix runs."
        } else {
            "The selected editor resolves."
        },
    )
}

fn helix_version_finding(result: Result<ExitStatus, Bounded>) -> Option<Check> {
    match result {
        Ok(status) if status.success() => None,
        Ok(_) | Err(Bounded::Spawn(_)) => Some(Check::fail(
            "editor",
            "The packaged Helix version command failed.",
            "Verify or reinstall the full Eon package through its installation route.",
        )),
        Err(_) => Some(Check::warn(
            "editor",
            "Could not verify that the packaged Helix version command runs.",
            "Retry eon doctor and check the package if the probe remains unavailable.",
        )),
    }
}

fn update_check(route: &LocalUpdate) -> Check {
    let id = "update";
    let target_fix =
        "Check the [updates] target in config.toml and verify that installation through its owner.";
    match route {
        LocalUpdate::ExternalLauncher => Check::pass(
            id,
            "Package maintenance finds its external Eon launcher on PATH.",
        ),
        LocalUpdate::NoExternalLauncher => Check::warn(
            id,
            "Could not select an external Eon launcher for maintenance.",
            "Put the installed Eon launcher on PATH or configure the [updates] target in config.toml.",
        ),
        LocalUpdate::ConfigurationUnknown => Check::warn(
            id,
            "Could not read the configured update route.",
            "Run eon config check and correct the configuration error.",
        ),
        LocalUpdate::InvalidConfiguration => {
            Check::fail(id, "The configured update target is invalid.", target_fix)
        }
        LocalUpdate::Target {
            installation,
            configured,
        } => {
            if *configured && (installation.error.is_some() || broken_provenance(installation)) {
                return Check::fail(
                    id,
                    "The configured update target is unreadable or has invalid provenance.",
                    target_fix,
                );
            }
            match (installation.kind, installation.reason) {
                ("nix-profile" | "eon-install", None | Some("pinned-install"))
                    if installation.proved =>
                {
                    Check::pass(id, "The update route is readable locally.")
                }
                ("externally-managed" | "unmanaged", _) => Check::warn(
                    id,
                    "This route uses manual package maintenance.",
                    "Update through the package manager or installation method you used to install Eon.",
                ),
                (_, Some("trust-unconfigured")) => Check::warn(
                    id,
                    "The installer route is readable, but its signer is not configured.",
                    "Provision an independent allowed-signers file and set updates.allowed_signers.",
                ),
                _ => Check::warn(
                    id,
                    "Could not verify an update route ready for package maintenance.",
                    target_fix,
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn helix_distinguishes_failed_versions_from_inconclusive_waits() {
        assert!(helix_version_finding(Ok(ExitStatus::from_raw(0))).is_none());
        for result in [
            Ok(ExitStatus::from_raw(1 << 8)),
            Err(Bounded::Spawn(io::ErrorKind::NotFound.into())),
        ] {
            assert_eq!(
                helix_version_finding(result).unwrap().status.label(),
                "FAIL"
            );
        }
        for result in [
            Err(Bounded::TimedOut(LOCAL_PROBE_LIMIT)),
            Err(Bounded::Wait(io::Error::other("test wait failure"))),
            Err(Bounded::Interrupted),
        ] {
            assert_eq!(
                helix_version_finding(result).unwrap().status.label(),
                "WARN"
            );
        }
    }
}
