//! `scenario`: full Eon launched in the Mac's logged-in GUI session and driven
//! mostly through its own CLI, with keys and captures where macOS allows them.

use crate::{
    mac,
    picture::{Picture, Region, backdrop_shares, blend, check_blend, check_shares},
    retina,
    step::{Failure, Log, Outcome, Runner, check, execute, succeed},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const MINUTE: Duration = Duration::from_secs(60);
const PAUSE: Duration = Duration::from_millis(250);
const DARK: [u8; 3] = [32; 3];
const GREY: [u8; 3] = [128; 3];
const LIGHT: [u8; 3] = [224; 3];
const OPACITY: f64 = 0.8;
/// The share of Venus's window that must differ from the desktop behind it.
const SHOWN: f64 = 0.05;
/// The share of Venus's window that a key's view must change, and that
/// pressing it again must restore to within; also the share that the Kitty
/// image's magenta must gain.
const VIEW: f64 = 0.01;
/// A 2x2 magenta image stretched over 20 columns and 10 rows, sent with the
/// pixels inline, the only medium Orbit stores (ORB-C16).
const IMAGE: &str = r"\033_Ga=T,f=24,s=2,v=2,c=20,r=10;/wD//wD//wD//wD/\033\\";

/// What a step needs from the Mac beyond the package.
#[derive(Clone, Copy)]
enum Need {
    /// The tool's user logged in to the Mac's screen.
    Gui,
    /// Screen Recording for the process that runs the tool.
    Screen,
    /// Accessibility for the process that runs the tool, which lets it post
    /// key events.
    Keys,
}

impl Need {
    fn as_str(self) -> &'static str {
        match self {
            Need::Gui => "GUI login",
            Need::Screen => "Screen Recording",
            Need::Keys => "Accessibility",
        }
    }
}

type Body = fn(&mut Scenario, &mut Log, Instant) -> Outcome;
/// Whether a need is met, or why not.
type Availability = Result<(), String>;

struct Spec {
    name: &'static str,
    needs: &'static [Need],
    /// Steps that must have passed first.
    requires: &'static [&'static str],
    /// What a pass shows.
    proves: &'static str,
    /// Whether Eon's window shows afterwards, so a capture checks it.
    window: bool,
    timeout: Duration,
    body: Body,
}

const GUI: &[Need] = &[Need::Gui];
const KEYS: &[Need] = &[Need::Gui, Need::Keys];
const BOTH: &[Need] = &[Need::Gui, Need::Keys, Need::Screen];

const STEPS: &[Spec] = &[
    Spec {
        name: "session",
        needs: &[],
        requires: &[],
        proves: "records whether the tool's user holds the Mac's screen and whether Screen \
                 Recording and Accessibility are granted; steps that need what is missing are \
                 skipped for Yu",
        window: false,
        timeout: MINUTE,
        body: session,
    },
    Spec {
        name: "launch",
        needs: GUI,
        requires: &["session"],
        proves: "`eon run` from the package, with a private HOME, configuration and runtime \
                 root, answers `eon workspace --json` and shows a Venus window",
        window: true,
        timeout: Duration::from_secs(5 * 60),
        body: launch,
    },
    Spec {
        name: "identity",
        needs: GUI,
        requires: &["launch"],
        proves: "`eon versions` names aarch64-darwin for Eon and every component, and \
                 `eon config-path` is the private configuration root",
        window: true,
        timeout: MINUTE,
        body: identity,
    },
    Spec {
        name: "retina",
        needs: GUI,
        requires: &["launch"],
        proves: "Venus logs a window scale factor of 2 or more and its window shows in a capture \
                 at that scale, on the main display as it is or switched for the step to a HiDPI \
                 mode it offers. It first records each display's scale and modes in \
                 scenario/displays.json and system_profiler's report in \
                 scenario/system-profiler-displays.json; without Screen Recording, or a HiDPI \
                 mode it can switch to, it is then skipped for Yu. It switches back before it ends, having asked \
                 macOS to undo the switch too when the tool exits",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: retina,
    },
    Spec {
        name: "picker",
        needs: KEYS,
        requires: &["launch"],
        proves: "Escape dismisses the first tab's directory picker, which has no CLI path, and \
                 its terminal runs the scenario's driver in the launch directory",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: picker,
    },
    Spec {
        name: "translucency",
        needs: BOTH,
        requires: &["picker"],
        proves: "OSC 11 sets Venus's default background to a known grey; over light and dark \
                 backdrops, its most common colour matches background_opacity * grey + \
                 (1 - background_opacity) * backdrop within the blend tolerance per channel, \
                 and the backdrop's share is within 0.05 of 1 - background_opacity. The grey \
                 and backdrops are measured through the same capture path; when something \
                 else lies behind Venus's window it is left for Yu. OSC 111 restores the \
                 default background, and the light backdrop returns before the step ends",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: translucency,
    },
    Spec {
        name: "output",
        needs: KEYS,
        requires: &["picker"],
        proves: "a command in an Eon pane sees TERM=eon, which macOS's own tput resolves, \
                 EON_TERMINAL and a nonzero terminal size, read back through a file since no CLI \
                 reads a screen",
        window: true,
        timeout: MINUTE,
        body: output,
    },
    Spec {
        name: "panes",
        needs: KEYS,
        requires: &["picker"],
        proves: "`eon pane create` from outside Eon and with a command from inside it, and \
                 `eon tab create`, show in `eon workspace --json`",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: create,
    },
    Spec {
        name: "focus",
        needs: KEYS,
        requires: &["panes"],
        proves: "`eon focus` by pane ID, down and up moves the active tab and selected pane",
        window: true,
        timeout: MINUTE,
        body: focus,
    },
    Spec {
        name: "edit",
        needs: KEYS,
        requires: &["picker"],
        proves: "`eon edit` opens a file in a Helix pane that stays live",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: edit,
    },
    Spec {
        name: "kitty",
        needs: BOTH,
        requires: &["picker"],
        proves: "a Kitty graphics image from a pane shows in Venus's window in a capture",
        window: true,
        timeout: MINUTE,
        body: kitty,
    },
    Spec {
        name: "keys",
        needs: KEYS,
        requires: &["picker"],
        proves: "Option+Shift+Y opens and hides the Files (Yazi) popup in `eon workspace \
                 --json`, and Command+V pastes `pbcopy` text, which replaces the Mac's clipboard, into a pane",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: keys,
    },
    Spec {
        name: "views",
        needs: BOTH,
        requires: &["picker"],
        proves: "Option+Shift+Z (zoom) and Option+/ (Shortcuts) each change Venus's window in a \
                 capture, and pressing them again restores it",
        window: true,
        timeout: MINUTE,
        body: views,
    },
    Spec {
        name: "detach",
        needs: KEYS,
        requires: &["picker"],
        proves: "Option+Shift+D ends Venus while the supervisor and its terminals live on, and \
                 `eon attach` shows them again",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: detach,
    },
    Spec {
        name: "crash",
        needs: GUI,
        requires: &["picker"],
        proves: "after Venus is killed, the supervisor and its terminals live on and \
                 `eon attach` shows them again",
        window: true,
        timeout: Duration::from_secs(2 * 60),
        body: crash,
    },
    Spec {
        name: "adoption",
        needs: GUI,
        requires: &["picker"],
        proves: "after the supervisor is killed, its Orbit hosts live on and a new `eon` \
                 adopts their terminals (EON-C15)",
        window: true,
        timeout: Duration::from_secs(3 * 60),
        body: adoption,
    },
    Spec {
        name: "stop",
        needs: GUI,
        requires: &["session"],
        proves: "`eon stop all` stops the generation and no process of the scenario is left",
        window: false,
        timeout: Duration::from_secs(2 * 60),
        body: stop,
    },
];

