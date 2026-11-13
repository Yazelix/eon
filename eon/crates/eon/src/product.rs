use eon_runtime::{
    BarLoad, Defaults, Inputs, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand,
    PopupDefinition, ReleaseStage, ShellConfig, SlotClass, SlotPolicy, StartupAnimation,
    TerminalAccess, TerminalConfig,
};
use eon_workspace_protocol::v43::{ALT, PopupGeometry, PopupKey, SHIFT, Shortcut};

pub(crate) const RELEASE_STAGE: ReleaseStage = ReleaseStage::Alpha;

pub(crate) fn inputs() -> Inputs {
    const MANIFEST: &str = include_str!("../../../../components/eon-v5.json");
    // The composition supplies its Helix here; without it, this build has none.
    let helix = nonempty_environment_path("EON_HX");
    let omitted: &[&str] = if helix.is_some() { &[] } else { &["helix"] };
    let (component_report, composed_components) =
        match eon_manifest::version_reports(MANIFEST, omitted) {
            Ok((whole, composed)) => (Ok(whole), Ok(composed)),
            Err(error) => (Err(error.to_string()), Err(error.to_string())),
        };
    Inputs {
        version: env!("CARGO_PKG_VERSION"),
        release_stage: RELEASE_STAGE,
        skill: include_str!("../../../skills/eon/SKILL.md"),
        component_report,
        presentation_protocols: eon_manifest::presentation_protocols(MANIFEST)
            .map_err(|error| error.to_string()),
        composed_components,
        assembly: env!("EON_WORKSPACE_DIGEST").as_bytes(),
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
            bar_disk_path: "~",
            shell: ShellConfig {
                command: vec!["eon-nu".into()],
                starship: true,
                zoxide: true,
                atuin: true,
                carapace: true,
                mise: true,
            },
            terminal: TerminalConfig {
                background_opacity: 0.8,
                background_blur: true,
                padding_color: PaddingColor::Extend,
                pane_frames: true,
                panel_heading: eon_runtime::PanelHeading::Connected,
                detect_local_hosts: true,
                focus_color: None,
                border_width: None,
                focus_border_width: None,
                cursor_trail_color: None,
                cursor_trail_duration: Some(1.5),
                font_family: None,
                font_fallbacks: if option_env!("EON_WITHOUT_ACCELERANDO") == Some("1") {
                    vec!["Symbols Nerd Font Mono".into()]
                } else {
                    vec![
                        "Symbols Nerd Font Mono".into(),
                        "Accelerando Terminal".into(),
                    ]
                },
                font_size: None,
                line_height: None,
                columns: None,
                rows: None,
            },
            anima: StartupAnimation {
                enabled: true,
                style: "random".into(),
                duration_seconds: 3,
            },
            warning_close_last_tab: true,
            animate_working_badge: true,
            number_keys: true,
            sidebar_enabled: true,
            terminal_access: TerminalAccess::Window,
            send_enter_delay_ms: 300,
            send_typing_quiet_ms: 2000,
            popups: PopupCatalog {
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: [
                    // Project has no command of its own: it shows the tab's Files pair
                    // on quick search (EON-C18).
                    (
                        "project",
                        "Quick search",
                        "KeyZ",
                        ALT,
                        PopupCommand::Argv(Vec::new()),
                        false,
                    ),
                    (
                        "git",
                        "Git",
                        "KeyJ",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["eon-lazygit".into()]),
                        true,
                    ),
                    (
                        "agent",
                        "Agent",
                        "KeyL",
                        ALT | SHIFT,
                        PopupCommand::AgentAuto,
                        true,
                    ),
                    (
                        "editor",
                        "Editor",
                        "KeyO",
                        ALT | SHIFT,
                        PopupCommand::Editor,
                        true,
                    ),
                    (
                        "files",
                        "Files",
                        "KeyY",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["eon-files".into()]),
                        true,
                    ),
                    (
                        "anima",
                        "Anima",
                        "KeyA",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["anima".into()]),
                        false,
                    ),
                    (
                        "commands",
                        "Commands",
                        "KeyM",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["eon-commands".into()]),
                        false,
                    ),
                    (
                        "sessions",
                        "Sessions",
                        "KeyS",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["eon-sessions".into()]),
                        false,
                    ),
                    // Scrollback's command is built for each press (EON-C10).
                    (
                        "scrollback",
                        "Scrollback",
                        "KeyE",
                        ALT | SHIFT,
                        PopupCommand::Scrollback,
                        false,
                    ),
                ]
                .into_iter()
                .map(
                    |(id, label, key, modifiers, command, keep_alive)| PopupDefinition {
                        id: id.into(),
                        label: label.into(),
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
            ansi_palette: "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff",
            custom_popup_keep_alive: true,
            // The packaged Helix on Eon's terminal PATH; none in a build without Helix.
            editor: helix.as_ref().map(|_| &["hx"][..]),
            editor_kind: eon_runtime::EditorKind::Auto,
            editor_open_in: eon_runtime::EditorOpenIn::Popup,
            agent_commands: &[
                // Codex's shared daemon would run this terminal's eon commands as another terminal.
                &["codex", "resume", "--no-daemon"],
                &["grok"],
                &["opencode"],
                &["pi"],
                &["claude", "--resume"],
            ],
        },
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        codexbar: configured_program("EON_CODEXBAR", "codexbar"),
        anima: configured_program("EON_ANIMA", "anima"),
        icon: eon_icon(std::env::var_os("EON_ICON")),
        managed: ManagedPrograms {
            nu: configured_program("EON_NU", "nu"),
            bash: configured_program("EON_BASH", "bash"),
            helix,
            yazi: configured_program("EON_YAZI", "yazi"),
            ya: configured_program("EON_YA", "ya"),
            lazygit: configured_program("EON_LAZYGIT", "lazygit"),
            nu_vendor_autoload: nonempty_environment_path("EON_NU_VENDOR_AUTOLOAD"),
            bash_rc: nonempty_environment_path("EON_BASH_RC"),
            shell_bin: nonempty_environment_path("EON_SESSION_BIN"),
        },
    }
}

