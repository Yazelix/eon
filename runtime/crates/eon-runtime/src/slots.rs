//! EON-C43 host policy and pure admission core; persistence and commands follow.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "EON-C43 persistence and CLI follow in .21.6 and .21.9"
    )
)]

use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(test)]
mod sim;

// A host policy has eighteen values; bound parsing cost even for malformed input.
const MAX_POLICY_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SlotClass {
    Heavy,
    Nix,
    Priority,
}

/// Product-supplied defaults, also the resolved host policy. Runtime supplies no defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotPolicy {
    pub count: u16,
    pub default_class: SlotClass,
    pub nix_max_running: u16,
    pub priority_burst: u64,
    pub nice: u8,
    pub cargo_jobs: u16,
    pub test_threads: u16,
    pub wait_timeout_seconds: u64,
    pub poll_seconds: u64,
    pub status_interval_seconds: u64,
    pub wait_alert_seconds: u64,
    pub max_load_per_cpu: f64,
    pub min_available_memory_percent: f64,
    pub min_available_memory_mib: u64,
    pub max_memory_pressure_percent: f64,
    pub max_io_pressure_percent: f64,
    pub guard_clear_seconds: u64,
    pub quiet_max_seconds: u64,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Configuration {
    slots: Settings,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    count: Option<u16>,
    default_class: Option<SlotClass>,
    nix_max_running: Option<u16>,
    priority_burst: Option<u64>,
    nice: Option<u8>,
    cargo_jobs: Option<u16>,
    test_threads: Option<u16>,
    wait_timeout_seconds: Option<u64>,
    poll_seconds: Option<u64>,
    status_interval_seconds: Option<u64>,
    wait_alert_seconds: Option<u64>,
    max_load_per_cpu: Option<f64>,
    min_available_memory_percent: Option<f64>,
    min_available_memory_mib: Option<u64>,
    max_memory_pressure_percent: Option<f64>,
    max_io_pressure_percent: Option<f64>,
    guard_clear_seconds: Option<u64>,
    quiet: QuietSettings,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct QuietSettings {
    max_seconds: Option<u64>,
}

impl SlotPolicy {
    fn parse(
        source: &str,
        path: &Path,
        defaults: &Self,
        mem_total_mib: Option<u64>,
    ) -> Result<Self, String> {
        let invalid = |detail: String| format!("invalid slot policy {}: {detail}", path.display());
        let settings = toml::from_str::<Configuration>(source)
            .map_err(|error| invalid(error.to_string()))?
            .slots;
        let policy = Self {
            count: settings.count.unwrap_or(defaults.count),
            default_class: settings.default_class.unwrap_or(defaults.default_class),
            nix_max_running: settings.nix_max_running.unwrap_or(defaults.nix_max_running),
            priority_burst: settings.priority_burst.unwrap_or(defaults.priority_burst),
            nice: settings.nice.unwrap_or(defaults.nice),
            cargo_jobs: settings.cargo_jobs.unwrap_or(defaults.cargo_jobs),
            test_threads: settings.test_threads.unwrap_or(defaults.test_threads),
            wait_timeout_seconds: settings
                .wait_timeout_seconds
                .unwrap_or(defaults.wait_timeout_seconds),
            poll_seconds: settings.poll_seconds.unwrap_or(defaults.poll_seconds),
            status_interval_seconds: settings
                .status_interval_seconds
                .unwrap_or(defaults.status_interval_seconds),
            wait_alert_seconds: settings
                .wait_alert_seconds
                .unwrap_or(defaults.wait_alert_seconds),
            max_load_per_cpu: settings
                .max_load_per_cpu
                .unwrap_or(defaults.max_load_per_cpu),
            min_available_memory_percent: settings
                .min_available_memory_percent
                .unwrap_or(defaults.min_available_memory_percent),
            min_available_memory_mib: settings
                .min_available_memory_mib
                .unwrap_or(defaults.min_available_memory_mib),
            max_memory_pressure_percent: settings
                .max_memory_pressure_percent
                .unwrap_or(defaults.max_memory_pressure_percent),
            max_io_pressure_percent: settings
                .max_io_pressure_percent
                .unwrap_or(defaults.max_io_pressure_percent),
            guard_clear_seconds: settings
                .guard_clear_seconds
                .unwrap_or(defaults.guard_clear_seconds),
            quiet_max_seconds: settings
                .quiet
                .max_seconds
                .unwrap_or(defaults.quiet_max_seconds),
        };
        for (key, valid, range) in [
            ("count", policy.count > 0, "1..65535"),
            ("cargo_jobs", policy.cargo_jobs > 0, "1..65535"),
            ("test_threads", policy.test_threads > 0, "1..65535"),
            (
                "nix_max_running",
                policy.nix_max_running > 0 && policy.nix_max_running <= policy.count,
                "1..slots.count",
            ),
            ("nice", policy.nice <= 19, "0..19"),
            (
                "priority_burst",
                policy.priority_burst > 0,
                "a positive integer",
            ),
            (
                "poll_seconds",
                policy.poll_seconds > 0,
                "a positive integer",
            ),
            (
                "status_interval_seconds",
                policy.status_interval_seconds > 0,
                "a positive integer",
            ),
            (
                "quiet.max_seconds",
                policy.quiet_max_seconds > 0,
                "a positive integer",
            ),
            (
                "max_load_per_cpu",
                policy.max_load_per_cpu.is_finite() && policy.max_load_per_cpu > 0.0,
                "finite and positive",
            ),
        ] {
            if !valid {
                return Err(invalid(format!("slots.{key} must be {range}")));
            }
        }
        for (key, value) in [
            (
                "min_available_memory_percent",
                policy.min_available_memory_percent,
            ),
            (
                "max_memory_pressure_percent",
                policy.max_memory_pressure_percent,
            ),
            ("max_io_pressure_percent", policy.max_io_pressure_percent),
        ] {
            if !value.is_finite() || value <= 0.0 || value > 100.0 {
                return Err(invalid(format!(
                    "slots.{key} must be finite, greater than 0 and at most 100"
                )));
            }
        }
        let total = mem_total_mib.filter(|total| *total > 0).ok_or_else(|| {
            invalid("slots.min_available_memory_mib requires known positive host MemTotal".into())
        })?;
        if policy.min_available_memory_mib == 0 || policy.min_available_memory_mib > total {
            return Err(invalid(format!(
                "slots.min_available_memory_mib must be positive and at most host MemTotal ({total} MiB)"
            )));
        }
        Ok(policy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    Clear,
    Held,
    Unknown,
}

struct Observations<'a> {
    /// Opaque caller-supplied timestamps; ordering uses registration sequence only.
    now: u64,
    /// Ok contains a validated policy; errors hold grants without losing tickets.
    policy: Result<&'a SlotPolicy, &'a str>,
    guard: Gate,
    quiet: Gate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Queued,
    Held { granted_at: u64 },
    Unknown { granted_at: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ticket {
    class: SlotClass,
    queued_at: u64,
    phase: Phase,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Queue {
    last_sequence: u64,
    tickets: BTreeMap<u64, Ticket>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Enqueue(SlotClass),
    Cancel(u64),
    TryGrant,
    WrapperLost(u64),
    /// The adapter has proved work ended and the slot lock is free.
    ProvenEnded(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HoldReason {
    InvalidPolicy(String),
    GuardHeld,
    GuardUnknown,
    QuietHeld,
    QuietUnknown,
    Capacity,
    NixCapacity,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    UnsupportedClass(SlotClass),
    SequenceExhausted,
    MissingTicket(u64),
    AlreadyGranted(u64),
    NotGranted(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Effect {
    Enqueued(u64),
    Granted(u64),
    Cancelled(u64),
    Unknown(u64),
    Released(u64),
    Held(HoldReason),
    Refused(Refusal),
}

impl Queue {
    /// The later adapter serializes and persists this state/effect before execution.
    fn apply(&mut self, input: Input, observations: Observations<'_>) -> Effect {
        match input {
            Input::Enqueue(class) => {
                if class == SlotClass::Priority {
                    return Effect::Refused(Refusal::UnsupportedClass(class));
                }
                let Some(sequence) = self.last_sequence.checked_add(1) else {
                    return Effect::Refused(Refusal::SequenceExhausted);
                };
                self.last_sequence = sequence;
                self.tickets.insert(
                    sequence,
                    Ticket {
                        class,
                        queued_at: observations.now,
                        phase: Phase::Queued,
                    },
                );
                Effect::Enqueued(sequence)
            }
            Input::TryGrant => self.try_grant(observations),
            Input::Cancel(sequence) => match self.tickets.get(&sequence) {
                None => Effect::Refused(Refusal::MissingTicket(sequence)),
                Some(ticket) if ticket.phase != Phase::Queued => {
                    Effect::Refused(Refusal::AlreadyGranted(sequence))
                }
                Some(_) => {
                    self.tickets.remove(&sequence);
                    Effect::Cancelled(sequence)
                }
            },
            Input::WrapperLost(sequence) | Input::ProvenEnded(sequence) => {
                let Some(ticket) = self.tickets.get_mut(&sequence) else {
                    return Effect::Refused(Refusal::MissingTicket(sequence));
                };
                let (Phase::Held { granted_at } | Phase::Unknown { granted_at }) = ticket.phase
                else {
                    return Effect::Refused(Refusal::NotGranted(sequence));
                };
                if matches!(input, Input::WrapperLost(_)) {
                    ticket.phase = Phase::Unknown { granted_at };
                    Effect::Unknown(sequence)
                } else {
                    self.tickets.remove(&sequence);
                    Effect::Released(sequence)
                }
            }
        }
    }

    fn try_grant(&mut self, observations: Observations<'_>) -> Effect {
        let policy = match observations.policy {
            Ok(policy) => policy,
            Err(detail) => return Effect::Held(HoldReason::InvalidPolicy(detail.into())),
        };
        match observations.guard {
            Gate::Held => return Effect::Held(HoldReason::GuardHeld),
            Gate::Unknown => return Effect::Held(HoldReason::GuardUnknown),
            Gate::Clear => {}
        }
        match observations.quiet {
            Gate::Held => return Effect::Held(HoldReason::QuietHeld),
            Gate::Unknown => return Effect::Held(HoldReason::QuietUnknown),
            Gate::Clear => {}
        }
        let mut holders = 0;
        let mut nix = 0;
        for ticket in self
            .tickets
            .values()
            .filter(|ticket| ticket.phase != Phase::Queued)
        {
            holders += 1;
            nix += usize::from(ticket.class == SlotClass::Nix);
        }
        if holders >= usize::from(policy.count) {
            return Effect::Held(HoldReason::Capacity);
        }
        let next = self.tickets.iter().find_map(|(&sequence, ticket)| {
            let eligible = match ticket.class {
                SlotClass::Heavy => true,
                SlotClass::Nix => nix < usize::from(policy.nix_max_running),
                SlotClass::Priority => false,
            };
            (ticket.phase == Phase::Queued && eligible).then_some(sequence)
        });
        match next {
            Some(sequence) => {
                self.tickets.get_mut(&sequence).unwrap().phase = Phase::Held {
                    granted_at: observations.now,
                };
                Effect::Granted(sequence)
            }
            None => Effect::Held(
                if self
                    .tickets
                    .values()
                    .any(|ticket| ticket.phase == Phase::Queued)
                {
                    HoldReason::NixCapacity
                } else {
                    HoldReason::Empty
                },
            ),
        }
    }
}

pub(super) struct HostPaths {
    pub(super) configuration: PathBuf,
    pub(super) state: PathBuf,
}

impl HostPaths {
    pub(super) fn for_current_user() -> io::Result<Self> {
        Ok(Self::from_account_home(account_home()?))
    }

    fn from_account_home(home: PathBuf) -> Self {
        Self {
            configuration: home.join(".config/eon/slots.toml"),
            state: home.join(".local/state/eon/slots"),
        }
    }

    #[cfg(test)]
    fn in_test_home(home: PathBuf) -> Self {
        Self::from_account_home(home)
    }

    pub(super) fn read_policy(
        &self,
        defaults: &SlotPolicy,
        mem_total_mib: Option<u64>,
    ) -> Result<SlotPolicy, String> {
        let read = || -> io::Result<String> {
            let file = match File::open(&self.configuration) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
                Err(error) => return Err(error),
            };
            let mut source = String::new();
            file.take(MAX_POLICY_BYTES + 1)
                .read_to_string(&mut source)?;
            if source.len() as u64 > MAX_POLICY_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "slot policy exceeds 64 KiB",
                ));
            }
            Ok(source)
        };
        let source = read().map_err(|error| {
            format!(
                "cannot read slot policy {}: {error}",
                self.configuration.display()
            )
        })?;
        SlotPolicy::parse(&source, &self.configuration, defaults, mem_total_mib)
    }
}

#[cfg(target_os = "linux")]
fn account_home() -> io::Result<PathBuf> {
    use std::ffi::CStr;
    let uid = rustix::process::geteuid().as_raw();
    let mut bytes = vec![0u8; 1024];
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: the output pointers and buffer are valid for this call. Strings
        // are copied before the buffer drops; a null result is never dereferenced.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            // Bound malformed NSS records as well as normal buffer growth.
            if bytes.len() >= 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "OS account record exceeds 1 MiB",
                ));
            }
            bytes.resize(bytes.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("OS account for effective UID {uid} not found"),
            ));
        }
        // SAFETY: successful getpwuid_r initialized entry and its buffer-backed strings.
        let entry = unsafe { entry.assume_init() };
        if entry.pw_dir.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "OS account has no home",
            ));
        }
        // SAFETY: pw_dir is a non-null, NUL-terminated string in the live buffer.
        return validated_home(unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes());
    }
}

