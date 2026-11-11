//! Real prompt scenarios for managed-command-path; script owns each isolated PTY.
use std::{
    env, fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const READY: &str = "__EON_READY__ ";

struct Shell {
    child: Child,
    output: Receiver<Vec<u8>>,
    home: PathBuf,
    transcript: Vec<u8>,
    initial: String,
    nu: bool,
}

impl Shell {
    fn start(name: &str, shell: &str, rc: &str, settings: &str, term: &str) -> Self {
        let args: Vec<_> = env::args().collect();
        let home = Path::new(&env::var("TMPDIR").unwrap()).join(name);
        fs::create_dir_all(home.join(".config/nushell")).unwrap();
        fs::create_dir_all(home.join("config")).unwrap();
        fs::create_dir_all(home.join("run")).unwrap();
        fs::create_dir_all(home.join("cache")).unwrap();
        fs::write(home.join(".bashrc"), rc).unwrap();
        fs::write(
            home.join(".config/nushell/config.nu"),
            if shell == "nu" { rc } else { "" },
        )
        .unwrap();
        fs::write(home.join(".config/nushell/env.nu"), "").unwrap();
        fs::write(
            home.join("config/config.toml"),
            format!("[shell]\n{settings}"),
        )
        .unwrap();
        fs::write(
            home.join("starship.toml"),
            format!("format = '{READY}'\nright_format = ''\n"),
        )
        .unwrap();
        let mut child = Command::new(&args[2])
            .args([
                "-qefc",
                &format!(
                    "{}/libexec/eon/bin/eon-{shell} {}",
                    args[1],
                    if shell == "bash" { "-i" } else { "" }
                ),
                "/dev/null",
            ])
            .env_clear()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_RUNTIME_DIR", home.join("run"))
            .env("EON_CONFIG_HOME", home.join("config"))
            .env("STARSHIP_CONFIG", home.join("starship.toml"))
            .env("PATH", format!("{}/libexec/eon/bin:{}", args[1], args[5]))
            .env("TERM", "xterm-256color")
            .env("TERM_PROGRAM", term)
            .env("SHELL", &args[6])
            .env("LC_ALL", "C.UTF-8")
            .env("USER", "eon-shell-check")
            .current_dir(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            while let Ok(n) = stdout.read(&mut bytes) {
                if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut result = Self {
            child,
            output,
            home,
            transcript: Vec::new(),
            initial: String::new(),
            nu: shell == "nu",
        };
        let initial = result.prompt(false, false);
        if shell == "bash" && !name.starts_with("owned") {
            assert!(
                marks(&initial).is_empty(),
                "initial command marks: {initial:?}"
            );
        }
        result.initial = initial;
        result
    }

    fn send(&mut self, bytes: &[u8]) {
        self.child.stdin.as_mut().unwrap().write_all(bytes).unwrap();
    }

    fn read_until(&mut self, done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut data = Vec::new();
        let mut queries = 0;
        loop {
            let bytes = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "{}: {error}: {:?}",
                        self.home.display(),
                        String::from_utf8_lossy(&data)
                    )
                });
            self.transcript.extend_from_slice(&bytes);
            data.extend(bytes);
            let text = String::from_utf8_lossy(&data);
            let count = text.matches("\x1b[6n").count();
            for _ in queries..count {
                self.send(b"\x1b[1;1R");
            }
            queries = count;
            if done(&text) {
                return text.into_owned();
            }
        }
    }

    fn prompt(&mut self, cycle: bool, completed: bool) -> String {
        let nu = self.nu;
        self.read_until(|text| {
            if cycle && !text.contains("\x1b[?2004l") || completed && !text.contains("\x1b]133;D;")
            {
                return false;
            }
            let Some(at) = text.rfind(READY) else {
                return false;
            };
            if cycle && at < text.rfind("\x1b[?2004l").unwrap()
                || completed && at < text.rfind("\x1b]133;D;").unwrap()
            {
                return false;
            }
            let tail = &text[at + READY.len()..];
            if nu {
                tail.contains("\x1b[?25h") && plain(tail).trim().is_empty()
            } else {
                plain(tail).trim().is_empty()
            }
        })
    }

    fn command(&mut self, command: &str, status: i32) -> String {
        self.send(format!("\x1b[200~{command}\x1b[201~\r").as_bytes());
        let result = self.prompt(self.nu, true);
        assert_eq!(
            marks(&result),
            ["C".to_string(), format!("D;{status}")],
            "{command}: {result:?}"
        );
        result
    }

    fn input(&mut self, bytes: &[u8]) -> String {
        self.send(bytes);
        self.prompt(self.nu && bytes == b"\r", self.nu && bytes == b"\x03")
    }

    fn output_prompt(&mut self, output: &str) -> String {
        self.read_until(|text| {
            let (Some(output), Some(prompt)) = (text.find(output), text.rfind(READY)) else {
                return false;
            };
            prompt > output && plain(&text[prompt + READY.len()..]).trim().is_empty()
        })
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        fs::write(self.home.join("transcript.bin"), &self.transcript).unwrap();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Strip terminal controls only in these test-owned transcripts.
fn plain(text: &str) -> String {
    let mut chars = text.chars();
    let mut result = String::new();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('(') => {
                chars.next();
            }
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut esc = false;
                for c in chars.by_ref() {
                    if c == '\x07' || esc && c == '\\' {
                        break;
                    }
                    esc = c == '\x1b';
                }
            }
            _ => {}
        }
    }
    result
}

