//! The PTY floor end to end: the completion writer and the reader on a real
//! PTY, the status query and its reply, and the ONLCR byte oracle.

use std::process::Command;

/// Two samples of `ascii` drain the corpus, one CR per LF and the query:
/// 16,777,289 + 272,685 + 4, plus only content-checked CR duplicates on Apple.
#[test]
fn floor_pty_drains_the_corpus_through_a_real_pty() {
    let out = std::env::temp_dir().join(format!("eon-bench-floor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let run = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
        .args(["floor", "--layer", "pty", "--only", "ascii"])
        .args(["--samples", "2", "--warmups", "0", "--max-load", "1000"])
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{}\n{stdout}{}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.contains("\nascii "), "{stdout}");
    let result = std::fs::read_to_string(out.join("result.json")).unwrap();
    let counts: Vec<u64> = result
        .split("\"pty_bytes\": ")
        .skip(1)
        .filter_map(|tail| {
            tail.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(counts.len(), 2, "{result}");
    for bytes in counts {
        if cfg!(target_vendor = "apple") {
            assert!(bytes >= 17_049_978, "{result}");
            // Direct stderr bypasses libtest capture so native CI retains the count.
            use std::io::Write;
            writeln!(
                std::io::stderr().lock(),
                "floor PTY: {bytes} bytes, {} duplicated CRs",
                bytes - 17_049_978
            )
            .unwrap();
        } else {
            assert_eq!(bytes, 17_049_978, "{result}");
        }
    }
    std::fs::remove_dir_all(&out).unwrap();
}

/// One drain of a small file through the real writer: the exact count
/// passes, one byte more fails the oracle, and a writer that exits before its
/// query (here, on a missing file) fails with the writer's error.
#[test]
fn drain_checks_the_byte_count_and_needs_the_query() {
    let dir = std::env::temp_dir().join(format!("eon-bench-drain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let writer = std::path::Path::new(env!("CARGO_BIN_EXE_eon-bench"));
    let (file, report) = (dir.join("lines.vt"), dir.join("report.json"));
    let contents = b"one\r\ntwo\r\n\x1b[1mthree\x1b[0m";
    std::fs::write(&file, contents).unwrap();
    let expected = eon_bench::pty::expected_bytes(contents);
    assert_eq!(expected, contents.len() as u64 + 2 + 4);

    let drained = eon_bench::pty::drain(writer, &file, &report, expected).unwrap();
    if cfg!(target_vendor = "apple") {
        assert!(drained.bytes >= expected);
    } else {
        assert_eq!(drained.bytes, expected);
    }
    assert!(drained.seconds > 0.0);

    let error = eon_bench::pty::drain(writer, &file, &report, expected + 1).err();
    assert!(
        error
            .as_deref()
            .is_some_and(|error| error.contains(&format!("expected {} plus", expected + 1))),
        "{error:?}"
    );

    let error = eon_bench::pty::drain(writer, &dir.join("missing.vt"), &report, expected).err();
    assert!(
        error.as_deref().is_some_and(
            |error| error.contains("without a status query") && error.contains("missing.vt")
        ),
        "{error:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
