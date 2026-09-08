use serde::Deserialize;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn configured_program(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable).map_or_else(|| fallback.into(), PathBuf::from)
}

pub(super) fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EonConfig {
    shell: ShellConfig,
    terminal: TerminalConfig,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct ShellConfig {
    command: Vec<String>,
    starship: bool,
    zoxide: bool,
    atuin: bool,
    carapace: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            command: vec!["eon-nu".into()],
            starship: true,
            zoxide: true,
            atuin: true,
            carapace: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TerminalConfig {
    pub(crate) background_opacity: f32,
    pub(crate) background_blur: bool,
    pub(crate) font_family: Option<String>,
    pub(crate) font_fallbacks: Vec<String>,
    pub(crate) font_size: Option<f32>,
    pub(crate) line_height: Option<f32>,
    pub(crate) columns: Option<u16>,
    pub(crate) rows: Option<u16>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            background_opacity: 0.8,
            background_blur: true,
            font_family: None,
            font_fallbacks: Vec::new(),
            font_size: None,
            line_height: None,
            columns: None,
            rows: None,
        }
    }
}

impl TerminalConfig {
    pub(crate) fn requires_startup_admission(&self) -> bool {
        self.font_family.is_some()
            || !self.font_fallbacks.is_empty()
            || self.font_size.is_some()
            || self.line_height.is_some()
            || self.columns.is_some()
            || self.rows.is_some()
    }
}

struct ManagedPrograms {
    nu: PathBuf,
    bash: PathBuf,
    zsh: PathBuf,
    fish: PathBuf,
    helix: PathBuf,
    yazi: PathBuf,
    ya: PathBuf,
    lazygit: PathBuf,
    nu_vendor_autoload: Option<PathBuf>,
    bash_rc: Option<PathBuf>,
    zsh_config: Option<PathBuf>,
    fish_init: Option<PathBuf>,
    shell_bin: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Nu,
    Bash,
    Zsh,
    Fish,
    Helix,
    Yazi,
    Ya,
    LazyGit,
}

pub(crate) fn tool(invocation: &OsStr) -> Option<Tool> {
    let name = Path::new(invocation).file_name()?.to_str()?;
    match name {
        "eon-nu" | "nu" => Some(Tool::Nu),
        "eon-bash" | "bash" => Some(Tool::Bash),
        "eon-zsh" | "zsh" => Some(Tool::Zsh),
        "eon-fish" | "fish" => Some(Tool::Fish),
        "eon-hx" | "hx" => Some(Tool::Helix),
        "eon-yazi" | "yazi" => Some(Tool::Yazi),
        "eon-ya" | "ya" => Some(Tool::Ya),
        "eon-lazygit" | "eon-lg" | "lazygit" => Some(Tool::LazyGit),
        _ => None,
    }
}

pub(crate) fn command(
    tool: Tool,
    config: &Path,
    arguments: &[OsString],
) -> Result<Command, String> {
    managed_command(tool, &managed_programs(), config, arguments)
}

pub(crate) fn shell_command(root: &Path) -> Result<Vec<String>, String> {
    Ok(read_shell_config(root)?.command)
}

pub(crate) fn terminal_presentation(root: &Path) -> Result<TerminalConfig, String> {
    let terminal = read_config(root)?.terminal;
    if !terminal.background_opacity.is_finite()
        || !(0.0..=1.0).contains(&terminal.background_opacity)
    {
        return Err(
            "terminal.background_opacity must be a finite number from 0.0 through 1.0".into(),
        );
    }
    for (field, value, minimum, maximum) in [
        ("font_size", terminal.font_size, 6.0, 96.0),
        ("line_height", terminal.line_height, 1.0, 3.0),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(minimum..=maximum).contains(&value)) {
            return Err(format!(
                "terminal.{field} must be a finite number from {minimum} through {maximum}"
            ));
        }
    }
    if terminal.font_fallbacks.len() > 8 {
        return Err("terminal.font_fallbacks accepts at most eight families".into());
    }
    for (field, family) in terminal
        .font_family
        .iter()
        .map(|family| ("font_family", family))
        .chain(
            terminal
                .font_fallbacks
                .iter()
                .map(|family| ("font_fallbacks", family)),
        )
    {
        if family.is_empty()
            || family.trim() != family
            || family.len() > 128
            || family.chars().any(char::is_control)
        {
            return Err(format!(
                "terminal.{field} requires nonempty trimmed family names of at most 128 UTF-8 bytes without controls"
            ));
        }
    }
    if terminal.columns == Some(0)
        || terminal.rows == Some(0)
        || u32::from(terminal.columns.unwrap_or(1)) * u32::from(terminal.rows.unwrap_or(1))
            > orbit_protocol::MAX_CELLS as u32
    {
        return Err("terminal.columns and terminal.rows must be positive and fit Orbit's 100,000-cell limit".into());
    }
    Ok(terminal)
}