/// `mac-proof scenario --help`, from the step table.
pub fn help() -> String {
    let mut text = "\
mac-proof scenario: launch full Eon on this Mac's screen and drive it, mostly through the eon CLI

  --package PATH  the built Eon package (nix build .#packages.aarch64-darwin.default)
  --out DIR       a fresh directory for the evidence
  --timeout DUR   the run's time limit, as 30s, 90m or 12h (default 1h)
  --dry-run       run nothing, on any system, and plan every step

`run` ends with the same steps, on the package it built. Run it over SSH while the tool's user
is logged in to the Mac's screen. Accessibility, which lets the tool post key events, and
Screen Recording must be granted once, by a click in System Settings > Privacy & Security, to
the program that runs the tool (for SSH, the SSH server); nothing grants them silently. Without
Accessibility only launch, identity, retina and stop run, since the first tab's picker takes a
key. The key steps bring Eon's window to the front, then post US-layout key codes to Venus's
process: posting alone would leave the desktop's focus alone, but whether Venus takes Option and
Command chords while inactive is unproven on a Mac. A step whose need is missing, or that
requires such a step, is skipped and listed for Yu in summary.txt, never passed.

Steps, in order:
"
    .to_owned();
    for spec in STEPS {
        let needs: Vec<&str> = spec.needs.iter().map(|need| need.as_str()).collect();
        text += &format!(
            "  {:<9} needs {}; requires {}\n{}\n",
            spec.name,
            if needs.is_empty() {
                "nothing".to_owned()
            } else {
                needs.join(", ")
            },
            if spec.requires.is_empty() {
                "no step".to_owned()
            } else {
                spec.requires.join(", ")
            },
            wrap(spec.proves, 12)
        );
    }
    text += "
Evidence, beside the summaries: scenario/captures/NN-<label>.png, the dark and grey backdrops
and the light desktop before launch and one capture after each step that shows the window,
where Venus's window must differ from the desktop behind it, plus translucency-before,
translucency-light, translucency-dark, translucency-restored, kitty's, views' and retina's own
captures; scenario/eon.log with what `eon` printed; scenario/logs with Eon's own logs; and
retina's scenario/displays.json and scenario/system-profiler-displays.json.

Left for Yu: Command+C on a pointer selection, the Eon Bar's CPU and RAM chips (no CLI reads
them yet: eon-agent-parity-a9g0.12), VoiceOver, Kotoeri input and blur.";
    text
}

