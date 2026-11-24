//! Test-only supplied inputs; no Eon assembly or graph dependency.

use crate::{
    BarLoad, Defaults, Inputs, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand,
    PopupDefinition, ShellConfig, SlotClass, SlotPolicy, StartupAnimation, TerminalAccess,
    TerminalConfig,
};
use eon_workspace_protocol::v50::{ALT, PopupGeometry, PopupKey, SHIFT, Shortcut};

pub(crate) fn input_lock() -> Shortcut {
    Shortcut {
        modifiers: ALT | SHIFT,
        key: "KeyI".into(),
    }
}

pub(crate) fn assert_not_update_validator(resource: &str) {
    assert!(
        std::env::var_os("EON_TEST_PREFLIGHT_LIFETIME").is_none(),
        "the read-only Update child tried to open {resource}"
    );
}

pub(crate) fn make_fifo(path: &std::path::Path) {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    #[cfg(target_os = "linux")]
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    assert!(
        std::process::Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.file_type().is_fifo());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
}

/// Returns the listener and the clients keeping its accept queue full.
pub(crate) fn busy_listener(path: &std::path::Path) -> (socket2::Socket, Vec<socket2::Socket>) {
    use socket2::{Domain, SockAddr, Socket, Type};
    use std::io::ErrorKind;

    let address = SockAddr::unix(path).unwrap();
    let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
    listener.bind(&address).unwrap();
    // Darwin treats a backlog of zero as somaxconn.
    listener.listen(1).unwrap();
    let mut queued = Vec::new();
    for _ in 0..16 {
        let client = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        client.set_nonblocking(true).unwrap();
        match client.connect(&address) {
            Ok(()) => queued.push(client),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::ConnectionRefused
                ) =>
            {
                assert!(!queued.is_empty(), "the listener refused its first client");
                return (listener, queued);
            }
            Err(error) => panic!(
                "cannot fill backlog after {} connections: {error}",
                queued.len()
            ),
        }
    }
    panic!("backlog never filled after {} connections", queued.len());
}

pub(crate) fn inputs() -> Inputs {
    Inputs {
        launch: Err("unwrapped fixture".into()),
        target_launch_facts: |_| Err("no installed target fixture".into()),
        update_store: "/nix/store".into(),
        version: "fixture-product",
        release_stage: crate::ReleaseStage::Alpha,
        skill: "fixture-skill",
        component_report: Ok("fixture-components".into()),
        presentation_protocols: Ok([
            eon_workspace_protocol::v50::VERSION,
            orbit_protocol::session::VERSION,
            orbit_protocol::VERSION,
        ]),
        composed_components: Ok("fixture-components".into()),
        codexbar: "missing-fixture-codexbar".into(),
        assembly: b"fixture-assembly",
        slot_policy: SlotPolicy {
            count: 2,
            default_class: SlotClass::Heavy,
            nix_max_running: 1,
            priority_burst: 1,
            nice: 19,
            cargo_jobs: 6,
            test_threads: 2,
            wait_timeout_seconds: 0,
            poll_seconds: 5,
            status_interval_seconds: 60,
            wait_alert_seconds: 2700,
            max_load_per_cpu: 0.75,
            min_available_memory_percent: 20.0,
            min_available_memory_mib: 4096,
            max_memory_pressure_percent: 10.0,
            max_io_pressure_percent: 50.0,
            guard_clear_seconds: 20,
            quiet_max_seconds: 5400,
        },
        defaults: Defaults {
            bar_widgets: &[
                "codex_quota_5h",
                "codex_quota_7d",
                "claude_quota_5h",
                "claude_quota_7d",
                "claude_quota_scoped",
                "cpu",
                "ram",
                "disk",
                "clock",
                "version",
            ],
            bar_load: BarLoad {
                ram_notice_percent: 60,
                ram_warning_percent: 75,
                ram_critical_percent: 90,
                cpu_notice_percent: 60,
                cpu_warning_percent: 75,
                cpu_critical_percent: 90,
                cpu_average_seconds: 30,
                cpu_temperature_warning_celsius: 85,
                cpu_temperature_critical_celsius: 95,
                disk_notice_percent: 60,
                disk_warning_percent: 75,
                disk_critical_percent: 90,
            },
            bar_gpu_interval_seconds: 30,
            bar_disk_path: "~",
            shell: ShellConfig {
                command: vec!["fixture-shell".into()],
                starship: true,
                zoxide: true,
                atuin: true,
                carapace: true,
                mise: true,
            },
            terminal: TerminalConfig {
                background_opacity: 0.6,
                background_blur: true,
                padding_color: PaddingColor::Extend,
                pane_frames: true,
                panel_heading: crate::PanelHeading::Connected,
                detect_local_hosts: true,
                focus_color: None,
                border_width: None,
                focus_border_width: None,
                cursor_trail_color: None,
                cursor_trail_duration: None,
                font_family: None,
                font_fallbacks: Vec::new(),
                font_size: None,
                line_height: None,
                columns: None,
                rows: None,
            },
            anima: StartupAnimation {
                enabled: true,
                style: "fixture-animation".into(),
                duration_seconds: 2,
            },
            warning_close_last_tab: true,
            animate_working_badge: true,
            number_keys: true,
            sidebar_enabled: true,
            attention: crate::AttentionNotifications {
                notify: true,
                notify_names: true,
            },
            terminal_access: TerminalAccess::Window,
            send_enter_delay_ms: 300,
            send_typing_quiet_ms: 2000,
            popups: PopupCatalog {
                input_lock: crate::fixtures::input_lock(),
                released_shortcuts: Default::default(),
                geometry: PopupGeometry {
                    side_margin: 11.0,
                    vertical_margin: 7.0,
                },
                entries: [
                    (
                        "project",
                        "KeyZ",
                        ALT,
                        PopupCommand::Argv(Vec::new()),
                        false,
                    ),
                    (
                        "git",
                        "KeyJ",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["fixture-git".into()]),
                        true,
                    ),
                    ("agent", "KeyL", ALT | SHIFT, PopupCommand::AgentAuto, true),
                    (
                        "files",
                        "KeyY",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["fixture-files".into()]),
                        true,
                    ),
                    (
                        "anima",
                        "KeyA",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["fixture-animation".into()]),
                        false,
                    ),
                ]
                .into_iter()
                .map(
                    |(id, key, modifiers, command, keep_alive)| PopupDefinition {
                        id: id.into(),
                        label: format!("Fixture {id}"),
                        shortcut: PopupKey::Bound(Shortcut {
                            modifiers,
                            key: key.into(),
                        }),
                        command,
                        keep_alive,
                    },
                )
                .collect(),
            },
            ansi_palette: "111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111,111111",
            terminal_default_colors: [[0xff; 3], [0; 3]],
            custom_popup_keep_alive: true,
            editor: Some(&["fixture-editor"]),
            editor_kind: crate::EditorKind::Auto,
            editor_open_in: crate::EditorOpenIn::Popup,
            agent_commands: &[],
            agent_launch: |_, _| Ok(None),
        },
        orbit: "fixture-orbit".into(),
        venus: "fixture-venus".into(),
        anima: "fixture-animation".into(),
        icon: None,
        managed: ManagedPrograms {
            nu: "fixture-nu".into(),
            bash: "fixture-bash".into(),
            helix: Some("fixture-editor".into()),
            yazi: "fixture-files".into(),
            ya: "fixture-file-helper".into(),
            lazygit: "fixture-git".into(),
            nu_vendor_autoload: None,
            bash_rc: None,
            shell_bin: None,
        },
    }
}