fn configured_program(variable: &str, fallback: &str) -> std::path::PathBuf {
    std::env::var_os(variable).map_or_else(|| fallback.into(), std::path::PathBuf::from)
}

/// The Eon icon's image from `EON_ICON`. Venus refuses a relative `--eon-icon`, so a relative
/// path counts as no icon rather than stopping every workspace window from launching.
fn eon_icon(value: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn nonempty_environment_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{eon_icon, inputs};

    #[test]
    fn packaged_font_defaults_follow_the_accelerando_omission() {
        let fonts = inputs().defaults.terminal.font_fallbacks;
        assert!(fonts.iter().any(|font| font == "Symbols Nerd Font Mono"));
        assert_eq!(
            fonts.iter().any(|font| font == "Accelerando Terminal"),
            option_env!("EON_WITHOUT_ACCELERANDO") != Some("1")
        );
    }

    #[test]
    fn agent_popup_starts_codex_without_its_shared_daemon() {
        let codex = inputs()
            .defaults
            .agent_commands
            .iter()
            .filter(|argv| argv[0] == "codex")
            .collect::<Vec<_>>();
        assert!(!codex.is_empty());
        for argv in codex {
            assert!(argv.contains(&"--no-daemon"), "{argv:?}");
        }
    }

    #[test]
    fn eon_icon_takes_only_an_absolute_path() {
        assert_eq!(
            eon_icon(Some("/nix/store/x-eon/share/eon/eon-icon-256.rgba".into())),
            Some("/nix/store/x-eon/share/eon/eon-icon-256.rgba".into())
        );
        for value in [
            None,
            Some(""),
            Some("eon-icon.rgba"),
            Some("./share/eon-icon.rgba"),
        ] {
            assert_eq!(eon_icon(value.map(Into::into)), None, "{value:?}");
        }
    }
}