fn marks(text: &str) -> Vec<String> {
    text.split("\x1b]133;")
        .skip(1)
        .filter_map(|part| {
            let mark = part.split(['\x07', '\x1b']).next().unwrap();
            (mark == "C" || mark.starts_with("D;")).then(|| mark.to_string())
        })
        .collect()
}

fn common(shell: &str, name: &str, rc: &str, settings: &str, term: &str) {
    let mut s = Shell::start(name, shell, rc, settings, term);
    if shell == "nu" {
        assert!(s.initial.contains("\x1b]133;A;") && s.initial.contains("\x1b]133;B\x1b\\"));
        assert!(s.initial.contains("\x1b]7;file://") && !s.initial.contains("\x1b]633;"));
    }
    if name == "blesh" {
        assert!(
            !s.initial.contains("ble.sh: insane environment"),
            "{}",
            s.initial
        );
        s.command("test -n \"${BLE_ATTACHED-}\"", 0);
    }
    if name == "bash-preexec" {
        s.command("test -n \"${bash_preexec_imported-}\"", 0);
        let args: Vec<_> = env::args().collect();
        let recorder = s.home.join("record-atuin");
        fs::write(&recorder, format!("#!{}\nprintf '%s\\n' \"$*\" >>\"$HOME/atuin-calls\"\nexec {}/libexec/eon/bin/atuin \"$@\"\n", args[6], args[1])).unwrap();
        fs::set_permissions(&recorder, fs::Permissions::from_mode(0o755)).unwrap();
        s.command("_eon_atuin=$HOME/record-atuin", 0);
    }
    let success = s.command("printf '%s%s\\n' hel lo", 0);
    assert!(success.find("\x1b]133;C").unwrap() < success.find("hello\r\n").unwrap());
    s.command("bash -c 'exit 7'", 7);
    if name == "bash-preexec" {
        // Observe the hook's real CLI call, without depending on Atuin's async DB commit.
        s.command("for _check in {1..100}; do grep -E '^history end --exit 7([[:space:]]|$)' \"$HOME/atuin-calls\" && break; sleep .05; done", 0);
        assert!(
            fs::read_to_string(s.home.join("atuin-calls"))
                .unwrap()
                .lines()
                .any(|line| line.starts_with("history end --exit 7 "))
        );
    }
    s.command("echo pipeline | bash -c 'cat >/dev/null; exit 7'", 7);
    if shell == "bash" {
        s.command("false | true", 0);
        s.command("set -o pipefail", 0);
        s.command("false | true", 1);
        s.command("set +o pipefail", 0);
        s.command("true foo", 0);
        assert!(plain(&s.command("printf '<%s>\\n' \"$_\"", 0)).contains("<foo>"));
        s.command("PS0=changed", 0);
        s.command("true", 0);
        assert!(marks(&s.input(b"\r")).is_empty());
        assert!(marks(&s.input(b"\x03")).is_empty());
        assert!(marks(&s.input(b"\x0c")).is_empty());
        if name == "blesh" {
            assert_eq!(marks(&s.input(b")\n")), ["C", "D;2"]);
        } else {
            assert!(marks(&s.input(b"if ;\r")).is_empty());
        }
        s.command("true", 0);
    } else {
        let empty = s.input(b"\r");
        let idle = s.input(b"\x03");
        println!(
            "{name}: empty {:?}, idle Ctrl+C {:?}",
            marks(&empty),
            marks(&idle)
        );
        assert_eq!(marks(&empty), ["C"]);
        assert_eq!(marks(&idle), ["D;7"]);
        assert!(!empty.contains("\x1b]633;") && !idle.contains("\x1b]633;"));
        assert!(plain(&s.command("print $env.TERM_PROGRAM", 0)).contains(term));
    }
    s.send(b"\x1b[200~bash -c 'echo RUNNING; exec sleep 30'\x1b[201~\r");
    let before = s.read_until(|text| text.contains("RUNNING\r\n"));
    assert_eq!(marks(&before), ["C"]);
    s.send(b"\x03");
    let after = s.prompt(false, true);
    assert_eq!(
        marks(&after),
        [if shell == "nu" { "D;-2" } else { "D;130" }],
        "{after:?}"
    );
    s.command("echo recovered", 0);
    if name == "bash-default" {
        s.command("false | true", 0);
        assert!(plain(&s.command("printf 'STARSHIP=%s:%s\\n' \"$STARSHIP_CMD_STATUS\" \"${STARSHIP_PIPE_STATUS[*]}\"", 0)).contains("STARSHIP=0:1 0"));
        assert!(
            plain(&s.command("test -n \"$STARSHIP_DURATION\" && echo timer-preserved", 0))
                .contains("timer-preserved")
        );
    }
    println!("{name}: command, status, pipeline, interrupt and recovery passed");
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let off = "starship=false\nzoxide=false\natuin=false\ncarapace=false\nmise=false\n";
    common("bash", "bash-default", "", "", "eon");
    common("nu", "nu-default", "", "", "eon");
    common("nu", "nu-vscode", "", "", "vscode");
    common(
        "nu",
        "nu-off",
        &format!(
            "$env.PROMPT_COMMAND = {{|| '{READY}' }}\n$env.PROMPT_COMMAND_RIGHT = {{|| '' }}\n$env.PROMPT_INDICATOR = ''\n"
        ),
        off,
        "eon",
    );
    common("bash", "bash-off", &format!("PS1='{READY}'\n"), off, "eon");
    common(
        "bash",
        "bash-nounset",
        &format!("PS1='{READY}'\nset -u\n"),
        off,
        "eon",
    );
    for (name, framework) in [("bash-preexec", &args[3]), ("blesh", &args[4])] {
        common(
            "bash",
            name,
            &format!("source {framework}\nPS1='{READY}'\n"),
            "",
            "eon",
        );
    }
    let rc = format!(
        "PS1='{READY}'\nuser_prompt() {{ printf '__HOOK=%s:%s\\n' \"$?\" \"${{PIPESTATUS[*]}}\"; }}\nPROMPT_COMMAND=user_prompt\nif [[ ${{_check_rc_read-}} ]]; then return 7; fi\n_check_rc_read=1\n"
    );
    let mut s = Shell::start("scalar-hook", "bash", &rc, off, "eon");
    assert!(s.command("false | true", 0).contains("__HOOK=0:1 0"));
    s.command("source \"$EON_BASH_RC\"", 0);
    s.command("false", 1);
    assert_eq!(marks(&s.input(b"source ~/.bashrc\r")), ["C"]);
    s.send(b"true\r");
    assert_eq!(marks(&s.prompt(false, true)), ["D;7", "C", "D;0"]);
    s.command("false", 1);
    let rc = rc.replace(
        "PROMPT_COMMAND=user_prompt",
        "PROMPT_COMMAND=(user_prompt ':')",
    );
    let mut s = Shell::start("array-hook", "bash", &rc, off, "eon");
    assert!(s.command("false | true", 0).contains("__HOOK=0:1 0"));
    let mut s = Shell::start(
        "a-only",
        "bash",
        &format!(
            r#"PS1='{READY}'
PROMPT_COMMAND='printf "\e]133;A\a"'
"#
        ),
        off,
        "eon",
    );
    assert!(s.command("false", 1).contains("\x1b]133;A\x07"));
    let rc = format!(
        "PS1='{READY}'\nowned_start() {{ printf '\\e]133;C\\a'; }}\nPS0='${{ owned_start; }}'\nowned_end() {{ printf '\\e]133;D;%s\\a' \"$?\"; }}\nPROMPT_COMMAND=owned_end\n"
    );
    let mut s = Shell::start("owned-marks", "bash", &rc, off, "eon");
    s.command("false", 1);
    let out = s.command("declare -F _eon_command_start", 1);
    assert!(!plain(&out).lines().any(|line| line == "_eon_command_start"));
    let rc = format!(
        "PS1='{READY}'\nowned_end() {{ printf '\\e]133;D;%s\\a' \"$?\"; }}\nPROMPT_COMMAND=owned_end\n"
    );
    let mut s = Shell::start("owned-d-only", "bash", &rc, off, "eon");
    let out = s.input(b"declare -F _eon_command_start\r");
    assert_eq!(marks(&out), ["D;1"]);
    let mut s = Shell::start("atuin-accept", "bash", "", "", "eon");
    s.command("bind -x '\"\\C-x\\C-e\": __atuin_accept_line false'", 0);
    s.send(b"\x18\x05");
    let accepted = s.prompt(false, true);
    assert_eq!(marks(&accepted), ["C", "D;1"]);
    s.command("true", 0);
    let mut s = Shell::start("nested", "nu", "", "", "eon");
    s.send(b"eon-bash -i\r");
    s.nu = false;
    let nested = s.prompt(false, false);
    assert_eq!(marks(&nested), ["C"]);
    s.command("false", 1);
    s.nu = true;
    s.send(b"\x04");
    let outer = s.prompt(false, true);
    assert_eq!(marks(&outer), ["D;1"]);
    s.command("echo after-nesting", 0);
    let mut s = Shell::start("redirected", "bash", "", "", "eon");
    s.command("exec >capture", 0);
    s.command("printf captured", 0);
    s.command("bind -x '\"\\C-x\\C-e\": __atuin_accept_line false'", 0);
    s.send(b"\x18\x05");
    assert_eq!(marks(&s.prompt(false, true)), ["C", "D;1"]);
    assert!(
        !fs::read_to_string(s.home.join("capture"))
            .unwrap()
            .contains("\x1b]133;")
    );
    s.send(b"exec 2>errors\r");
    assert_eq!(
        marks(&s.read_until(|text| text.contains("\x1b]133;D;0"))),
        ["C", "D;0"]
    );
    s.command("exec 2>/dev/tty", 0);
    assert!(
        !fs::read_to_string(s.home.join("errors"))
            .unwrap()
            .contains("\x1b]133;")
    );
    let mut s = Shell::start("piped", "bash", "", "", "eon");
    s.send(b"eon-bash -i | cat >pipe-capture\r");
    assert_eq!(marks(&s.prompt(false, false)), ["C"]);
    s.command("printf captured", 0);
    s.send(b"\x04");
    assert_eq!(marks(&s.prompt(false, true)), ["D;0"]);
    let captured = fs::read_to_string(s.home.join("pipe-capture")).unwrap();
    assert!(captured.contains("captured") && !captured.contains("\x1b]133;"));
    fs::write(s.home.join("input"), "printf plain\\n\nfalse\nexit\n").unwrap();
    let setsid = Path::new(&args[2]).parent().unwrap().join("setsid");
    s.command(
        &format!(
            "{} --wait eon-bash -i <input >no-tty-output 2>no-tty-errors",
            setsid.display()
        ),
        1,
    );
    for file in ["no-tty-output", "no-tty-errors"] {
        assert!(
            !fs::read_to_string(s.home.join(file))
                .unwrap()
                .contains("\x1b]133;")
        );
    }
    let mut s = Shell::start("piped-nu", "bash", "", "atuin=false\n", "eon");
    s.send(b"eon-nu | tee nu-capture\r");
    assert_eq!(marks(&s.prompt(false, false)), ["C"]);
    s.send(b"\x1b[200~$env.config.shell_integration.osc133 = true; print ('pipe-' + 'payload')\x1b[201~\r");
    let result = s.output_prompt("pipe-payload\r\n");
    assert!(marks(&result).is_empty());
    s.send(b"exit\r");
    assert_eq!(marks(&s.prompt(false, true)), ["D;0"]);
    let captured = fs::read_to_string(s.home.join("nu-capture")).unwrap();
    assert!(captured.contains("pipe-payload") && !captured.contains("\x1b]133;"));
    let mut s = Shell::start(
        "nu-opt-out",
        "nu",
        "$env.config.shell_integration.osc133 = false\n",
        "",
        "eon",
    );
    assert!(!s.initial.contains("\x1b]133;"));
    s.send(b"\x1b[200~print ('opt-' + 'out')\x1b[201~\r");
    assert!(!s.output_prompt("opt-out\r\n").contains("\x1b]133;"));
    let mut s = Shell::start("stopped-job", "bash", "", "", "eon");
    s.send(b"bash -c 'echo STOP_READY; exec sleep 30'\r");
    assert_eq!(
        marks(&s.read_until(|text| text.contains("STOP_READY\r\n"))),
        ["C"]
    );
    s.send(b"\x1a");
    assert_eq!(marks(&s.prompt(false, true)), ["D;148"]);
    s.command("kill -KILL \"$(jobs -p %+)\"", 0);
    println!("managed shell prompt scenarios passed");
}