pub(crate) fn session_path(prefix: &Path) -> Result<OsString, String> {
    prepend_path(prefix, env::var_os("PATH").as_deref(), "Eon Session")
}

fn managed_command(
    tool: Tool,
    programs: &ManagedPrograms,
    config: &Path,
    arguments: &[OsString],
) -> Result<Command, String> {
    let shell = match tool {
        Tool::Nu | Tool::Bash | Tool::Zsh | Tool::Fish => Some(read_shell_config(config)?),
        _ => None,
    };
    let program = match tool {
        Tool::Nu => &programs.nu,
        Tool::Bash => &programs.bash,
        Tool::Zsh => &programs.zsh,
        Tool::Fish => &programs.fish,
        Tool::Helix => &programs.helix,
        Tool::Yazi => &programs.yazi,
        Tool::Ya => &programs.ya,
        Tool::LazyGit => &programs.lazygit,
    };
    let mut command = Command::new(program);
    match tool {
        Tool::Nu => {
            if let Some(path) = &programs.nu_vendor_autoload {
                command.env(
                    "NU_VENDOR_AUTOLOAD_DIR",
                    path.join(
                        integration_mask(
                            shell.as_ref().unwrap(),
                            env::var_os("ATUIN_NOBIND").is_some(),
                        )
                        .to_string(),
                    ),
                );
            }
        }
        Tool::Bash => {
            if let Some(path) = &programs.bash_rc {
                command.args([OsStr::new("--rcfile"), path.as_os_str()]);
            }
        }
        Tool::Zsh => {
            if let Some(path) = &programs.zsh_config {
                command.env("ZDOTDIR", path);
                if let Some(user) = env::var_os("EON_USER_ZDOTDIR")
                    .or_else(|| env::var_os("ZDOTDIR"))
                    .or_else(|| env::var_os("HOME"))
                {
                    command.env("EON_USER_ZDOTDIR", user);
                }
            }
        }
        Tool::Fish => {
            if let Some(path) = &programs.fish_init {
                command
                    .env("EON_FISH_INIT", path)
                    .args(["-C", "source \"$EON_FISH_INIT\""]);
            }
        }
        Tool::Yazi | Tool::Ya => {
            command.env_remove("YAZI_CONFIG_HOME");
        }
        Tool::LazyGit => {
            command
                .env_remove("CONFIG_DIR")
                .env_remove("LG_CONFIG_FILE")
                .env("XDG_CONFIG_DIRS", config);
        }
        Tool::Helix => {
            command
                .env_remove("CARGO_MANIFEST_DIR")
                .env_remove("HELIX_RUNTIME")
                .env_remove("HELIX_STEEL_CONFIG");
        }
    }
    if let Some(shell) = &shell {
        for (name, enabled) in [
            ("STARSHIP", shell.starship),
            ("ZOXIDE", shell.zoxide),
            ("ATUIN", shell.atuin),
            ("CARAPACE", shell.carapace),
        ] {
            command.env(format!("EON_SHELL_{name}"), if enabled { "1" } else { "0" });
        }
        if let Some(bin) = &programs.shell_bin {
            command.env(
                "PATH",
                prepend_path(bin, env::var_os("PATH").as_deref(), "managed shell")?,
            );
        }
    }
    command.args(arguments).env("EON_CONFIG_HOME", config);
    if shell.is_none() {
        command.env("XDG_CONFIG_HOME", config);
    }
    Ok(command)
}

fn integration_mask(shell: &ShellConfig, atuin_nobind: bool) -> u8 {
    u8::from(shell.starship)
        | (u8::from(shell.zoxide) << 1)
        | (u8::from(shell.atuin) << 2)
        | (u8::from(shell.carapace) << 3)
        | (u8::from(shell.atuin && atuin_nobind) << 4)
}

fn managed_programs() -> ManagedPrograms {
    ManagedPrograms {
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
    }
}

fn read_shell_config(root: &Path) -> Result<ShellConfig, String> {
    let config = read_config(root)?;
    if config.shell.command.is_empty() || config.shell.command[0].is_empty() {
        return Err("shell.command must not be empty".into());
    }
    if config
        .shell
        .command
        .iter()
        .any(|argument| argument.contains('\0'))
    {
        return Err("shell.command must not contain NUL".into());
    }
    Ok(config.shell)
}

