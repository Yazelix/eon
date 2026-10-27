//! Test-only supplied inputs; no Eon assembly or graph dependency.

use crate::{
    Defaults, Inputs, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand, PopupDefinition,
    ShellConfig, StartupAnimation, TerminalConfig,
};
use eon_workspace_protocol::v21::{ALT, PopupGeometry, PopupKey, SHIFT, Shortcut};

pub(crate) fn inputs() -> Inputs {
    Inputs {
        version: "fixture-product",
        component_report: Ok("fixture-components".into()),
        codexbar: "missing-fixture-codexbar".into(),
        assembly: b"fixture-assembly",
        defaults: Defaults {
            bar_widgets: &[
                "codex_quota",
                "claude_quota",
                "cpu",
                "ram",
                "clock",
                "version",
            ],
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
            sidebar_enabled: true,
            popups: PopupCatalog {
                geometry: PopupGeometry {
                    side_margin: 11.0,
                    vertical_margin: 7.0,
                },
                entries: [
                    ("project", "KeyZ", ALT, PopupCommand::Project, false),
                    (
                        "git",
                        "KeyJ",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["fixture-git".into()]),
                        true,
                    ),
                    ("agent", "KeyL", ALT | SHIFT, PopupCommand::AgentAuto, true),
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
            custom_popup_keep_alive: true,
            agent_commands: &[],
        },
        orbit: "fixture-orbit".into(),
        venus: "fixture-venus".into(),
        anima: "fixture-animation".into(),
        managed: ManagedPrograms {
            nu: "fixture-nu".into(),
            bash: "fixture-bash".into(),
            helix: "fixture-editor".into(),
            yazi: "fixture-files".into(),
            ya: "fixture-file-helper".into(),
            lazygit: "fixture-git".into(),
            nu_vendor_autoload: None,
            bash_rc: None,
            shell_bin: None,
        },
    }
}