fn wrap(text: &str, indent: usize) -> String {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let line = lines.last_mut().expect("one line at least");
        if !line.is_empty() && indent + line.len() + 1 + word.len() > 96 {
            lines.push(word.to_owned());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
        .iter()
        .map(|line| format!("{:indent$}{line}", ""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs every scenario step, each also requiring `after`.
pub fn steps(runner: &mut Runner, scenario: &mut Scenario, after: &[&str]) -> io::Result<()> {
    for spec in STEPS {
        let requires: Vec<&str> = after.iter().chain(spec.requires).copied().collect();
        runner.step(
            scenario,
            spec.name,
            spec.timeout,
            &requires,
            false,
            |scenario, log, deadline| scenario.perform(spec, log, deadline),
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum Environment {
    Private,
    User,
}

/// The backdrop process, with stdin held open until it is killed and reaped.
struct Backdrop {
    child: Child,
}

impl Backdrop {
    fn new(rgb: [u8; 3], log: &mut Log) -> Result<Backdrop, Failure> {
        let failed = |error: io::Error| Failure::Check(format!("backdrop: {error}"));
        let mut command = Command::new(std::env::current_exe().map_err(failed)?);
        command
            .arg("backdrop")
            .args(rgb.map(|channel| channel.to_string()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log.stdio().map_err(failed)?);
        log.line(&format!("$ {command:?} &"));
        let mut backdrop = Backdrop {
            child: command.spawn().map_err(failed)?,
        };
        let stdout = backdrop.child.stdout.take().expect("piped stdout");
        let (sender, ready) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) if line == "shown" => {
                        let _ = sender.send(Ok(()));
                        break;
                    }
                    Ok(_) => (),
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        until(Instant::now() + Duration::from_secs(10), || {
            if let Some(status) = backdrop.child.try_wait().map_err(failed)? {
                return Err(Failure::Check(format!(
                    "backdrop exited before shown: {status}"
                )));
            }
            match ready.try_recv() {
                Ok(result) => result.map(|()| Some(())).map_err(failed),
                Err(_) => Ok(None),
            }
        })
        .map_err(|error| match error {
            Failure::Timeout => {
                Failure::Check("backdrop did not report shown within 10 s".to_owned())
            }
            other => other,
        })?;
        thread::sleep(Duration::from_secs(1));
        if let Some(status) = backdrop.child.try_wait().map_err(failed)? {
            return Err(Failure::Check(format!(
                "backdrop exited after shown: {status}"
            )));
        }
        Ok(backdrop)
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Scenario {
    environment: Environment,
    package: PathBuf,
    /// Where captures, Eon's output and its logs go.
    evidence: PathBuf,
    /// The private HOME, the launch directory and the driver's exchange.
    root: PathBuf,
    /// EON_RUNTIME_DIR, short enough for socket paths (103 bytes on macOS).
    runtime: PathBuf,
    gui: Availability,
    screen: Availability,
    keys: Availability,
    supervisor: Option<Child>,
    backdrop: Option<Backdrop>,
    /// Every process of the scenario seen so far, as pid and command.
    seen: BTreeMap<u32, String>,
    /// The desktop before launch.
    desktop: Option<Picture>,
    /// The dark backdrop before launch.
    dark: Option<Picture>,
    /// The grey reference before launch.
    grey: Option<Picture>,
    captures: u32,
    commands: u32,
    /// The pane that runs the driver.
    driver: String,
}

impl Scenario {
    pub(crate) fn new(package: PathBuf, evidence: PathBuf, environment: Environment) -> Scenario {
        let id = std::process::id();
        let unprobed = || Err("not probed".to_owned());
        Scenario {
            environment,
            package,
            evidence: match environment {
                Environment::Private => evidence.join("scenario"),
                Environment::User => evidence,
            },
            root: std::env::temp_dir().join(format!("mac-proof-scenario-{id}")),
            runtime: PathBuf::from(format!("/tmp/es-{id}")),
            gui: unprobed(),
            screen: unprobed(),
            keys: unprobed(),
            supervisor: None,
            backdrop: None,
            seen: BTreeMap::new(),
            desktop: None,
            dark: None,
            grey: None,
            captures: 0,
            commands: 0,
            driver: String::new(),
        }
    }

    fn available(&self, need: Need) -> &Availability {
        match need {
            Need::Gui => &self.gui,
            Need::Screen => &self.screen,
            Need::Keys => &self.keys,
        }
    }

    fn show_backdrop(&mut self, rgb: [u8; 3], log: &mut Log) -> Outcome {
        self.backdrop = None;
        self.backdrop = Some(Backdrop::new(rgb, log)?);
        Ok(())
    }

    fn perform(&mut self, spec: &Spec, log: &mut Log, deadline: Instant) -> Outcome {
        for need in spec.needs {
            if let Err(reason) = self.available(*need) {
                return Err(Failure::Unavailable(format!(
                    "needs {}: {reason}",
                    need.as_str()
                )));
            }
        }
        (spec.body)(self, log, deadline)?;
        if !spec.window || self.screen.is_err() {
            return Ok(());
        }
        self.shown(spec.name, log, deadline)
    }

    /// Captures the screen until Venus's window shows in it.
    fn shown(&mut self, label: &str, log: &mut Log, deadline: Instant) -> Outcome {
        // A window may still be drawing after its process starts.
        let shown = deadline.min(Instant::now() + Duration::from_secs(30));
        loop {
            let (picture, region) = self.window_capture(label, log, deadline)?;
            let desktop = self
                .desktop
                .as_ref()
                .map(|desktop| desktop.crop(region))
                .transpose()
                .map_err(Failure::Check)?;
            let changed = desktop.map_or(1.0, |desktop| picture.changed(&desktop));
            if !picture.blank() && changed >= SHOWN {
                return Ok(());
            }
            if Instant::now() >= shown {
                return Err(Failure::Check(format!(
                    "Venus's window is blank or differs from the desktop before launch in only {:.1}% of it",
                    changed * 100.0
                )));
            }
            // The next capture replaces this one.
            self.captures -= 1;
            thread::sleep(Duration::from_secs(1));
        }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub(crate) fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn exchange(&self) -> PathBuf {
        self.root.join("exchange")
    }

    fn eon_path(&self) -> PathBuf {
        self.package.join("bin/eon")
    }

    /// `eon` from the launch directory, with private Eon roots in either mode.
    pub(crate) fn eon(&self, arguments: &[&str]) -> Command {
        let home = self.home();
        let mut command = Command::new(self.eon_path());
        command.args(arguments).current_dir(self.work());
        if matches!(self.environment, Environment::Private) {
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("LANG", "en_US.UTF-8")
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("XDG_STATE_HOME", home.join(".local/state"))
                .env("XDG_DATA_HOME", home.join(".local/share"))
                .env("XDG_CACHE_HOME", home.join(".cache"))
                .env("TMPDIR", &self.runtime);
            for name in ["USER", "LOGNAME"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
        command
            // A terminal's targeting variables must not escape into the private workspace.
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .env_remove("EON_TAB")
            .env("EON_CONFIG_HOME", home.join(".config/eon"))
            .env("XDG_STATE_HOME", home.join(".local/state"))
            .env("EON_RUNTIME_DIR", &self.runtime);
        command
    }

    pub(crate) fn prepare(&mut self) -> Outcome {
        let failed = |error: io::Error| Failure::Check(format!("preparing the scenario: {error}"));
        if self.root.exists() {
            fs::remove_dir_all(&self.root).map_err(failed)?;
        }
        std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
            .create(&self.root)
            .map_err(failed)?;
        for directory in [
            self.home().join(".config/eon"),
            self.work(),
            self.exchange(),
            self.evidence.join("captures"),
        ] {
            fs::create_dir_all(directory).map_err(failed)?;
        }
        // A runtime root that exists may belong to another run.
        std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
            .create(&self.runtime)
            .map_err(failed)
    }

    /// Starts a supervisor in its own process group, its output in eon.log.
    pub(crate) fn start(&mut self, arguments: &[&str], log: &mut Log) -> Outcome {
        let output = File::options()
            .create(true)
            .append(true)
            .open(self.evidence.join("eon.log"))
            .and_then(|file| Ok((file.try_clone()?, file)))
            .map_err(|error| Failure::Check(format!("eon.log: {error}")))?;
        let mut command = self.eon(arguments);
        command
            .stdin(Stdio::null())
            .stdout(output.0)
            .stderr(output.1);
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        log.line(&format!("$ {command:?} &"));
        let child = command
            .spawn()
            .map_err(|error| Failure::Command(format!("cannot start eon: {error}")))?;
        log.line(&format!("mac-proof: supervisor pid {}", child.id()));
        self.supervisor = Some(child);
        Ok(())
    }

    fn supervisor_alive(&mut self) -> Outcome {
        let exited = match self.supervisor.as_mut() {
            Some(child) => child.try_wait().ok().flatten(),
            None => return Err(Failure::Check("no supervisor was started".to_owned())),
        };
        check(exited.is_none(), || {
            format!("the supervisor exited: {}", exited.expect("exited"))
        })
    }

    /// `eon workspace --json`, which answers once the supervisor serves requests.
    fn workspace(&self, log: &mut Log, deadline: Instant) -> Result<Value, Failure> {
        self.act(&["workspace"], log, deadline)
    }

    /// Runs `eon ARGUMENTS --json` and returns its reply.
    fn act(&self, arguments: &[&str], log: &mut Log, deadline: Instant) -> Result<Value, Failure> {
        let mut arguments = arguments.to_vec();
        let at = arguments
            .iter()
            .position(|argument| *argument == "--")
            .unwrap_or(arguments.len());
        arguments.insert(at, "--json");
        let output = succeed(log, &mut self.eon(&arguments), deadline, true)?;
        serde_json::from_slice(&output)
            .map_err(|error| Failure::Check(format!("eon {}: {error}", arguments.join(" "))))
    }

    /// The scenario's processes now, which also joins them to `seen`.
    fn processes(&mut self, log: &mut Log, deadline: Instant) -> Result<Vec<Process>, Failure> {
        let listing = listing(log, deadline)?;
        // With the slash, the root of a run whose pid extends this one's
        // does not match.
        let markers = [
            format!("{}/", self.runtime.display()),
            format!("{}/", self.root.display()),
        ];
        let roots: Vec<u32> = self.supervisor.iter().map(Child::id).collect();
        let ours = ours(&listing, &roots, &markers);
        for process in &ours {
            self.seen.insert(process.pid, process.command.clone());
        }
        Ok(ours)
    }

    /// Venus under the current supervisor.
    pub(crate) fn venus(
        &mut self,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<Option<u32>, Failure> {
        let supervisor = self.supervisor.as_ref().map_or(0, Child::id);
        let processes = self.processes(log, deadline)?;
        let under = ours(&processes, &[supervisor], &[]);
        Ok(under
            .iter()
            .find(|process| process.name().contains("yazelix-venus"))
            .map(|process| process.pid))
    }

    /// Venus under the supervisor once its window is on the screen, which
    /// may be a while after its process starts.
    pub(crate) fn wait_for_venus(
        &mut self,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<u32, Failure> {
        log.line("mac-proof: waiting for Venus's window under the supervisor");
        until(deadline, || match self.venus(log, deadline)? {
            Some(venus) if mac::windows(venus).is_ok_and(|(windows, _)| !windows.is_empty()) => {
                Ok(Some(venus))
            }
            _ => Ok(None),
        })
    }

    pub(crate) fn wait_for_workspace(
        &mut self,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<Value, Failure> {
        log.line("mac-proof: waiting for eon workspace --json to answer");
        until(deadline, || {
            self.supervisor_alive()?;
            let mut command = self.eon(&["workspace", "--json"]);
            let (status, output) = execute(log, &mut command, deadline, true, true)?;
            Ok(status
                .success()
                .then(|| serde_json::from_slice(&output).ok())
                .flatten())
        })
    }

    /// `screencapture` of the main display into the evidence, read back small.
    fn capture(
        &mut self,
        label: &str,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<Picture, Failure> {
        self.captures += 1;
        let png = self
            .evidence
            .join(format!("captures/{:02}-{label}.png", self.captures));
        let bmp = self.root.join("capture.bmp");
        succeed(
            log,
            Command::new("screencapture")
                .args(["-x", "-t", "png"])
                .arg(&png),
            deadline,
            false,
        )?;
        succeed(
            log,
            Command::new("sips")
                .args(["-s", "format", "bmp", "-Z", "640"])
                .arg(&png)
                .arg("--out")
                .arg(&bmp),
            deadline,
            false,
        )?;
        let bytes = fs::read(&bmp)
            .map_err(|error| Failure::Check(format!("{}: {error}", bmp.display())))?;
        let _ = fs::remove_file(&bmp);
        Picture::from_bmp(&bytes)
            .map_err(|error| Failure::Check(format!("{}: {error}", png.display())))
    }

    /// A capture, and the part of it that Venus's window covers.
    fn window_capture(
        &mut self,
        label: &str,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<(Picture, Region), Failure> {
        let venus = self
            .venus(log, deadline)?
            .ok_or_else(|| Failure::Check("no Venus under the supervisor".to_owned()))?;
        let region = window(venus).map_err(Failure::Check)?;
        log.line(&format!("mac-proof: Venus's window at {region:?}"));
        let picture = self.capture(label, log, deadline)?;
        let window = picture.crop(region).map_err(Failure::Check)?;
        Ok((window, region))
    }

    /// Brings Venus to the front and presses `keys` there.
    fn press(&mut self, keys: &[Key], log: &mut Log, deadline: Instant) -> Outcome {
        let venus = self.venus(log, deadline)?.ok_or_else(|| {
            Failure::Check("no Venus under the supervisor to send keys to".to_owned())
        })?;
        let keys: Vec<(u16, u64)> = keys.iter().map(|key| (key.code, key.flags)).collect();
        log.line(&format!("mac-proof: keys {keys:?} to Venus {venus}"));
        mac::press(venus, &keys).map_err(Failure::Check)
    }

    /// Runs `script` with /bin/sh in the driver's pane and returns its output.
    fn inside(
        &mut self,
        script: &str,
        log: &mut Log,
        deadline: Instant,
    ) -> Result<String, Failure> {
        self.commands += 1;
        log.line(&format!(
            "mac-proof: driver command {}: {script}",
            self.commands
        ));
        let (status, output) = exchange(&self.exchange(), self.commands, script, deadline)?;
        log.line(&output);
        check(status == "0", || {
            format!(
                "driver command {} exited {status}: {}",
                self.commands,
                output.trim()
            )
        })?;
        Ok(output)
    }

    /// Shows the driver's pane, which `eon focus` brings to the active tab.
    fn show_driver(&self, log: &mut Log, deadline: Instant) -> Outcome {
        check(!self.driver.is_empty(), || {
            "the driver's pane is unknown".to_owned()
        })?;
        self.act(&["focus", &self.driver], log, deadline).map(drop)
    }

    /// Every terminal of the workspace's panes; a terminal that ends takes
    /// its pane with it.
    fn terminals(&self, log: &mut Log, deadline: Instant) -> Result<BTreeSet<String>, Failure> {
        let workspace = self.workspace(log, deadline)?;
        Ok(panes(&workspace)
            .into_iter()
            .map(|pane| pane.session)
            .collect())
    }

    /// Ends `venus` by `end`, then checks that the terminals outlived it and
    /// that `eon attach` shows them again. Venus's end may add one: it
    /// cancels the pending picker of the tab that `panes` created, which
    /// starts that tab's terminal (EON-C18).
    fn restart_venus(
        &mut self,
        venus: u32,
        end: impl FnOnce(&mut Scenario, &mut Log) -> Outcome,
        log: &mut Log,
        deadline: Instant,
    ) -> Outcome {
        let before = self.terminals(log, deadline)?;
        end(self, log)?;
        log.line(&format!("mac-proof: waiting for Venus {venus} to end"));
        until(deadline, || {
            Ok((self.venus(log, deadline)? != Some(venus)).then_some(()))
        })?;
        thread::sleep(Duration::from_secs(2));
        self.supervisor_alive()?;
        let after = self.terminals(log, deadline)?;
        check(before.is_subset(&after), || {
            format!("terminals {before:?} became {after:?} without Venus")
        })?;
        let again = self.reattach(log, deadline)?;
        check(again != venus, || "Venus did not restart".to_owned())?;
        let shown = self.terminals(log, deadline)?;
        check(shown == after, || {
            format!("terminals {after:?} came back as {shown:?}")
        })
    }

    pub(crate) fn reattach(&mut self, log: &mut Log, deadline: Instant) -> Result<u32, Failure> {
        succeed(log, &mut self.eon(&["attach"]), deadline, false)?;
        self.wait_for_venus(log, deadline)
    }

    /// The scenario's processes still alive: those it finds now, and those
    /// seen before, which as orphans may no longer descend from a supervisor.
    fn survivors(&mut self, log: &mut Log, deadline: Instant) -> Result<Vec<Process>, Failure> {
        // Joins those found now to `seen`.
        self.processes(log, deadline)?;
        Ok(listing(log, deadline)?
            .into_iter()
            .filter(|process| self.seen.get(&process.pid) == Some(&process.command))
            .collect())
    }

    /// Kills what is left of the scenario by exact pid and removes its
    /// directories; returns what was still alive, or why it could not tell.
    pub(crate) fn clean(&mut self, log: &mut Log) -> Vec<String> {
        self.backdrop = None;
        let logs = self.home().join(".local/state/eon/logs");
        let copied = Command::new("cp")
            .arg("-R")
            .arg(&logs)
            .arg(self.evidence.join("logs"))
            .status();
        log.line(&format!(
            "mac-proof: copying {} into the evidence: {copied:?}",
            logs.display()
        ));
        let mut left = Vec::new();
        match self.survivors(log, Instant::now() + MINUTE) {
            Ok(processes) => {
                for process in processes {
                    let _ = Command::new("kill")
                        .args(["-KILL", &process.pid.to_string()])
                        .status();
                    left.push(format!("{} {}", process.pid, process.command));
                }
            }
            Err(_) => left.push("unknown: listing processes failed".to_owned()),
        }
        if let Some(mut child) = self.supervisor.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.runtime);
        let _ = fs::remove_dir_all(&self.root);
        left
    }
}

impl Drop for Scenario {
    /// Whatever a failed or skipped `stop` left behind.
    fn drop(&mut self) {
        if (self.supervisor.is_some() || self.root.exists())
            && let Ok(file) = File::options()
                .create(true)
                .append(true)
                .open(self.evidence.join("eon.log"))
        {
            let mut log = Log::new(file);
            let left = self.clean(&mut log);
            log.line(&format!(
                "mac-proof: cleanup after the steps killed {left:?}"
            ));
        }
    }
}

/// Waits until `probe` yields a value, failing at `deadline`.
fn until<T>(
    deadline: Instant,
    mut probe: impl FnMut() -> Result<Option<T>, Failure>,
) -> Result<T, Failure> {
    loop {
        if let Some(value) = probe()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(Failure::Timeout);
        }
        thread::sleep(PAUSE);
    }
}

/// A key as a macOS virtual key code (US layout) and CGEvent modifier flags.
#[derive(Clone, Copy)]
struct Key {
    code: u16,
    flags: u64,
}

const SHIFT: u64 = 0x2_0000;
const OPTION: u64 = 0x8_0000;
const COMMAND: u64 = 0x10_0000;
const ESCAPE: Key = Key { code: 53, flags: 0 };
const RETURN: Key = Key { code: 36, flags: 0 };
const FILES: Key = Key {
    code: 16,
    flags: OPTION | SHIFT,
};
const ZOOM: Key = Key {
    code: 6,
    flags: OPTION | SHIFT,
};
const DETACH: Key = Key {
    code: 2,
    flags: OPTION | SHIFT,
};
const SHORTCUTS: Key = Key {
    code: 44,
    flags: OPTION,
};
const PASTE: Key = Key {
    code: 9,
    flags: COMMAND,
};

/// Where the largest on-screen window of process `venus` lies on the main
/// display.
fn window(venus: u32) -> Result<Region, String> {
    let (windows, display) = mac::windows(venus)?;
    let bounds: Vec<_> = windows.into_iter().map(|window| window.bounds).collect();
    region(&bounds, display)
}

/// Runs each command the harness drops into the exchange directory ($1), in
/// turn, from inside an Eon terminal. `ready` holds the directory it started
/// in; a command's output goes to N.out and its exit status to N.status,
/// and fd 3 is the terminal itself.
const DRIVER: &str = r#"start=$(pwd)
cd "$1" || exit 1
exec 3>&1
printf '%s\n' "$start" > ready.tmp && mv ready.tmp ready
n=1
while [ ! -e stop ]; do
  if [ -e "$n.cmd" ]; then
    /bin/sh "$n.cmd" > "$n.out" 2>&1
    echo "$?" > "$n.tmp"
    mv "$n.tmp" "$n.status"
    n=$((n + 1))
  else
    sleep 0.2
  fi
done
"#;

/// Hands command `n` to the driver in `directory` and waits for its exit
/// status and output.
fn exchange(
    directory: &Path,
    n: u32,
    script: &str,
    deadline: Instant,
) -> Result<(String, String), Failure> {
    let written = directory.join(format!("{n}.tmp-cmd"));
    fs::write(&written, script)
        .and_then(|()| fs::rename(&written, directory.join(format!("{n}.cmd"))))
        .map_err(|error| Failure::Check(format!("driver command {n}: {error}")))?;
    let status = until(deadline, || {
        Ok(fs::read_to_string(directory.join(format!("{n}.status"))).ok())
    })?;
    let output = fs::read_to_string(directory.join(format!("{n}.out"))).unwrap_or_default();
    Ok((status.trim().to_owned(), output))
}

/// Quotes `text` as one /bin/sh word.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    parent: u32,
    pub command: String,
}

impl Process {
    /// The executable's file name.
    pub fn name(&self) -> &str {
        let program = self.command.split_whitespace().next().unwrap_or("");
        program.rsplit('/').next().unwrap_or(program)
    }
}

/// This user's processes.
pub fn listing(log: &mut Log, deadline: Instant) -> Result<Vec<Process>, Failure> {
    let user = succeed(log, Command::new("id").arg("-u"), deadline, true)?;
    let user = String::from_utf8_lossy(&user).trim().to_owned();
    // Every process, with no column cut short; BSD ps -U would leave out
    // those without a controlling terminal.
    let listing = succeed(
        log,
        Command::new("ps").args(["-A", "-ww", "-o", "uid=,pid=,ppid=,command="]),
        deadline,
        true,
    )?;
    Ok(parse_processes(&String::from_utf8_lossy(&listing), &user))
}

/// `ps -o uid=,pid=,ppid=,command=` lines of user id `user`.
fn parse_processes(listing: &str, user: &str) -> Vec<Process> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != user {
                return None;
            }
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some(Process {
                pid,
                parent,
                command: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

/// The processes in `roots` or whose command names a `marker`, and all
/// their descendants.
fn ours(listing: &[Process], roots: &[u32], markers: &[String]) -> Vec<Process> {
    let mut chosen: BTreeSet<u32> = listing
        .iter()
        .filter(|process| {
            roots.contains(&process.pid)
                || markers
                    .iter()
                    .any(|marker| process.command.contains(marker.as_str()))
        })
        .map(|process| process.pid)
        .collect();
    loop {
        let before = chosen.len();
        for process in listing {
            if chosen.contains(&process.parent) {
                chosen.insert(process.pid);
            }
        }
        if chosen.len() == before {
            break;
        }
    }
    listing
        .iter()
        .filter(|process| chosen.contains(&process.pid))
        .cloned()
        .collect()
}

#[derive(Debug, PartialEq)]
struct Pane {
    id: String,
    tab: String,
    session: String,
}

/// Every pane of `eon workspace --json`, tab by tab.
fn panes(workspace: &Value) -> Vec<Pane> {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    workspace["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|tab| {
            tab["panes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |pane| Pane {
                    id: text(&pane["id"]),
                    tab: text(&tab["id"]),
                    session: text(&pane["session"]),
                })
        })
        .collect()
}

/// The active tab and its selected pane.
fn selection(workspace: &Value) -> (String, String) {
    let active = workspace["active_tab"].as_str().unwrap_or_default();
    let tab = workspace["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|tab| tab["id"] == active);
    let pane = tab.and_then(|tab| tab["selected_pane"].as_str());
    (active.to_owned(), pane.unwrap_or_default().to_owned())
}

/// The active tab's shown popup's entry, such as `files`.
fn shown_popup(workspace: &Value) -> Option<String> {
    let active = &workspace["active_tab"];
    let tab = workspace["tabs"]
        .as_array()?
        .iter()
        .find(|tab| &tab["id"] == active)?;
    let shown = tab["selected_popup"].as_str()?;
    let popup = tab["popups"]
        .as_array()?
        .iter()
        .find(|popup| popup["id"] == shown)?;
    popup["entry"].as_str().map(str::to_owned)
}

/// Where the largest of `windows` (x, y, width and height) lies, in shares
/// of a `display` of that width and height.
fn region(windows: &[[f64; 4]], [across, down]: [f64; 2]) -> Result<Region, String> {
    let largest = windows
        .iter()
        .max_by(|a, b| (a[2] * a[3]).total_cmp(&(b[2] * b[3])));
    match largest {
        Some(&[x, y, width, height])
            if width > 0.0 && height > 0.0 && across > 0.0 && down > 0.0 =>
        {
            Ok(Region {
                x: x / across,
                y: y / down,
                width: width / across,
                height: height / down,
            })
        }
        _ => Err(format!(
            "no on-screen window with an area among {windows:?} on a {across}x{down} display"
        )),
    }
}

/// Checks that `eon versions` names `target` for Eon and each component.
fn versions_name(report: &str, target: &str) -> Result<(), String> {
    let product = format!("eon {target}");
    let lines: Vec<&str> = report.lines().skip_while(|line| *line != product).collect();
    if lines.len() < 2 {
        return Err(format!("no `{product}` line followed by components"));
    }
    match lines
        .iter()
        .find(|line| !line.ends_with(&format!(" {target}")))
    {
        Some(line) => Err(format!("`{line}` names another target than {target}")),
        None => Ok(()),
    }
}

/// A grant's availability.
fn grant(granted: bool, need: Need) -> Availability {
    match granted {
        true => Ok(()),
        false => Err(format!(
            "not granted; grant {} once to the program that runs the tool (over SSH, sshd-keygen-wrapper) in System Settings > Privacy & Security, which takes a click on the Mac",
            need.as_str()
        )),
    }
}

/// Whether `user` holds the Mac's screen, from the owner of /dev/console.
fn console(owner: &str, user: &str) -> Availability {
    match owner {
        _ if owner == user => Ok(()),
        "root" | "" => Err(format!(
            "no one is logged in to the Mac's screen; log {user} in, or, with FileVault off, turn on automatic login with `sudo sysadminctl -autologin set -userName {user} -password -` and restart"
        )),
        _ => Err(format!("{owner} holds the Mac's screen, not {user}")),
    }
}

fn session(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).trim().to_owned();
    let user = text(succeed(log, Command::new("id").arg("-un"), deadline, true)?);
    let owner = text(succeed(
        log,
        Command::new("stat").args(["-f", "%Su", "/dev/console"]),
        deadline,
        true,
    )?);
    scenario.gui = console(&owner, &user);
    // Every other step needs the GUI login, so without it the grants do not
    // matter.
    if scenario.gui.is_ok() {
        let (screen, keys) = mac::grants().map_err(Failure::Check)?;
        scenario.screen = grant(screen, Need::Screen);
        scenario.keys = grant(keys, Need::Keys);
    }
    for need in [Need::Gui, Need::Screen, Need::Keys] {
        log.line(&format!(
            "mac-proof: {}: {}",
            need.as_str(),
            match scenario.available(need) {
                Ok(()) => "available",
                Err(reason) => reason,
            }
        ));
    }
    Ok(())
}

fn launch(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let failed = |error: io::Error| Failure::Check(format!("preparing the scenario: {error}"));
    scenario.prepare()?;
    let config = scenario.home().join(".config/eon");
    // Anima would delay the picker by three seconds.
    fs::write(
        config.join("config.toml"),
        format!("[anima]\nenabled = false\n\n[terminal]\nbackground_opacity = {OPACITY}\n"),
    )
    .map_err(failed)?;
    let driver = scenario.root.join("driver.sh");
    fs::write(&driver, DRIVER).map_err(failed)?;
    if scenario.screen.is_ok() {
        scenario.show_backdrop(DARK, log)?;
        scenario.dark = Some(scenario.capture("backdrop-dark", log, deadline)?);
        scenario.show_backdrop(GREY, log)?;
        scenario.grey = Some(scenario.capture("backdrop-grey", log, deadline)?);
        scenario.show_backdrop(LIGHT, log)?;
        scenario.desktop = Some(scenario.capture("desktop", log, deadline)?);
    }
    let exchange = scenario.exchange();
    scenario.start(
        &[
            "run",
            "--",
            "/bin/sh",
            &driver.to_string_lossy(),
            &exchange.to_string_lossy(),
        ],
        log,
    )?;
    scenario.wait_for_workspace(log, deadline)?;
    scenario.wait_for_venus(log, deadline).map(drop)
}

fn translucency(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    let venus = scenario
        .venus(log, deadline)?
        .ok_or_else(|| Failure::Check("no Venus under the supervisor".to_owned()))?;
    let region = window(venus).map_err(Failure::Check)?;
    log.line(&format!("mac-proof: Venus's window at {region:?}"));
    let crop = |picture: &Option<Picture>| {
        picture
            .as_ref()
            .ok_or_else(|| Failure::Check("no backdrop capture before launch".to_owned()))?
            .crop(region)
            .map_err(Failure::Check)
    };
    let (light, dark, grey) = (
        crop(&scenario.desktop)?,
        crop(&scenario.dark)?,
        crop(&scenario.grey)?,
    );
    if !light.blank() || !dark.blank() || !grey.blank() {
        return Err(Failure::Unavailable(
            "something besides the backdrop lies behind Venus's window; left for Yu".to_owned(),
        ));
    }
    let (b_light, b_dark, c) = (light.dominant().0, dark.dominant().0, grey.dominant().0);
    let expected_light = blend(c, b_light, OPACITY);
    let expected_dark = blend(c, b_dark, OPACITY);
    let references = format!(
        "B_light {b_light:?}, B_dark {b_dark:?}, C' {c:?}, background_opacity {OPACITY}, \
         expected light {expected_light:?}, expected dark {expected_dark:?}"
    );
    let (p0, share) = scenario
        .capture("translucency-before", log, deadline)?
        .crop(region)
        .map_err(Failure::Check)?
        .dominant();
    log.line(&format!(
        "mac-proof: background before OSC 11: P0 {p0:?} (share {share})"
    ));
    let observed = (|| {
        let [r, g, b] = GREY;
        scenario.inside(
            &format!(r"printf '\033]11;rgb:{r:02x}/{g:02x}/{b:02x}\033\\' >&3"),
            log,
            deadline,
        )?;
        let ready = Instant::now() + Duration::from_secs(10);
        let (p_light, light_share) = loop {
            let picture = scenario
                .capture("translucency-light", log, deadline)?
                .crop(region)
                .map_err(Failure::Check)?;
            let observed = picture.dominant();
            if check_blend(observed.0, expected_light).is_ok() || Instant::now() >= ready {
                break observed;
            }
            // The next capture replaces this one.
            scenario.captures -= 1;
            thread::sleep(PAUSE);
        };
        scenario.show_backdrop(DARK, log)?;
        let p_dark = scenario
            .capture("translucency-dark", log, deadline)?
            .crop(region)
            .map_err(Failure::Check)?;
        let (p_dark, dark_share) = p_dark.dominant();
        let colours = format!(
            "P_light {p_light:?} (share {light_share}), P_dark {p_dark:?} (share {dark_share}), \
             {references}"
        );
        let shares = backdrop_shares(p_light, p_dark, b_light, b_dark)
            .map_err(|error| Failure::Check(format!("{colours}; {error}")))?;
        let background: [f64; 3] = std::array::from_fn(|channel| {
            (f64::from(p_dark[channel]) - shares[channel] * f64::from(b_dark[channel]))
                / (1.0 - shares[channel])
        });
        let numbers =
            format!("{colours}; backdrop shares {shares:?}, implied background C {background:?}");
        log.line(&format!("mac-proof: {numbers}"));
        check(light_share >= 0.25 && dark_share >= 0.25, || {
            format!(
                "{numbers}; dominant shares must each be at least 0.25; \
                 straight-vs-premultiplied or linear-light blending may show this way"
            )
        })?;
        check_blend(p_light, expected_light).map_err(|error| {
            Failure::Check(format!(
                "grey background did not match within 10 s; {numbers}; {error}"
            ))
        })?;
        check_blend(p_dark, expected_dark)
            .map_err(|error| Failure::Check(format!("{numbers}; {error}")))?;
        check_shares(shares, 1.0 - OPACITY)
            .map_err(|error| Failure::Check(format!("{numbers}; {error}")))
    })();
    // Restore both even after a failure, with time for the driver to answer.
    let mut restored = scenario
        .inside(
            r"printf '\033]111\033\\' >&3",
            log,
            Instant::now() + Duration::from_secs(10),
        )
        .map(drop);
    let backdrop = scenario.show_backdrop(LIGHT, log);
    if restored.is_ok() && backdrop.is_ok() {
        let now = Instant::now();
        let deadline = if now >= deadline {
            now + Duration::from_secs(30)
        } else {
            deadline
        };
        restored = (|| {
            let expected = p0.map(f64::from);
            let ready = Instant::now() + Duration::from_secs(10);
            let (colour, share) = loop {
                let picture = scenario
                    .capture("translucency-restored", log, deadline)?
                    .crop(region)
                    .map_err(Failure::Check)?;
                let observed = picture.dominant();
                if check_blend(observed.0, expected).is_ok() || Instant::now() >= ready {
                    break observed;
                }
                // The next capture replaces this one.
                scenario.captures -= 1;
                thread::sleep(PAUSE);
            };
            log.line(&format!(
                "mac-proof: background after OSC 111: P0 {p0:?}, dominant {colour:?} (share {share})"
            ));
            check_blend(colour, expected).map_err(|error| {
                Failure::Check(format!(
                    "OSC 111 did not restore the background within 10 s: \
                     P0 {p0:?}, last dominant {colour:?}; {error}"
                ))
            })
        })();
    }
    observed.and(restored).and(backdrop)
}

fn identity(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let report = succeed(log, &mut scenario.eon(&["versions"]), deadline, true)?;
    versions_name(&String::from_utf8_lossy(&report), "aarch64-darwin").map_err(Failure::Check)?;
    let path = succeed(log, &mut scenario.eon(&["config-path"]), deadline, true)?;
    let path = String::from_utf8_lossy(&path).trim().to_owned();
    let expected = scenario.home().join(".config/eon");
    // macOS's temporary directories sit behind the /var and /tmp links.
    check(
        fs::canonicalize(&path).ok() == fs::canonicalize(&expected).ok(),
        || format!("eon config-path printed {path}, not {}", expected.display()),
    )
}

fn retina(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let failed = |error: io::Error| Failure::Check(format!("recording the displays: {error}"));
    let displays = mac::displays().map_err(Failure::Check)?;
    for display in &displays {
        log.line(&format!(
            "mac-proof: display {}{}{} runs {}; {} modes, HiDPI ones among them: {:?}",
            display.id,
            if display.main { ", the main one," } else { "" },
            if display.builtin { " (built in)" } else { "" },
            display.current,
            display.modes.len(),
            display
                .modes
                .iter()
                .filter(|mode| mode.scale() >= 2.0)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        ));
    }
    fs::write(
        scenario.evidence.join("displays.json"),
        retina::report(&displays).to_string(),
    )
    .map_err(failed)?;
    let profile = retina::system_profiler(log, &["SPDisplaysDataType"], deadline)?;
    fs::write(
        scenario.evidence.join("system-profiler-displays.json"),
        profile.to_string(),
    )
    .map_err(failed)?;
    match retina::profiled(&profile) {
        Ok(profiled) => {
            for display in profiled {
                log.line(&format!(
                    "mac-proof: system_profiler: {}{} has {:?} pixels as {:?} points, scale {}",
                    display.name,
                    if display.main {
                        ", the main display,"
                    } else {
                        ""
                    },
                    display.pixels,
                    display.points,
                    display.scale()
                ));
            }
        }
        Err(error) => log.line(&format!(
            "mac-proof: system_profiler's report does not parse: {error}"
        )),
    }
    // The displays above stay recorded for Yu when the step cannot go on.
    if let Err(reason) = &scenario.screen {
        return Err(Failure::Unavailable(format!(
            "needs {}: {reason}",
            Need::Screen.as_str()
        )));
    }
    let main = displays
        .iter()
        .find(|display| display.main)
        .ok_or_else(|| Failure::Check("no main display among the active ones".to_owned()))?;
    let output = scenario.evidence.join("eon.log");
    let scales = || {
        retina::venus_scales(&String::from_utf8_lossy(
            &fs::read(&output).unwrap_or_default(),
        ))
    };
    // Venus's scale once the last it logged after the first `seen` is
    // `wanted`, as `fits` tells.
    let logged = |seen: usize, wanted: &str, fits: &dyn Fn(f64) -> bool, log: &mut Log| {
        let settled = deadline.min(Instant::now() + Duration::from_secs(30));
        until(settled, || {
            Ok(scales()
                .get(seen..)
                .and_then(|new| new.last().copied())
                .filter(|scale| fits(*scale)))
        })
        .map_err(|_| {
            Failure::Check(format!(
                "Venus's window scale factor is not {wanted} after 30 seconds; eon.log has {:?}, the first {seen} from before",
                scales()
            ))
        })
        .inspect(|scale| {
            log.line(&format!(
                "mac-proof: Venus's window scale factor is {scale}"
            ))
        })
    };
    let mut switched = None;
    let seen = if main.current.scale() >= 2.0 {
        0
    } else {
        let mode = retina::hidpi(&main.current, &main.modes).ok_or_else(|| {
            Failure::Unavailable(format!(
                "no HiDPI display: the main display runs {} and offers no HiDPI mode",
                main.current
            ))
        })?;
        let seen = scales().len();
        log.line(&format!("mac-proof: switching the main display to {mode}"));
        switched = Some(mac::switch(main.id, mode).map_err(|error| {
            Failure::Unavailable(format!("cannot switch the main display to {mode}: {error}"))
        })?);
        seen
    };
    let scale = logged(seen, "2 or more", &|scale| scale >= 2.0, log)?;
    scenario.shown("retina", log, deadline)?;
    log.line(&format!(
        "mac-proof: Retina proved remotely: Venus's window at scale {scale} on the main display, in a capture"
    ));
    if let Some(mut switched) = switched {
        let seen = scales().len();
        log.line(&format!(
            "mac-proof: switching the main display back to {}",
            main.current
        ));
        switched.restore().map_err(Failure::Check)?;
        let original = main.current.scale();
        logged(
            seen,
            &format!("back at {original}"),
            &|scale| scale == original,
            log,
        )?;
    }
    Ok(())
}

fn picker(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let ready = scenario.exchange().join("ready");
    // The picker may not take keys yet when the window first shows.
    while !ready.is_file() {
        scenario.press(&[ESCAPE], log, deadline)?;
        log.line("mac-proof: waiting for the driver to start");
        let wait = deadline.min(Instant::now() + Duration::from_secs(10));
        match until(wait, || Ok(ready.is_file().then_some(()))) {
            Err(Failure::Timeout) if Instant::now() < deadline => {}
            other => other?,
        }
    }
    let started = fs::read_to_string(&ready).unwrap_or_default();
    let work = fs::canonicalize(scenario.work())
        .map_err(|error| Failure::Check(format!("the launch directory: {error}")))?;
    check(
        fs::canonicalize(started.trim()).is_ok_and(|started| started == work),
        || {
            format!(
                "the driver started in {}, not the launch directory {}",
                started.trim(),
                work.display()
            )
        },
    )?;
    let workspace = scenario.workspace(log, deadline)?;
    let panes = panes(&workspace);
    let [pane] = &panes[..] else {
        return Err(Failure::Check(format!(
            "want the driver's pane alone, found {panes:?}"
        )));
    };
    scenario.driver = pane.id.clone();
    Ok(())
}

fn output(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let output = scenario.inside(
        r#"printf '%s\n' "TERM=$TERM" "EON_TERMINAL=$EON_TERMINAL" "$(/usr/bin/tput longname)" "$(/bin/stty size <&3)""#,
        log,
        deadline,
    )?;
    let lines: Vec<&str> = output.lines().collect();
    // `stty size` prints rows and columns.
    let size: Vec<u32> = lines
        .get(3)
        .map(|line| line.split(' ').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();
    check(
        lines.first() == Some(&"TERM=eon")
            && lines
                .get(1)
                .is_some_and(|line| line.len() > "EON_TERMINAL=".len())
            && lines.get(2) == Some(&"Eon terminal")
            && matches!(size[..], [rows, columns] if rows > 0 && columns > 0),
        || format!("the pane's command printed {lines:?}"),
    )
}

fn create(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let before = scenario.workspace(log, deadline)?;
    let count = |workspace: &Value, tab: &str| {
        panes(workspace)
            .iter()
            .filter(|pane| pane.tab == tab)
            .count()
    };
    let tab = selection(&before).0;
    let tabs = before["tabs"].as_array().map_or(0, Vec::len);
    scenario.act(&["pane", "create", "--no-focus"], log, deadline)?;
    let eon = quote(&scenario.eon_path().to_string_lossy());
    scenario.inside(
        &format!("{eon} pane create --json --no-focus -- /bin/sleep 3600"),
        log,
        deadline,
    )?;
    scenario.act(&["tab", "create"], log, deadline)?;
    let after = scenario.workspace(log, deadline)?;
    let (panes_before, panes_after) = (count(&before, &tab), count(&after, &tab));
    let tabs_after = after["tabs"].as_array().map_or(0, Vec::len);
    check(
        panes_after == panes_before + 2 && tabs_after == tabs + 1,
        || {
            format!(
                "tab {tab} went from {panes_before} to {panes_after} panes and the workspace from {tabs} to {tabs_after} tabs"
            )
        },
    )
}

fn focus(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    let workspace = scenario.workspace(log, deadline)?;
    let shown = selection(&workspace);
    let tab = panes(&workspace)
        .into_iter()
        .find(|pane| pane.id == scenario.driver)
        .map(|pane| pane.tab)
        .unwrap_or_default();
    check(shown == (tab.clone(), scenario.driver.clone()), || {
        format!("eon focus {} left {shown:?} selected", scenario.driver)
    })?;
    scenario.act(&["focus", "down"], log, deadline)?;
    let down = selection(&scenario.workspace(log, deadline)?);
    scenario.act(&["focus", "up"], log, deadline)?;
    let up = selection(&scenario.workspace(log, deadline)?);
    check(
        down.0 == tab && down.1 != scenario.driver && up == shown,
        || format!("down selected {down:?} and up {up:?}, from {shown:?}"),
    )
}

fn edit(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    let file = scenario.work().join("scenario.txt");
    fs::write(&file, "Opened by mac-proof's scenario.\n")
        .map_err(|error| Failure::Check(format!("{}: {error}", file.display())))?;
    let before: BTreeSet<String> = panes(&scenario.workspace(log, deadline)?)
        .into_iter()
        .map(|pane| pane.id)
        .collect();
    scenario.act(&["edit", "--", "scenario.txt"], log, deadline)?;
    // Helix that fails to start ends its terminal, and with it its pane,
    // within this.
    thread::sleep(Duration::from_secs(3));
    let added: Vec<Pane> = panes(&scenario.workspace(log, deadline)?)
        .into_iter()
        .filter(|pane| !before.contains(&pane.id))
        .collect();
    check(added.len() == 1, || {
        format!("eon edit added {added:?}, not one pane")
    })
}

fn kitty(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    thread::sleep(Duration::from_secs(1));
    let (before, _) = scenario.window_capture("kitty-before", log, deadline)?;
    scenario.inside(&format!("printf '{IMAGE}' >&3"), log, deadline)?;
    thread::sleep(Duration::from_secs(2));
    let (after, _) = scenario.window_capture("kitty-after", log, deadline)?;
    let (was, is) = (before.magenta(), after.magenta());
    check(is - was >= VIEW, || {
        format!(
            "magenta covers {:.2}% of Venus's window after the image and {:.2}% before",
            is * 100.0,
            was * 100.0
        )
    })
}

fn keys(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    scenario.press(&[FILES], log, deadline)?;
    log.line("mac-proof: waiting for the Files popup");
    until(deadline, || {
        Ok(
            (shown_popup(&scenario.workspace(log, deadline)?).as_deref() == Some("files"))
                .then_some(()),
        )
    })?;
    scenario.press(&[FILES], log, deadline)?;
    log.line("mac-proof: waiting for the Files popup to hide");
    until(deadline, || {
        Ok(shown_popup(&scenario.workspace(log, deadline)?)
            .is_none()
            .then_some(()))
    })?;

    // Command+V puts `pbcopy`'s text, which replaces the Mac's clipboard,
    // into a pane that reads one line.
    let token = format!("mac-proof paste {}", std::process::id());
    let copied = scenario.root.join("copied");
    let pasted = scenario.root.join("pasted");
    fs::write(&copied, &token)
        .map_err(|error| Failure::Check(format!("{}: {error}", copied.display())))?;
    succeed(
        log,
        Command::new("/bin/sh")
            .args(["-c", r#"/usr/bin/pbcopy < "$0""#])
            .arg(&copied),
        deadline,
        false,
    )?;
    // Renamed into place, so the harness never reads it half written.
    let reader = format!(
        r#"IFS= read -r line; printf %s "$line" > {0}.tmp && /bin/mv {0}.tmp {0}; exec /bin/sleep 3600"#,
        quote(&pasted.to_string_lossy())
    );
    let eon = quote(&scenario.eon_path().to_string_lossy());
    scenario.inside(
        &format!("{eon} pane create --json -- /bin/sh -c {}", quote(&reader)),
        log,
        deadline,
    )?;
    thread::sleep(Duration::from_secs(1));
    scenario.press(&[PASTE, RETURN], log, deadline)?;
    log.line("mac-proof: waiting for the pasted line");
    let line = until(deadline, || Ok(fs::read_to_string(&pasted).ok()))?;
    check(line == token, || {
        format!("the pane read {line:?}, not {token:?}")
    })
}

fn views(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    scenario.show_driver(log, deadline)?;
    let mut changes = Vec::new();
    for (name, key) in [("zoom", ZOOM), ("shortcuts", SHORTCUTS)] {
        let (before, region) = scenario.window_capture(&format!("{name}-before"), log, deadline)?;
        let mut toggle = |scenario: &mut Scenario, label: &str| {
            scenario.press(&[key], log, deadline)?;
            thread::sleep(Duration::from_secs(1));
            let picture = scenario.capture(label, log, deadline)?;
            picture.crop(region).map_err(Failure::Check)
        };
        let during = toggle(scenario, name)?;
        // The same key restores the view.
        let after = toggle(scenario, &format!("{name}-after"))?;
        changes.push((name, during.changed(&before), after.changed(&before)));
    }
    log.line(&format!(
        "mac-proof: share of Venus's window each key changed, then left changed: {changes:?}"
    ));
    // Something else on the screen would not also come and go with the key.
    check(
        changes
            .iter()
            .all(|(_, during, after)| *during >= VIEW && *after < VIEW),
        || format!("a key's view did not show and go in Venus's window: {changes:?}"),
    )
}

fn detach(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let venus = scenario.wait_for_venus(log, deadline)?;
    scenario.restart_venus(
        venus,
        |scenario, log| scenario.press(&[DETACH], log, deadline),
        log,
        deadline,
    )
}

fn crash(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let venus = scenario.wait_for_venus(log, deadline)?;
    scenario.restart_venus(
        venus,
        |_, log| {
            succeed(
                log,
                Command::new("kill").args(["-KILL", &venus.to_string()]),
                deadline,
                false,
            )
            .map(drop)
        },
        log,
        deadline,
    )
}

fn adoption(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    let before = scenario.terminals(log, deadline)?;
    let hosts: Vec<Process> = scenario
        .processes(log, deadline)?
        .into_iter()
        .filter(|process| process.name() == "yazelix-orbit")
        .collect();
    check(!hosts.is_empty(), || {
        "no Orbit host to outlive the supervisor".to_owned()
    })?;
    let mut old = scenario
        .supervisor
        .take()
        .ok_or_else(|| Failure::Check("no supervisor was started".to_owned()))?;
    log.line(&format!("mac-proof: killing the supervisor {}", old.id()));
    let _ = old.kill();
    let _ = old.wait();
    thread::sleep(Duration::from_secs(2));
    let alive = listing(log, deadline)?;
    // Reparented now, so only pid and command still identify them.
    let lost: Vec<&Process> = hosts
        .iter()
        .filter(|host| {
            !alive
                .iter()
                .any(|process| (process.pid, &process.command) == (host.pid, &host.command))
        })
        .collect();
    check(lost.is_empty(), || {
        format!("Orbit hosts ended with the supervisor: {lost:?}")
    })?;
    scenario.start(&[], log)?;
    scenario.wait_for_workspace(log, deadline)?;
    scenario.wait_for_venus(log, deadline)?;
    let adopted = scenario.terminals(log, deadline)?;
    let missing: Vec<&String> = before.difference(&adopted).collect();
    check(missing.is_empty(), || {
        format!("the new supervisor did not adopt terminals {missing:?}; it shows {adopted:?}")
    })
}

pub(crate) fn stop(scenario: &mut Scenario, log: &mut Log, deadline: Instant) -> Outcome {
    check(scenario.supervisor.is_some(), || {
        "no supervisor is running".to_owned()
    })?;
    scenario.processes(log, deadline)?;
    let reply = scenario.act(&["stop", "all"], log, deadline);
    log.line("mac-proof: waiting for the supervisor to exit");
    let exited = until(deadline, || {
        Ok(scenario
            .supervisor
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten()))
    });
    // Orbit hosts and their children may still be ending.
    let settled = deadline.min(Instant::now() + Duration::from_secs(15));
    let _ = until(settled, || {
        Ok(scenario.survivors(log, settled)?.is_empty().then_some(()))
    });
    let left = scenario.clean(log);
    let reply = reply?;
    let results = reply.as_array().cloned().unwrap_or_default();
    check(
        !results.is_empty() && results.iter().all(|result| result.get("stopped").is_some()),
        || format!("eon stop all replied {reply}"),
    )?;
    exited?;
    check(left.is_empty(), || {
        format!("left behind, now killed: {left:?}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_environment_keeps_home_and_tmpdir_but_isolates_eon_targeting() {
        let scenario = Scenario::new(
            PathBuf::from("/package"),
            PathBuf::from("/evidence"),
            Environment::User,
        );
        let command = scenario.eon(&["run"]);
        let environment: BTreeMap<_, _> = command.get_envs().collect();
        for name in ["EON_WINDOW_RUNTIME_DIR", "EON_TERMINAL", "EON_TAB"] {
            assert_eq!(environment.get(std::ffi::OsStr::new(name)), Some(&None));
        }
        for name in [
            "HOME",
            "TMPDIR",
            "PATH",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
        ] {
            assert!(!environment.contains_key(std::ffi::OsStr::new(name)));
        }
        for (name, path) in [
            ("EON_CONFIG_HOME", scenario.home().join(".config/eon")),
            ("XDG_STATE_HOME", scenario.home().join(".local/state")),
            ("EON_RUNTIME_DIR", scenario.runtime.clone()),
        ] {
            assert_eq!(
                environment.get(std::ffi::OsStr::new(name)),
                Some(&Some(path.as_os_str()))
            );
        }
        assert_eq!(command.get_current_dir(), Some(scenario.work().as_path()));
    }
    #[test]
    fn the_driver_runs_each_command_in_turn_and_reports_it() {
        let directory =
            std::env::temp_dir().join(format!("mac-proof-driver-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let script = directory.join("driver.sh");
        fs::write(&script, DRIVER).unwrap();
        let terminal = directory.join("terminal");
        let mut driver = Command::new("/bin/sh")
            .arg(&script)
            .arg(&directory)
            .current_dir("/")
            .stdout(File::create(&terminal).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + MINUTE;
        let first = exchange(
            &directory,
            1,
            "printf 'out\\n'; printf 'tty\\n' >&3",
            deadline,
        );
        assert_eq!(first.ok(), Some(("0".to_owned(), "out\n".to_owned())));
        let second = exchange(&directory, 2, "echo failing >&2; exit 7", deadline);
        assert_eq!(second.ok(), Some(("7".to_owned(), "failing\n".to_owned())));
        fs::write(directory.join("stop"), "").unwrap();
        assert!(driver.wait().unwrap().success());
        assert_eq!(fs::read_to_string(directory.join("ready")).unwrap(), "/\n");
        assert_eq!(fs::read_to_string(&terminal).unwrap(), "tty\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_scenario_finds_its_own_processes_and_reads_eon_s_replies() {
        let listing = parse_processes(
            "  501    10     1 /nix/store/x-eon/bin/.eon-wrapped run -- /bin/sh /t/root/driver.sh\n\
             501    11    10 /nix/store/x-venus/bin/.yazelix-venus-wrapped --admission 3\n\
             501    12     1 /nix/store/x-orbit/bin/yazelix-orbit serve /tmp/es-9/generations/g1/session-1.sock\n\
             501    13    12 /bin/sleep 3600\n\
             501    14     1 /usr/bin/ssh other\n\
             501    16     1 /nix/store/x-orbit/bin/yazelix-orbit serve /tmp/es-90/generations/g1/session-1.sock\n\
             0      15    12 /usr/sbin/root-owned /tmp/es-9\n\
             garbage\n",
            "501",
        );
        assert_eq!(listing.len(), 6);
        let pids = |processes: Vec<Process>| -> Vec<u32> {
            processes.iter().map(|process| process.pid).collect()
        };
        assert_eq!(
            pids(ours(&listing, &[10], &["/tmp/es-9/".to_owned()])),
            [10, 11, 12, 13]
        );
        assert_eq!(listing[1].name(), ".yazelix-venus-wrapped");
        assert_eq!(pids(ours(&listing, &[10], &[])), [10, 11]);

        let workspace = json!({
            "active_tab": "t2",
            "tabs": [
                {"id": "t1", "selected_pane": "p1", "selected_popup": null, "popups": [],
                 "panes": [{"id": "p1", "session": "session-1"}]},
                {"id": "t2", "selected_pane": "p3", "selected_popup": "u1",
                 "popups": [{"id": "u1", "entry": "files"}],
                 "panes": [{"id": "p2", "session": "session-2"},
                           {"id": "p3", "session": "session-3"}]}
            ]
        });
        let found = panes(&workspace);
        assert_eq!(found.len(), 3);
        assert_eq!(
            found[2],
            Pane {
                id: "p3".to_owned(),
                tab: "t2".to_owned(),
                session: "session-3".to_owned(),
            }
        );
        assert_eq!(selection(&workspace), ("t2".to_owned(), "p3".to_owned()));
        assert_eq!(shown_popup(&workspace).as_deref(), Some("files"));
        assert_eq!(
            shown_popup(&json!({"active_tab": "t1", "tabs": workspace["tabs"]})),
            None
        );

        let report =
            "eon 0.9.0 g1-abc\neonw 23\neon aarch64-darwin\norbit 1 workspace aarch64-darwin";
        assert_eq!(versions_name(report, "aarch64-darwin"), Ok(()));
        assert!(
            versions_name(
                &report.replace("orbit 1 workspace aarch64", "orbit 1 workspace x86_64"),
                "aarch64-darwin"
            )
            .is_err()
        );
        assert!(versions_name("eon 0.9.0 g1-abc", "aarch64-darwin").is_err());

        // The largest window, in shares of the main display.
        assert_eq!(
            region(
                &[[0.0, 0.0, 100.0, 50.0], [400.0, 0.0, 800.0, 600.0]],
                [1600.0, 800.0]
            ),
            Ok(Region {
                x: 0.25,
                y: 0.0,
                width: 0.5,
                height: 0.75
            })
        );
        assert!(
            region(&[], [1600.0, 800.0]).is_err()
                && region(&[[0.0, 0.0, 0.0, 600.0]], [1600.0, 800.0]).is_err()
        );
        assert_eq!(console("yu", "yu"), Ok(()));
        assert!(
            console("root", "yu")
                .unwrap_err()
                .contains("sysadminctl -autologin")
        );
    }
}
