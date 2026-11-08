//! The kitten workloads: kitty's `kitten __benchmark__`, the number people
//! paste, one benchmark per run. kitten writes its data through `/dev/tty`
//! and times it to its own status-report replies, as the completion writer
//! does, then prints its result to stdout, where the harness collects it. It
//! prints the time rounded to three figures and the rate in MiB/s to one
//! decimal, not the bytes it sent, so the bytes are the rate times the time.

pub const PREFIX: &str = "kitten-";

/// The benchmarks of the pinned kitten (kitty 0.47.4), as its help lists them.
pub const BENCHMARKS: [&str; 5] = ["ascii", "unicode", "csi", "images", "long_escape_codes"];

/// The pinned kitten's default repetitions of each benchmark's data.
pub const REPETITIONS: u64 = 100;

#[derive(Debug, PartialEq)]
pub struct Reported {
    pub seconds: f64,
    pub bytes: u64,
}

/// The one result line of a single-benchmark run, such as
/// `  Only ASCII chars : 1.23s      @ ESC[32m812.3  ESC[m MB/s`.
pub fn parse(output: &str) -> Result<Reported, String> {
    let text = strip_escapes(output);
    let line = text
        .lines()
        .skip_while(|line| line.trim() != "Results:")
        .find(|line| line.trim_end().ends_with("MB/s"))
        .ok_or(format!("kitten printed no result: {text:?}"))?;
    let failure = || format!("unexpected kitten result {line:?}");
    let (_, measured) = line.rsplit_once(" : ").ok_or_else(failure)?;
    let (duration, rate) = measured.split_once('@').ok_or_else(failure)?;
    let seconds = go_duration(duration.trim()).ok_or_else(failure)?;
    let rate: f64 = rate
        .trim()
        .trim_end_matches("MB/s")
        .trim()
        .parse()
        .map_err(|_| failure())?;
    if seconds <= 0.0 || rate <= 0.0 {
        return Err(failure());
    }
    Ok(Reported {
        seconds,
        bytes: (rate * f64::from(1 << 20) * seconds).round() as u64,
    })
}

/// Text without CSI sequences.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char == '\x1b' {
            if chars.next() == Some('[') {
                for char in chars.by_ref() {
                    if ('@'..='~').contains(&char) {
                        break;
                    }
                }
            }
        } else {
            out.push(char);
        }
    }
    out
}

/// Seconds in a Go `time.Duration` string, such as `1m2.5s` or `812.34ms`.
fn go_duration(text: &str) -> Option<f64> {
    let mut seconds = 0.0;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest
            .find(|char: char| !(char.is_ascii_digit() || char == '.'))
            .unwrap_or(rest.len());
        let value: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let (unit, scale) = [
            ("ns", 1e-9),
            ("us", 1e-6),
            ("µs", 1e-6),
            ("ms", 1e-3),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 3600.0),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(unit))?;
        seconds += value * scale;
        rest = &rest[unit.len()..];
    }
    (!text.is_empty()).then_some(seconds)
}

#[cfg(test)]
mod tests {
    use super::{Reported, go_duration, parse};

    /// kitty 0.47.4's output for one benchmark, colors included; the bytes
    /// are its MiB/s rate times its time.
    #[test]
    fn the_result_line_gives_time_and_bytes() {
        let output = "\x1b]\x1b\\\x1bcThese results measure the time it takes the terminal \
            to fully parse all the data sent to it.\nNote that \x1b[31mrendering is \
            suppressed\x1b[m (if the terminal supports the synchronized output escape code) \
            to better benchmark parser performance. Use the --render flag to enable \
            rendering.\n\nResults:\n  Only ASCII chars : 1.23s      @ \x1b[32m812.5  \x1b[m \
            MB/s\n";
        assert_eq!(
            parse(output).unwrap(),
            Reported {
                seconds: 1.23,
                bytes: (812.5_f64 * 1_048_576.0 * 1.23).round() as u64,
            }
        );
        assert!(parse("Results:\n").is_err());
        assert!(parse("  Only ASCII chars : 1.23s @ 812.5 MB/s\n").is_err());
    }

    #[test]
    fn go_durations_read_every_unit() {
        assert_eq!(go_duration("1m40.5s"), Some(100.5));
        assert_eq!(go_duration("812.25ms"), Some(0.81225));
        assert_eq!(go_duration("12µs"), Some(12e-6));
        assert_eq!(go_duration("1h0m0s"), Some(3600.0));
        for bad in ["", "1.2", "s", "1x"] {
            assert_eq!(go_duration(bad), None, "{bad:?}");
        }
    }
}
