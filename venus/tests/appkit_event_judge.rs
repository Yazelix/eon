#![forbid(unsafe_code)]

#[path = "appkit_event_driver/judge.rs"]
mod judge;

use judge::*;
use serde_json::{Value, json};

fn run(fix: &str, spec: &str, val: Value, err: &str, pass: bool) {
    let mut lines = Vec::new();
    for item in spec.split(';').filter(|s| !s.is_empty()) {
        let (item, n) = item
            .split_once('*')
            .map_or((item, 1), |(k, n)| (k, n.parse().unwrap()));
        let (s, d) = item.split_once('|').unwrap_or((item, ""));
        for _ in 0..n {
            let i = lines.len();
            lines.push(format!("venus\t1\t{i}\t{i}\t0\t{s}\t{d}"));
        }
    }
    let (_, failures) = evaluate(fix, &val, &parse_trace(&lines, expected_windows(fix)), err);
    assert_eq!(failures.is_empty(), pass, "{fix}: {failures:?}");
}

#[test]
fn verdicts_pass_and_reject_boundary_regressions() {
    let ok = |f: &str, s: &str, v| run(f, s, v, "", true);
    let bad = |f: &str, s: &str, v| run(f, s, v, "", false);

    // static-echo
    let e_ok = || json!({"echo_calls": 2000, "committed": "日本"});
    let e_tr = "driver_open|echo;driver_close|echo";
    ok("static-echo", e_tr, e_ok());
    bad(
        "static-echo",
        e_tr,
        json!({"echo_calls": 2000, "committed": "日本日本"}),
    );
    run(
        "static-echo",
        e_tr,
        e_ok(),
        "venus: WARN event storm: Ime\n",
        false,
    );
    bad(
        "static-echo",
        "driver_open|echo;rebuild_begin;driver_close|echo",
        e_ok(),
    );

    // moving-caret-30 / moving-caret-60
    let c_tr = "driver_prime|pre;driver_open|motion;present_revision|1 Some(1);present_revision|1 Some(2);driver_close|motion;driver_open|tail;driver_close|tail";
    let c_late = "present_revision|1 Some(1);driver_prime|pre;driver_open|motion;present_revision|1 Some(2);driver_close|motion;driver_open|tail;driver_close|tail";
    let c_no_grid = "driver_prime|pre;driver_open|motion;present_revision|1 None*2;driver_close|motion;driver_open|tail;driver_close|tail";
    let c_tail = "driver_prime|pre;driver_open|motion;present_revision|1 Some(1);driver_close|motion;driver_open|tail;rebuild_begin;driver_close|tail";
    let cv_mc = |t, p, m, rc| json!({"ticked": t, "parked": p, "prime_moved": m, "distinct_rects": 2, "rect_changes": rc});
    ok("moving-caret-30", c_tr, cv_mc(true, true, true, 3));
    bad("moving-caret-60", c_tr, cv_mc(true, true, false, 2));
    bad("moving-caret-60", c_tr, cv_mc(false, true, true, 2));
    bad("moving-caret-60", c_tr, cv_mc(true, false, true, 2));
    bad("moving-caret-60", c_late, cv_mc(true, true, true, 3));
    bad("moving-caret-60", c_no_grid, cv_mc(true, true, true, 2));
    bad("moving-caret-30", c_tail, cv_mc(true, true, true, 2));
    bad("moving-caret-30", c_tr, cv_mc(true, true, true, 4));

    // held-repeat
    let h_tr = "driver_open|hold;present_revision|1 Some(1);input|key Released;key_released|held=0;driver_close|hold;driver_open|cancel;input|key Released;key_released|held=0;driver_close|cancel";
    let h_no_hold = "driver_open|hold;present_revision|1 Some(1);input|key Released;driver_close|hold;driver_open|cancel;input|key Released;key_released|held=0;driver_close|cancel";
    let h_no_cancel = "driver_open|hold;present_revision|1 Some(1);input|key Released;key_released|held=0;driver_close|hold;driver_open|cancel;input|key Released;driver_close|cancel";
    let h_retained = "driver_open|hold;present_revision|1 Some(1);input|key Released;key_released|held=1;driver_close|hold;driver_open|cancel;input|key Released;key_released|held=0;driver_close|cancel";
    let hv = |hs, saw, bytes, sent| json!({"hold_sent": hs, "held_bytes": 22, "cancel_saw_input": saw, "cancel_bytes": bytes, "cancel_sent": sent});
    ok("held-repeat", h_tr, hv(42, true, 4, 5));
    bad("held-repeat", h_tr, hv(0, true, 4, 5));
    bad("held-repeat", h_tr, hv(42, false, 4, 5));
    bad("held-repeat", h_tr, hv(42, true, 2, 5));
    bad("held-repeat", h_tr, hv(42, true, 4, 0));
    bad("held-repeat", h_no_hold, hv(42, true, 4, 5));
    bad("held-repeat", h_no_cancel, hv(42, true, 4, 5));
    bad("held-repeat", h_retained, hv(42, true, 4, 5));

    // configure-storm
    const R: &str =
        "input|pointer Pressed;input_rejected|non-terminal-or-unpublished-geometry;input_end;";
    let dims = |i: usize| ["960x520", "960x600"][i & 1];
    let probe = |i: usize, g: usize, m: &str, p: usize| {
        let d = dims(i);
        format!("driver_probe|{i} {d};resized|{d} g={g};{m}present_revision|{g} Some({p});")
    };
    let seq = |r: std::ops::Range<usize>| r.map(|i| probe(i, i + 2, R, i + 2)).collect::<String>();
    let wrap = |s: &str, r: &str| {
        format!(
            "present_revision|1 Some(1);driver_open|same;{s}driver_close|same;driver_open|resize;{r}driver_close|resize"
        )
    };
    let (sa, all, t0_9, t1_9) = ("resized|960x600 g=1*20;", seq(0..10), seq(0..9), seq(1..10));
    let cv = |rc| json!({"resize_rect_changes": rc});
    let cbad = |s: &str, r: &str, rc: u64| bad("configure-storm", &wrap(s, r), cv(rc));
    let p0 = |mid: &str| format!("driver_probe|0 960x520;{mid}present_revision|2 Some(2);{t1_9}");
    ok("configure-storm", &wrap(sa, &all), cv(10));
    cbad(sa, &all, 11);
    let r2 =
        "input|pointer Pressed;input_rejected|non-terminal-or-unpublished-geometry*2;input_end;";
    let five: String = (0..10)
        .map(|i| probe(i, i + 2, if i < 5 { r2 } else { "" }, i + 2))
        .collect();
    let stale: String = (0..10).map(|i| probe(i, 2, R, 1)).collect();
    let tail_s: String = (1..10).map(|i| probe(i, i + 3, R, i + 3)).collect();
    let late = format!(
        "{t0_9}driver_probe|9 960x600;resized|960x600 g=11;present_revision|11 Some(11);{R}"
    );
    let no_pres = format!("{t0_9}driver_probe|9 960x600;resized|960x600 g=11;{R}");
    let bad_rej =
        "resized|960x520 g=2;input|pointer Pressed;input_rejected|not-attached;input_end;";
    let bare_rej = "resized|960x520 g=2;input_rejected|non-terminal-or-unpublished-geometry;";
    let inter = format!(
        "driver_probe|0 960x520;resized|960x520 g=2;{R}resized|960x560 g=3;present_revision|3 Some(3);{tail_s}"
    );
    let rune_p0 = format!(
        "driver_probe|0 960x520;resized|960x520 g=2;{R}present_revision|3 Some(3);{tail_s}"
    );
    ok("configure-storm", &wrap(sa, &rune_p0), cv(10));
    cbad(sa, &five, 5);
    cbad(sa, &late, 10);
    cbad(sa, &no_pres, 9);
    cbad("", &all, 10);
    cbad(sa, &stale, 10);
    cbad(sa, &p0(&format!("resized|960x600 g=2;{R}")), 10);
    cbad(sa, &p0(bad_rej), 10);
    cbad(sa, &p0(bare_rej), 10);
    cbad(sa, &p0(&format!("resized|960x520 g=1;{R}")), 10);
    cbad(sa, &inter, 10);
    cbad("resized|960x600 g=1*19;resized|960x520 g=1;", &all, 10);

    // modifier-storm
    let m_tr = "driver_open|storm;rebuild_begin;rebuild_end|false;input|modifier Pressed*200;input|modifier Released*200;driver_close|storm";
    let m_reb = "driver_open|storm;rebuild_begin;rebuild_end|true;input|modifier Pressed*200;input|modifier Released*200;driver_close|storm";
    let m_full = "driver_open|storm;rebuild_begin;rebuild_end|full;input|modifier Pressed*200;input|modifier Released*200;driver_close|storm";
    let m_po = "driver_open|storm;input|modifier Pressed*400;driver_close|storm";
    let m_over = "driver_open|storm;input|modifier Pressed*201;input|modifier Released*201;driver_close|storm";
    ok("modifier-storm", m_tr, json!({}));
    bad("modifier-storm", m_reb, json!({}));
    bad("modifier-storm", m_full, json!({}));
    bad("modifier-storm", m_po, json!({}));
    bad("modifier-storm", m_over, json!({}));
    bad("modifier-storm", m_tr, json!({"rect_changes": 1}));
    bad("modifier-storm", m_tr, json!({"missed_acks": 1}));
    bad("modifier-storm", m_tr, json!({"extra_acks": 1}));
    bad(
        "modifier-storm",
        "driver_open|storm;driver_close|storm",
        json!({}),
    );
}

