//! The vtebench workloads: alacritty/vtebench's benchmark scripts, each run
//! once at the suite's grid, as vtebench runs them, and split the way
//! vtebench writes a sample: a reset (`ESC c`) and the setup script's output,
//! written untimed first, then the benchmark's output repeated until it
//! reaches the minimum size, by default 1 MiB, vtebench's `--min-bytes
//! 1048576` as Orbit's earlier measurements ran it. Unlike vtebench, which
//! times only its blocking write, the completion writer times the benchmark
//! part to the status reply. The scripts size their output from the terminal
//! (`tput`), so the files depend on the grid and are not pinned; each records
//! its SHA-256.

use std::{
    ffi::OsString,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{environment, pty, sha256::Sha256};

/// The benchmarks of vtebench's `benchmarks/` directory at the pinned
/// revision.
pub const BENCHMARKS: [&str; 12] = [
    "cursor_motion",
    "dense_cells",
    "light_cells",
    "medium_cells",
    "scrolling",
    "scrolling_bottom_region",
    "scrolling_bottom_small_region",
    "scrolling_fullscreen",
    "scrolling_top_region",
    "scrolling_top_small_region",
    "sync_medium_cells",
    "unicode",
];

pub const PREFIX: &str = "vtebench-";
/// The default minimum size of a sample.
pub const MIN_BYTES: u64 = 1 << 20;
const RESET: &[u8] = b"\x1bc";

pub struct Workload {
    pub id: String,
    /// The reset and setup, written before the timed part.
    pub setup: PathBuf,
    /// The timed part.
    pub path: PathBuf,
    pub length: u64,
    pub sha256: String,
}

/// Writes `OUT/<benchmark>.setup.vt` and `OUT/<benchmark>.vt`, at least
/// `min_bytes`, for each of `names`, running the scripts under `benchmarks` with a controlling terminal of
/// `rows` x `cols` and `PATH` set to `path` when given.
pub fn generate(
    benchmarks: &Path,
    names: &[&str],
    out: &Path,
    (rows, cols): (u16, u16),
    min_bytes: u64,
    path: Option<&OsString>,
) -> Result<Vec<Workload>, String> {
    std::fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
    names
        .iter()
        .map(|name| {
            let dir = benchmarks.join(name);
            let setup = dir.join("setup");
            let mut setup_bytes = RESET.to_vec();
            if setup.exists() {
                setup_bytes.extend(script_output(&setup, rows, cols, path)?);
            }
            let benchmark = script_output(&dir.join("benchmark"), rows, cols, path)?;
            let bytes = repeat(&benchmark, min_bytes)
                .ok_or(format!("{}: the benchmark wrote nothing", dir.display()))?;
            let setup = out.join(format!("{name}.setup.vt"));
            let file = out.join(format!("{name}.vt"));
            for (path, bytes) in [(&setup, &setup_bytes), (&file, &bytes)] {
                std::fs::write(path, bytes)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
            }
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            Ok(Workload {
                id: format!("{PREFIX}{name}"),
                setup,
                path: file,
                length: bytes.len() as u64,
                sha256: hasher.hex(),
            })
        })
        .collect()
}

/// A vtebench sample's timed bytes: the benchmark repeated whole until at
/// least `min_bytes`; `None` for an empty benchmark.
fn repeat(benchmark: &[u8], min_bytes: u64) -> Option<Vec<u8>> {
    if benchmark.is_empty() {
        return None;
    }
    let min_bytes = usize::try_from(min_bytes.max(1)).ok()?;
    let count = (min_bytes - 1) / benchmark.len() + 1;
    Some(benchmark.repeat(count))
}

/// A script's stdout, run by `sh` in its own session whose controlling
/// terminal is a fresh PTY of the grid, as its stdin; the scripts find the
/// terminal with `ps -o tty=` and size their output with `tput`.
fn script_output(
    script: &Path,
    rows: u16,
    cols: u16,
    path: Option<&OsString>,
) -> Result<Vec<u8>, String> {
    let failure = |error: String| format!("{}: {error}", script.display());
    // The master stays open, unread, until the script ends; the script
    // writes only to its stdout.
    let (_master, slave) = pty::open(rows, cols).map_err(|error| failure(error.to_string()))?;
    let mut command = Command::new("sh");
    environment::die_with_parent(&mut command);
    command
        .arg(script)
        .stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("TERM", "xterm-256color")
        .env_remove("COLUMNS")
        .env_remove("LINES");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command
        .output()
        .map_err(|error| failure(format!("sh: {error}")))?;
    if !output.status.success() {
        return Err(failure(format!(
            "{}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::{MIN_BYTES, repeat};

    /// The benchmark repeats whole until 1 MiB, as vtebench's
    /// `Benchmark::new` repeats it; exactly 1 MiB needs no extra.
    #[test]
    fn a_sample_repeats_the_benchmark_whole_to_one_mib() {
        let sample = repeat(&[b'x'; 1000], MIN_BYTES).unwrap();
        assert_eq!(sample.len(), 1049 * 1000);
        let quarter = vec![b'y'; MIN_BYTES as usize / 4];
        assert_eq!(repeat(&quarter, MIN_BYTES).unwrap().len() as u64, MIN_BYTES);
        assert!(repeat(b"", MIN_BYTES).is_none());
    }
}
