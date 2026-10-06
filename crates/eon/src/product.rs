use eon_runtime::{
    ComponentFacts, Defaults, Inputs, ManagedPrograms, PopupCatalog, PopupCommand, PopupDefinition,
    ShellConfig, StartupAnimation, TerminalConfig,
};
use eon_workspace_protocol::v7::{ALT, PopupGeometry, SHIFT, Shortcut};

pub(crate) fn inputs() -> Inputs {
    const MANIFEST: &str = include_str!("../../../components/eon-alpha-v3.json");
    const ASSEMBLY: &[&[u8]] = &[
        include_bytes!("main.rs"),
        include_bytes!("product.rs"),
        include_bytes!("../../eon-manifest/src/lib.rs"),
        include_bytes!("../../eon-manifest/Cargo.toml"),
        include_bytes!("../Cargo.toml"),
        include_bytes!("../../../Cargo.toml"),
        include_bytes!("../../../Cargo.lock"),
        include_bytes!("../../../flake.nix"),
        include_bytes!("../../../flake.lock"),
        MANIFEST.as_bytes(),
    ];
    Inputs {
        version: env!("CARGO_PKG_VERSION"),
        components: (|| {
            Ok(ComponentFacts {
                report: eon_manifest::version_report(MANIFEST)
                    .map_err(|error| error.to_string())?,
                orbit_revision: eon_manifest::component_revision(MANIFEST, "orbit")
                    .map_err(|error| error.to_string())?,
            })
        })(),
        assembly: ASSEMBLY,
        defaults: Defaults {
            shell: ShellConfig {
                command: vec!["eon-nu".into()],
                starship: true,
                zoxide: true,
                atuin: true,
                carapace: true,
            },
            terminal: TerminalConfig {
                background_opacity: 0.8,
                background_blur: true,
                pane_frames: true,
                cursor_trail_color: None,
                font_family: None,
                font_fallbacks: Vec::new(),
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
            popups: PopupCatalog {
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: [
                    (
                        "project",
                        "Project",
                        "KeyZ",
                        ALT,
                        PopupCommand::Project,
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
                        "anima",
                        "Anima",
                        "KeyA",
                        ALT | SHIFT,
                        PopupCommand::Argv(vec!["anima".into()]),
                        false,
                    ),
                ]
                .into_iter()
                .map(
                    |(id, label, key, modifiers, command, keep_alive)| PopupDefinition {
                        id: id.into(),
                        label: label.into(),
                        shortcut: Shortcut {
                            modifiers,
                            key: key.into(),
                        },
                        command,
                        keep_alive,
                    },
                )
                .collect(),
            },
            ansi_palette: "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff",
            custom_popup_keep_alive: true,
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
        anima: configured_program("EON_ANIMA", "anima"),
        managed: ManagedPrograms {
            nu: configured_program("EON_NU", "nu"),
            bash: configured_program("EON_BASH", "bash"),
            zsh: configured_program("EON_ZSH", "zsh"),
            fish: configured_program("EON_FISH", "fish"),
            helix: configured_program("EON_HX", "hx"),
            yazi: configured_program("EON_YAZI", "yazi"),
            ya: configured_program("EON_YA", "ya"),
            lazygit: configured_program("EON_LAZYGIT", "lazygit"),
            nu_vendor_autoload: nonempty_environment_path("EON_NU_VENDOR_AUTOLOAD"),
            bash_rc: nonempty_environment_path("EON_BASH_RC"),
            zsh_config: nonempty_environment_path("EON_ZSH_CONFIG"),
            fish_init: nonempty_environment_path("EON_FISH_INIT"),
            shell_bin: nonempty_environment_path("EON_SESSION_BIN"),
        },
    }
}

fn configured_program(variable: &str, fallback: &str) -> std::path::PathBuf {
    std::env::var_os(variable).map_or_else(|| fallback.into(), std::path::PathBuf::from)
}

fn nonempty_environment_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}