#[test]
fn driver_flag_detection_allows_cargo_test_filters() {
    let s = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    for non in [&[][..], &["filter"], &["--nocapture", "--exact"]] {
        assert!(!has_driver_flag(&s(non)));
    }
    let flags = [
        "--host",
        "--host=venus",
        "--out",
        "--orbit",
        "--mac-proof",
        "--only",
        "--deadline",
    ];
    for f in flags {
        assert!(has_driver_flag(&s(&[f, "v"])));
    }
    let df = |xs: &[&str], k: &str| driver_flag(&s(xs), k);
    assert_eq!(df(&["--host", "rune"], "--host").as_deref(), Some("rune"));
    assert_eq!(
        df(&["--out", "D", "--host=rune"], "--host").as_deref(),
        Some("rune")
    );
    assert_eq!(
        df(&["--out=D", "--host=rune"], "--out").as_deref(),
        Some("D")
    );
    assert_eq!(df(&["--host"], "--host"), None);
}

#[test]
fn trace_integrity_and_receipt_schema_fail_closed() {
    let mk = |rows: &[(u64, u64, &str, &str)]| -> Vec<String> {
        rows.iter()
            .map(|(q, l, s, d)| format!("venus\t1\t{q}\t{q}\t{l}\t{s}\t{d}"))
            .collect()
    };
    let (op, cl) = ("driver_open", "driver_close");
    assert!(
        !parse_trace(&mk(&[(0, 0, op, "echo"), (2, 0, cl, "echo")]), &["echo"])
            .errors
            .is_empty()
    );
    let lost = mk(&[(0, 1, op, "echo"), (1, 2, cl, "echo")]);
    assert_eq!(
        parse_trace(&lost, &["echo"]).errors,
        vec!["trace lost 2 packet(s)"]
    );
    assert!(
        parse_trace(&mk(&[(1, 0, cl, "echo"), (0, 0, op, "echo")]), &["echo"])
            .errors
            .is_empty()
    );
    assert!(!parse_trace(&[], &["echo"]).errors.is_empty());
    let c = receipt_json(
        "aarch64-darwin",
        "static-echo",
        "rune",
        "pass",
        &[],
        json!({"echo_calls": 2000}),
    );
    let res = result_json("aarch64-darwin", "rune", vec![c]);
    assert_eq!(res["schema"], 1);
    assert_eq!(res["tool"], "appkit-event-driver");
    assert_eq!(res["platform"], "aarch64-darwin");
    assert_eq!(res["result"], "pass");
}