#[cfg(not(target_os = "linux"))]
fn account_home() -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "EON-C43 canonical account resolution is supported only on Linux",
    ))
}

fn validated_home(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let home = PathBuf::from(OsStr::from_bytes(bytes));
    if !home.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OS account home must be absolute",
        ));
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::ffi::OsStrExt,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "eon-slots-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parse(source: &str) -> Result<SlotPolicy, String> {
        SlotPolicy::parse(
            source,
            Path::new("fixture/slots.toml"),
            &crate::fixtures::inputs().slot_policy,
            Some(65536),
        )
    }

    #[test]
    fn partial_and_complete_policy_resolve_supplied_defaults() {
        let defaults = crate::fixtures::inputs().slot_policy;
        assert_eq!(parse("").unwrap(), defaults);
        let mut expected = defaults.clone();
        expected.count = 3;
        assert_eq!(parse("[slots]\ncount = 3").unwrap(), expected);
        let source = "[slots]
count = 4
default_class = 'priority'
nix_max_running = 3
priority_burst = 2
nice = 10
cargo_jobs = 8
test_threads = 3
wait_timeout_seconds = 100
poll_seconds = 2
status_interval_seconds = 30
wait_alert_seconds = 900
max_load_per_cpu = 0.5
min_available_memory_percent = 25.5
min_available_memory_mib = 8192
max_memory_pressure_percent = 5
max_io_pressure_percent = 30
guard_clear_seconds = 10
[slots.quiet]
max_seconds = 1000";
        assert_eq!(
            parse(source).unwrap(),
            SlotPolicy {
                count: 4,
                default_class: SlotClass::Priority,
                nix_max_running: 3,
                priority_burst: 2,
                nice: 10,
                cargo_jobs: 8,
                test_threads: 3,
                wait_timeout_seconds: 100,
                poll_seconds: 2,
                status_interval_seconds: 30,
                wait_alert_seconds: 900,
                max_load_per_cpu: 0.5,
                min_available_memory_percent: 25.5,
                min_available_memory_mib: 8192,
                max_memory_pressure_percent: 5.0,
                max_io_pressure_percent: 30.0,
                guard_clear_seconds: 10,
                quiet_max_seconds: 1000,
            }
        );
        assert_eq!(
            parse("[slots]\ndefault_class = 'nix'")
                .unwrap()
                .default_class,
            SlotClass::Nix
        );
    }

    #[test]
    fn policy_rejects_malformed_unknown_and_wrongly_typed_values() {
        for source in [
            "[slots",
            "other = 1",
            "[slots]\ncout = 1",
            "[slots.quiet]\nother = 1",
            "[slots]\ncount = 1\ncount = 2",
            "[slots]\ncount = true",
            "[slots]\ncount = 1.5",
            "[slots]\nquiet = 1",
            "[slots]\ndefault_class = 'other'",
            "[slots]\ndefault_class = 'Heavy'",
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains("fixture/slots.toml"), "{error}");
        }
    }

    #[test]
    fn policy_checks_every_numeric_range_and_boundary() {
        for key in ["count", "cargo_jobs", "test_threads"] {
            for value in ["0", "-1", "65536", "18446744073709551616"] {
                assert!(
                    parse(&format!("[slots]\n{key} = {value}")).is_err(),
                    "{key}={value}"
                );
            }
            for value in ["1", "65535"] {
                parse(&format!("[slots]\n{key} = {value}")).unwrap();
            }
        }
        for (key, invalid, valid) in [
            ("nix_max_running", &["0", "-1", "3"][..], &["1", "2"][..]),
            ("nice", &["-1", "20", "256"], &["0", "19"]),
            (
                "priority_burst",
                &["-1", "0"],
                &["1", "9223372036854775807"],
            ),
            ("poll_seconds", &["-1", "0"], &["1", "9223372036854775807"]),
            (
                "status_interval_seconds",
                &["-1", "0"],
                &["1", "9223372036854775807"],
            ),
            (
                "quiet.max_seconds",
                &["-1", "0"],
                &["1", "9223372036854775807"],
            ),
            (
                "wait_timeout_seconds",
                &["-1"],
                &["0", "9223372036854775807"],
            ),
            ("wait_alert_seconds", &["-1"], &["0", "9223372036854775807"]),
            (
                "guard_clear_seconds",
                &["-1"],
                &["0", "9223372036854775807"],
            ),
            (
                "min_available_memory_mib",
                &["-1", "0", "65537"],
                &["1", "65536"],
            ),
            (
                "max_load_per_cpu",
                &["0", "-0.1", "nan", "inf", "-inf"],
                &["0.1", "1e308"],
            ),
        ] {
            for value in invalid {
                let error = parse(&format!("[slots]\n{key} = {value}")).unwrap_err();
                assert!(error.contains("slots.toml"), "{key}={value}: {error}");
            }
            for value in valid {
                parse(&format!("[slots]\n{key} = {value}"))
                    .unwrap_or_else(|error| panic!("{key}={value}: {error}"));
            }
        }
        for key in [
            "min_available_memory_percent",
            "max_memory_pressure_percent",
            "max_io_pressure_percent",
        ] {
            for value in ["0", "-0.1", "100.1", "nan", "inf", "-inf"] {
                assert!(
                    parse(&format!("[slots]\n{key} = {value}")).is_err(),
                    "{key}={value}"
                );
            }
            for value in ["0.1", "100"] {
                parse(&format!("[slots]\n{key} = {value}")).unwrap();
            }
        }
    }

    #[test]
    fn memory_floor_requires_a_known_sufficient_host_total() {
        let defaults = crate::fixtures::inputs().slot_policy;
        for total in [None, Some(0), Some(4095)] {
            assert!(SlotPolicy::parse("", Path::new("slots.toml"), &defaults, total).is_err());
        }
        SlotPolicy::parse("", Path::new("slots.toml"), &defaults, Some(4096)).unwrap();
    }

    #[test]
    fn injected_home_reads_only_its_bounded_policy_without_creating_state() {
        let fixture = Fixture::new();
        let byte_paths =
            HostPaths::in_test_home(fixture.0.join(std::ffi::OsStr::from_bytes(b"home-\xff")));
        let defaults = crate::fixtures::inputs().slot_policy;
        assert_eq!(
            byte_paths.configuration,
            fixture.0.join(std::ffi::OsStr::from_bytes(
                b"home-\xff/.config/eon/slots.toml"
            ))
        );
        assert_eq!(
            byte_paths.state,
            fixture.0.join(std::ffi::OsStr::from_bytes(
                b"home-\xff/.local/state/eon/slots"
            ))
        );
        // APFS rejects non-UTF-8 filenames; only the path assertions need those bytes.
        let paths = HostPaths::in_test_home(fixture.0.join("home"));
        assert_eq!(paths.read_policy(&defaults, Some(65536)).unwrap(), defaults);
        assert!(!paths.configuration.parent().unwrap().exists());
        fs::create_dir_all(paths.configuration.parent().unwrap()).unwrap();
        fs::write(&paths.configuration, "[slots]\ncount = 3").unwrap();
        assert_eq!(paths.read_policy(&defaults, Some(65536)).unwrap().count, 3);
        for source in [vec![0xff], vec![b' '; (MAX_POLICY_BYTES + 1) as usize]] {
            fs::write(&paths.configuration, source).unwrap();
            assert!(
                paths
                    .read_policy(&defaults, Some(65536))
                    .unwrap_err()
                    .contains("slots.toml")
            );
        }
        fs::remove_file(&paths.configuration).unwrap();
        fs::create_dir(&paths.configuration).unwrap();
        assert!(paths.read_policy(&defaults, Some(65536)).is_err());
        assert!(!paths.state.exists());
    }

    #[test]
    fn account_home_validation_preserves_bytes_and_refuses_missing_or_relative_home() {
        for bytes in [b"".as_slice(), b"relative", b"~/home"] {
            assert_eq!(
                validated_home(bytes).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert_eq!(
            validated_home(b"/home/\xff")
                .unwrap()
                .as_os_str()
                .as_bytes(),
            b"/home/\xff"
        );
        #[cfg(not(target_os = "linux"))]
        assert!(
            matches!(HostPaths::for_current_user(), Err(error) if error.kind() == io::ErrorKind::Unsupported)
        );
    }

    #[cfg(target_os = "linux")]
    fn isolated_account() -> bool {
        // This gate reads only OS identity/mount fixtures, never host policy/state.
        account_home().is_ok_and(|home| home == Path::new("/tmp/eon-slot-account"))
            && fs::read_to_string("/etc/passwd").is_ok_and(|text| {
                text.contains(":Eon isolated test account:/tmp/eon-slot-account:")
            })
            && fs::read_to_string("/etc/nsswitch.conf")
                .is_ok_and(|text| text == "passwd: files\ngroup: files\n")
            && fs::read_to_string("/proc/self/mountinfo")
                .is_ok_and(|text| text.lines().any(|line| line.contains(" /etc/passwd ")))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn canonical_home_ignores_session_and_test_root_overrides() {
        if !isolated_account() {
            eprintln!(
                "SKIP canonical slot policy access: requires eon-test's private OS account and mounts (not direct Cargo or package-build harnesses)"
            );
            return;
        }
        let fixture = Fixture::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "slots::tests::canonical_child",
                "--nocapture",
                "--",
                "--slot-root",
            ])
            .arg(&fixture.0)
            .env("EON_SLOT_CHECK_CANONICAL_CHILD", "1")
            .env("HOME", &fixture.0)
            .env("USER", "another-account")
            .env("LOGNAME", "another-account")
            .env("XDG_CONFIG_HOME", &fixture.0)
            .env("XDG_STATE_HOME", &fixture.0)
            .env("XDG_RUNTIME_DIR", &fixture.0)
            .env("EON_CONFIG_HOME", &fixture.0)
            .env("EON_RUNTIME_DIR", &fixture.0)
            .env("EON_WINDOW_RUNTIME_DIR", &fixture.0)
            .env("EON_SLOT_TEST_ROOT", &fixture.0)
            .env("EON_SLOTS_TEST_ROOT", &fixture.0)
            .current_dir(&fixture.0)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("canonical slot policy checked"),
            "{output:?}"
        );
        assert!(fs::read_dir(&fixture.0).unwrap().next().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn canonical_child() {
        if std::env::var_os("EON_SLOT_CHECK_CANONICAL_CHILD").is_none() {
            return;
        }
        assert!(
            isolated_account(),
            "canonical policy access needs isolated OS account mounts"
        );
        let paths = HostPaths::for_current_user().unwrap();
        assert_eq!(
            paths.configuration,
            Path::new("/tmp/eon-slot-account/.config/eon/slots.toml")
        );
        assert_eq!(
            paths.state,
            Path::new("/tmp/eon-slot-account/.local/state/eon/slots")
        );
        let defaults = crate::fixtures::inputs().slot_policy;
        assert_eq!(paths.read_policy(&defaults, Some(65536)).unwrap(), defaults);
        assert!(!paths.configuration.exists());
        assert!(!paths.state.exists());
        println!("canonical slot policy checked");
    }
}