fn read_config(root: &Path) -> Result<EonConfig, String> {
    let path = root.join("config.toml");
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EonConfig::default());
        }
        Err(error) => {
            return Err(format!("cannot read {}: {error}", path.display()));
        }
    };
    toml::from_str(&source)
        .map_err(|error| format!("invalid Eon configuration {}: {error}", path.display()))
}

fn prepend_path(prefix: &Path, path: Option<&OsStr>, owner: &str) -> Result<OsString, String> {
    let mut paths = path
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .filter(|path| path != prefix)
        .collect::<Vec<_>>();
    paths.insert(0, prefix.into());
    env::join_paths(paths).map_err(|error| format!("cannot construct {owner} PATH: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        ManagedPrograms, Tool, integration_mask, managed_command, prepend_path, read_shell_config,
        terminal_presentation, tool,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn shell_configuration_is_strict_and_defaults_without_a_file() {
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-0",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let default = read_shell_config(&root).unwrap();
        assert_eq!(default.command, ["eon-nu"]);
        assert!(default.starship && default.zoxide && default.atuin && default.carapace);

        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-fish\", \"--no-config\"]\nstarship = false\nzoxide = false\natuin = false\ncarapace = false\n",
        )
        .unwrap();
        let configured = read_shell_config(&root).unwrap();
        assert_eq!(configured.command, ["eon-fish", "--no-config"]);
        assert!(
            !configured.starship && !configured.zoxide && !configured.atuin && !configured.carapace
        );
        assert_eq!(integration_mask(&configured, true), 0);
        assert_eq!(integration_mask(&default, false), 15);
        assert_eq!(integration_mask(&default, true), 31);

        for (source, expected) in [
            ("[shell]\ncommand = []\n", "shell.command must not be empty"),
            (
                "[shell]\ncommand = [\"eon-nu\"]\nunknown = true\n",
                "unknown field `unknown`",
            ),
            ("[shell\n", "invalid Eon configuration"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(read_shell_config(&root).unwrap_err().contains(expected));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_presentation_is_strict_and_bounded() {
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-presentation",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let presentation = terminal_presentation(&root).unwrap();
        assert_eq!(presentation.background_opacity, 0.8);
        assert!(presentation.background_blur);

        for (source, expected_opacity, expected_blur) in [
            ("", 0.8, true),
            ("[shell]\nstarship = false\n", 0.8, true),
            ("[terminal]\nbackground_opacity = 0.0\n", 0.0, true),
            (
                "[terminal]\nbackground_opacity = 0.88\nbackground_blur = false\n",
                0.88,
                false,
            ),
            (
                "[terminal]\nbackground_opacity = 1.0\nbackground_blur = true\n",
                1.0,
                true,
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let presentation = terminal_presentation(&root).unwrap();
            assert_eq!(presentation.background_opacity, expected_opacity);
            assert_eq!(presentation.background_blur, expected_blur);
        }

        for (source, field) in [
            (
                "[terminal]\nbackground_opacity = nan\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = inf\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = -0.01\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = 1.01\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = \"0.88\"\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = 0.5\nbackground_opacity = 0.6\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_blur = \"true\"\n",
                "background_blur",
            ),
            (
                "[terminal]\nbackground_blur = true\nbackground_blur = false\n",
                "background_blur",
            ),
            ("[terminal]\nunknown = true\n", "unknown"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                terminal_presentation(&root).unwrap_err().contains(field),
                "invalid {field} configuration did not name its field"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_invocation_names_are_bounded() {
        use Tool::{Bash, Fish, Helix, LazyGit, Nu, Ya, Yazi, Zsh};

        for (name, expected) in [
            ("eon-nu", Some(Nu)),
            ("nu", Some(Nu)),
            ("eon-bash", Some(Bash)),
            ("bash", Some(Bash)),
            ("eon-zsh", Some(Zsh)),
            ("zsh", Some(Zsh)),
            ("eon-fish", Some(Fish)),
            ("fish", Some(Fish)),
            ("eon-hx", Some(Helix)),
            ("hx", Some(Helix)),
            ("eon-yazi", Some(Yazi)),
            ("yazi", Some(Yazi)),
            ("eon-ya", Some(Ya)),
            ("ya", Some(Ya)),
            ("eon-lazygit", Some(LazyGit)),
            ("eon-lg", Some(LazyGit)),
            ("lazygit", Some(LazyGit)),
            ("lg", None),
            ("eon-starship", None),
            ("eon-zoxide", None),
            ("eon", None),
        ] {
            assert_eq!(tool(Path::new(name).as_os_str()), expected);
        }
        assert_eq!(
            tool(Path::new("/nix/store/example/bin/eon-nu").as_os_str()),
            Some(Nu)
        );
    }

    #[test]
    fn managed_commands_use_exact_programs_and_private_configuration() {
        use Tool::{Bash, Fish, Helix, LazyGit, Nu, Ya, Yazi, Zsh};

        let programs = ManagedPrograms {
            nu: "/managed/nu".into(),
            bash: "/managed/bash".into(),
            zsh: "/managed/zsh".into(),
            fish: "/managed/fish".into(),
            helix: "/managed/hx".into(),
            yazi: "/managed/yazi".into(),
            ya: "/managed/ya".into(),
            lazygit: "/managed/lazygit".into(),
            nu_vendor_autoload: Some("/managed/autoload".into()),
            bash_rc: Some("/managed/bashrc".into()),
            zsh_config: Some("/managed/zsh-config".into()),
            fish_init: Some("/managed/fish-init".into()),
            shell_bin: Some("/managed/bin".into()),
        };
        let config = Path::new("/private/eon");
        for (tool, program, removed_variables) in [
            (
                Helix,
                "/managed/hx",
                &["CARGO_MANIFEST_DIR", "HELIX_RUNTIME", "HELIX_STEEL_CONFIG"][..],
            ),
            (Yazi, "/managed/yazi", &["YAZI_CONFIG_HOME"][..]),
            (Ya, "/managed/ya", &["YAZI_CONFIG_HOME"][..]),
            (
                LazyGit,
                "/managed/lazygit",
                &["CONFIG_DIR", "LG_CONFIG_FILE"][..],
            ),
        ] {
            let command = managed_command(tool, &programs, config, &["--version".into()]).unwrap();
            assert_eq!(command.get_program(), program);
            assert_eq!(
                command.get_args().map(OsString::from).collect::<Vec<_>>(),
                vec![OsString::from("--version")]
            );
            for variable in ["EON_CONFIG_HOME", "XDG_CONFIG_HOME"] {
                assert_eq!(
                    command_environment(&command, variable),
                    Some(Some(config.as_os_str()))
                );
            }
            for variable in removed_variables {
                assert_eq!(command_environment(&command, variable), Some(None));
            }
            if tool == LazyGit {
                assert_eq!(
                    command_environment(&command, "XDG_CONFIG_DIRS"),
                    Some(Some(config.as_os_str()))
                );
            }
        }

        let command = managed_command(Nu, &programs, config, &["--version".into()]).unwrap();
        assert_eq!(command.get_program(), "/managed/nu");
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--version"].map(OsString::from)
        );
        assert_eq!(
            command_environment(&command, "EON_CONFIG_HOME"),
            Some(Some(config.as_os_str()))
        );
        assert_eq!(
            command_environment(&command, "NU_VENDOR_AUTOLOAD_DIR"),
            Some(Some(OsStr::new("/managed/autoload/15")))
        );
        assert_eq!(command_environment(&command, "XDG_CONFIG_HOME"), None);
        assert_eq!(command_environment(&command, "STARSHIP_CONFIG"), None);

        for (tool, program, arguments) in [
            (
                Bash,
                "/managed/bash",
                vec!["--rcfile", "/managed/bashrc", "--version"],
            ),
            (Zsh, "/managed/zsh", vec!["--version"]),
            (
                Fish,
                "/managed/fish",
                vec!["-C", "source \"$EON_FISH_INIT\"", "--version"],
            ),
        ] {
            let command = managed_command(tool, &programs, config, &["--version".into()]).unwrap();
            assert_eq!(command.get_program(), program);
            assert_eq!(
                command.get_args().map(OsString::from).collect::<Vec<_>>(),
                arguments
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            );
            for integration in ["STARSHIP", "ZOXIDE", "ATUIN", "CARAPACE"] {
                assert_eq!(
                    command_environment(&command, &format!("EON_SHELL_{integration}")),
                    Some(Some(OsStr::new("1")))
                );
            }
        }
        assert_eq!(
            command_environment(
                &managed_command(Zsh, &programs, config, &[]).unwrap(),
                "ZDOTDIR"
            ),
            Some(Some(OsStr::new("/managed/zsh-config")))
        );
        assert_eq!(
            command_environment(
                &managed_command(Fish, &programs, config, &[]).unwrap(),
                "EON_FISH_INIT"
            ),
            Some(Some(OsStr::new("/managed/fish-init")))
        );
    }

    #[test]
    fn session_path_contains_one_managed_prefix() {
        let path = prepend_path(
            Path::new("/managed/bin"),
            Some(OsStr::new("/managed/bin:/usr/bin:/managed/bin")),
            "test",
        )
        .unwrap();

        assert_eq!(
            std::env::split_paths(&path).collect::<Vec<_>>(),
            ["/managed/bin", "/usr/bin"].map(PathBuf::from)
        );
    }
}
