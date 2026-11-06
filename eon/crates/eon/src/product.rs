use eon_runtime::{
    BarLoad, Defaults, Inputs, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand,
    PopupDefinition, ShellConfig, StartupAnimation, TerminalAccess, TerminalConfig,
};
use eon_workspace_protocol::v35::{ALT, PopupGeometry, PopupKey, SHIFT, Shortcut};

pub(crate) fn inputs() -> Inputs {
    const MANIFEST: &str = include_str!("../../../../components/eon-v5.json");
    Inputs {
        version: env!("CARGO_PKG_VERSION"),
        skill: include_str!("../../../skills/eon/SKILL.md"),
        component_report: eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string()),
        assembly: env!("EON_WORKSPACE_DIGEST").as_bytes(),
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
                ram_warning_percent: 80,
                ram_critical_percent: 90,
                cpu_warning_percent: 90,
                cpu_average_seconds: 30,
                disk_warning_percent: 85,
                disk_critical_percent: 95,
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
                focus_color: None,
                border_width: None,
                focus_border_width: None,
                cursor_trail_color: None,
                cursor_trail_duration: Some(1.5),
                font_family: None,
                font_fallbacks: vec![
                    "Symbols Nerd Font Mono".into(),
                    "Accelerando Terminal".into(),
                ],
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
                        "workspaces",
                        "Workspaces",
                        "KeyS",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["eon-workspaces".into()]),
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
            // The packaged Helix on Eon's terminal PATH.
            editor: &["hx"],
            agent_commands: &[
                &["codex", "resume"],
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
            helix: configured_program("EON_HX", "hx"),
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
    use super::eon_icon;

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
