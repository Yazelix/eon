//! Shared pure trace parsing, verdict evaluation, and receipt formatting for AppKit event proofs.
#![allow(dead_code)]

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const FIXTURES: [(&str, &[&str]); 6] = [
    ("static-echo", &["echo"]),
    ("moving-caret-30", &["motion", "tail"]),
    ("moving-caret-60", &["motion", "tail"]),
    ("held-repeat", &["hold", "cancel"]),
    ("configure-storm", &["same", "resize"]),
    ("modifier-storm", &["storm"]),
];
pub const ECHO_COMMITS: u64 = 2000;
pub const CARET_TICKS: u64 = 120;
pub const CARET_MOVES: u64 = 4;
pub const HOLD_REPEATS: u64 = 42;
pub const REPEAT_DELAY_MS: u64 = 300;
pub const REPEAT_INTERVAL_MS: u64 = 40;
pub const SETTLE_MS: u64 = 300;
pub const TOGGLES: u64 = 20;
pub const MODES: u64 = 10;
pub const MODIFIER_PAIRS: u64 = 200;

pub fn host_platform() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-darwin"
    } else {
        "unknown-darwin"
    }
}

pub fn has_driver_flag(args: &[String]) -> bool {
    const F: &[&str] = &[
        "--out",
        "--host",
        "--orbit",
        "--mac-proof",
        "--only",
        "--deadline",
    ];
    args.iter()
        .any(|a| a.split('=').next().is_some_and(|k| F.contains(&k)))
}

pub fn driver_flag(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_owned());
        }
    }
    None
}

fn parse_dims(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

fn parse_resized(d: &str) -> Option<((u32, u32), u64)> {
    let (dims, g) = d.split_once(" g=")?;
    Some((parse_dims(dims)?, g.parse().ok()?))
}

fn parse_present(d: &str) -> Option<(u64, Option<u64>)> {
    let (g, rev) = d.split_once(' ')?;
    let rev = match rev {
        "None" => None,
        s => Some(s.strip_prefix("Some(")?.strip_suffix(')')?.parse().ok()?),
    };
    Some((g.parse().ok()?, rev))
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct WindowCounts {
    pub frames: u64,
    pub rebuilds: u64,
    pub content_rebuilds: u64,
    pub noop_rebuilds: u64,
    pub unknown_rebuild_ends: u64,
    pub rejects: u64,
    pub resizes: u64,
    pub resize_sizes: BTreeSet<(u32, u32)>,
    pub held_key_clears: u64,
    pub modifier_presses: u64,
    pub modifier_releases: u64,
    pub probes_total: u64,
    pub probes_ok: u64,
}

#[derive(Default, Debug, Clone)]
pub struct TraceSummary {
    pub windows: BTreeMap<String, WindowCounts>,
    pub primed_before_grid: Option<bool>,
    pub saw_grid_after_prime: bool,
    pub errors: Vec<String>,
}

#[derive(Default)]
struct ProbeState {
    expected_dims: Option<(u32, u32)>,
    distinct_from_prev: bool,
    base_gen: u64,
    base_rev: u64,
    resize_gen: Option<u64>,
    intervening_resize: bool,
    saw_post_resize_present: bool,
    saw_reject_before_present: bool,
    saw_new_grid_after_reject: bool,
}

impl ProbeState {
    fn ok(&self) -> bool {
        self.expected_dims.is_some()
            && self.distinct_from_prev
            && self.resize_gen.is_some()
            && !self.intervening_resize
            && self.saw_reject_before_present
            && self.saw_new_grid_after_reject
    }
}

pub fn expected_windows(fixture: &str) -> &'static [&'static str] {
    FIXTURES
        .iter()
        .find(|(f, _)| *f == fixture)
        .map_or(&[], |(_, w)| *w)
}

fn record(
    c: &mut WindowCounts,
    probe: &mut Option<ProbeState>,
    stage: &str,
    detail: &str,
    present: Option<(u64, Option<u64>)>,
    in_ptr: bool,
) {
    match stage {
        "present_revision" => {
            c.frames += 1;
            if let Some((g, rev)) = present
                && let Some(p) = probe
                && let Some(rg) = p.resize_gen
                && g >= rg
            {
                p.saw_post_resize_present = true;
                if !p.intervening_resize
                    && rev.is_some_and(|r| r > p.base_rev)
                    && p.saw_reject_before_present
                {
                    p.saw_new_grid_after_reject = true;
                }
            }
        }
        "rebuild_begin" => c.rebuilds += 1,
        "rebuild_end" => match detail {
            "true" => c.content_rebuilds += 1,
            "false" => c.noop_rebuilds += 1,
            _ => c.unknown_rebuild_ends += 1,
        },
        "resized" => {
            c.resizes += 1;
            if let Some((dims, g)) = parse_resized(detail) {
                c.resize_sizes.insert(dims);
                if let Some(p) = probe {
                    if p.resize_gen.is_none() {
                        if p.expected_dims == Some(dims) && g > p.base_gen {
                            p.resize_gen = Some(g);
                        }
                    } else if p.expected_dims != Some(dims) {
                        p.intervening_resize = true;
                    }
                }
            }
        }
        "input_rejected" => {
            c.rejects += 1;
            if in_ptr
                && detail == "non-terminal-or-unpublished-geometry"
                && let Some(p) = probe
                && p.resize_gen.is_some()
                && !p.saw_post_resize_present
            {
                p.saw_reject_before_present = true;
            }
        }
        "key_released" if detail == "held=0" => c.held_key_clears += 1,
        "input" if detail == "modifier Pressed" => c.modifier_presses += 1,
        "input" if detail == "modifier Released" => c.modifier_releases += 1,
        _ => {}
    }
}

pub fn parse_trace(lines: &[String], expected: &[&str]) -> TraceSummary {
    let mut out = TraceSummary::default();
    let (mut recs, mut max_lost) = (Vec::with_capacity(lines.len()), 0u64);
    for line in lines {
        let p: Vec<&str> = line.splitn(7, '\t').collect();
        match (
            p.as_slice(),
            p.get(2).and_then(|s| s.parse::<u64>().ok()),
            p.get(4).and_then(|s| s.parse().ok()),
        ) {
            (["venus", _, _, _, _, stage, detail], Some(seq), Some(lost)) => {
                max_lost = max_lost.max(lost);
                recs.push((seq, *stage, *detail));
            }
            _ => out.errors.push(format!("bad trace line: {line}")),
        }
    }
    if max_lost > 0 {
        out.errors.push(format!("trace lost {max_lost} packet(s)"));
    }
    recs.sort_by_key(|(seq, _, _)| *seq);
    let (mut active, mut probe): (Option<(String, WindowCounts)>, Option<ProbeState>) =
        (None, None);
    let (mut next_seq, mut saw_grid, mut in_ptr) = (None::<u64>, false, false);
    let (mut last_pres, mut prev_dims) = (None::<(u64, u64)>, None::<(u32, u32)>);
    let finish = |c: &mut WindowCounts, probe: &mut Option<ProbeState>| {
        if let Some(p) = probe.take() {
            c.probes_total += 1;
            c.probes_ok += u64::from(p.ok());
        }
    };
    for (seq, stage, detail) in recs {
        if let Some(exp) = next_seq
            && seq != exp
        {
            out.errors.push(format!("trace seq gap: {seq} != {exp}"));
        }
        next_seq = Some(seq + 1);
        match (stage, detail) {
            ("driver_prime", _) => out.primed_before_grid = Some(!saw_grid),
            ("driver_open", phase) => {
                if active.is_some() {
                    out.errors.push(format!("nested driver_open {phase}"));
                }
                (probe, prev_dims, in_ptr) = (None, None, false);
                active = Some((phase.to_owned(), WindowCounts::default()));
            }
            ("driver_probe", detail) => {
                if let Some((_, c)) = &mut active {
                    finish(c, &mut probe);
                    let expected_dims = detail.split_whitespace().nth(1).and_then(parse_dims);
                    let distinct_from_prev = expected_dims.is_some() && expected_dims != prev_dims;
                    prev_dims = expected_dims;
                    let (base_gen, base_rev) = last_pres.unwrap_or((0, 0));
                    probe = Some(ProbeState {
                        expected_dims,
                        distinct_from_prev,
                        base_gen,
                        base_rev,
                        ..Default::default()
                    });
                }
            }
            ("driver_close", phase) => match active.take() {
                Some((open, mut c)) if open == phase => {
                    finish(&mut c, &mut probe);
                    in_ptr = false;
                    out.windows.insert(open, c);
                }
                _ => out.errors.push(format!("mismatched driver_close {phase}")),
            },
            (stage, detail) => {
                match (stage, detail) {
                    ("input", "pointer Pressed") => in_ptr = true,
                    ("input", _) | ("input_end", _) => in_ptr = false,
                    _ => {}
                }
                let present = (stage == "present_revision")
                    .then(|| parse_present(detail))
                    .flatten();
                if let Some((g, Some(rev))) = present {
                    last_pres = Some((g, rev));
                    saw_grid = true;
                    out.saw_grid_after_prime |= out.primed_before_grid.is_some();
                }
                if let Some((_, c)) = &mut active {
                    record(c, &mut probe, stage, detail, present, in_ptr);
                }
            }
        }
    }
    if let Some((open, _)) = active {
        out.errors.push(format!("unclosed trace window {open}"));
    }
    for &n in expected {
        if !out.windows.contains_key(n) {
            out.errors.push(format!("missing trace window {n}"));
        }
    }
    out
}

pub fn evaluate(fixture: &str, v: &Value, t: &TraceSummary, stderr: &str) -> (Value, Vec<String>) {
    let mut f = t.errors.clone();
    if let Some(err) = v["driver_error"].as_str() {
        f.push(err.to_owned());
    }
    let storm = stderr
        .lines()
        .filter(|l| l.contains("venus: WARN event storm:"))
        .count() as u64;
    let u = |k: &str| v[k].as_u64().unwrap_or(0);
    let b = |k: &str| v[k].as_bool().unwrap_or(false);
    let w = |k: &str| t.windows.get(k).cloned().unwrap_or_default();
    let mut c: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    let mut req = |ok: bool, msg: String| {
        if !ok {
            f.push(msg);
        }
    };
    let kv = |k: &str| format!("{k} {}", v[k]);
    let mut set = |k: &str, val: Value| {
        c.insert(k.into(), val);
    };
    match fixture {
        "static-echo" => {
            let e = w("echo");
            req(u("echo_calls") >= ECHO_COMMITS, kv("echo_calls"));
            req(e.rebuilds == 0, format!("echo_rebuilds {}", e.rebuilds));
            req(u("rect_changes") == 0, kv("rect_changes"));
            req(v["committed"] == "日本", kv("committed"));
            req(storm == 0, format!("storm_warns {storm}"));
            set("echo_rebuilds", e.rebuilds.into());
            set("storm_warns", storm.into());
        }
        "moving-caret-30" | "moving-caret-60" => {
            let (mo, ta) = (w("motion"), w("tail"));
            let (pre, grid) = (t.primed_before_grid == Some(true), t.saw_grid_after_prime);
            let (moved, max_rc) = (b("prime_moved"), mo.frames + 1);
            let (tk, pk) = (b("ticked"), b("parked"));
            req(pre && grid && moved, format!("prime {pre}/{grid}/{moved}"));
            req(tk && pk, format!("tick/park {tk}/{pk}"));
            req(u("distinct_rects") >= 2, kv("distinct_rects"));
            req(u("rect_changes") <= max_rc, format!("rc > {max_rc}"));
            req(u("tail_rect_changes") == 0, kv("tail_rect_changes"));
            req(ta.rebuilds == 0, format!("tail_rebuilds {}", ta.rebuilds));
            set("primed_before_grid", pre.into());
            set("grid_after_prime", grid.into());
            set("frames", mo.frames.into());
            set("tail_rebuilds", ta.rebuilds.into());
        }
        "held-repeat" => {
            let (h, ca) = (w("hold"), w("cancel"));
            let (hs, cs) = (u("hold_sent"), u("cancel_sent"));
            let (cb, cd) = (u("cancel_bytes"), u("cancel_dwell_bytes"));
            let (hc, cc, saw) = (h.held_key_clears, ca.held_key_clears, b("cancel_saw_input"));
            req(
                hs == HOLD_REPEATS,
                format!("hold_sent {hs} != {HOLD_REPEATS}"),
            );
            req(u("held_bytes") > HOLD_REPEATS / 2, kv("held_bytes"));
            req(h.frames > 0, "no presented frame during hold".into());
            req(hc >= 1, format!("hold held_key_clears {hc}"));
            req(u("dwell_bytes") == 0, kv("dwell_bytes"));
            req(saw && cb >= 3, format!("cancel_in {saw}/{cb}"));
            let c_ok = (1..HOLD_REPEATS).contains(&cs) && cc >= 1 && cd == 0;
            req(c_ok, format!("cancel sent={cs} clears={cc} dwell={cd}"));
            set("frames", h.frames.into());
            set("hold_held_key_clears", hc.into());
            set("cancel_held_key_clears", cc.into());
        }
        "configure-storm" => {
            let (sa, re) = (w("same"), w("resize"));
            let (bound, rrc) = (re.frames.min(MODES + 1), u("resize_rect_changes"));
            let (sr, sn) = (sa.resizes, sa.resize_sizes.len());
            req(sr >= TOGGLES && sn == 1, format!("same {sr} sizes {sn}"));
            req(u("same_rect_changes") == 0, kv("same_rect_changes"));
            req(sa.rebuilds == 0, format!("same_rebuilds {}", sa.rebuilds));
            let (pt, po) = (re.probes_total, re.probes_ok);
            req(pt >= MODES && po >= MODES, format!("probes {po}/{pt}"));
            let in_b = (1..=bound).contains(&rrc);
            req(in_b, format!("rrc {rrc} !in 1..={bound}"));
            set("same_resizes", sr.into());
            set("same_rebuilds", sa.rebuilds.into());
            set("resize_frames", re.frames.into());
            set("resize_rejects", re.rejects.into());
            set("resize_probes_total", pt.into());
            set("resize_probes_ok", po.into());
        }
        "modifier-storm" => {
            let st = w("storm");
            let (mp, mr) = (st.modifier_presses, st.modifier_releases);
            let (cr, nr, unk) = (
                st.content_rebuilds,
                st.noop_rebuilds,
                st.unknown_rebuild_ends,
            );
            let (ma, ea) = (u("missed_acks"), u("extra_acks"));
            let m_ok = mp == MODIFIER_PAIRS && mr == MODIFIER_PAIRS;
            req(m_ok, format!("mods {mp}/{mr} != {MODIFIER_PAIRS}"));
            let matched = st.rebuilds == cr + nr && unk == 0;
            req(
                matched,
                format!("rebuilds {} != {cr}+{nr} unk={unk}", st.rebuilds),
            );
            req(cr == 0, format!("content_rebuilds {cr}"));
            req(ma == 0 && ea == 0, format!("acks missed={ma} extra={ea}"));
            req(u("rect_changes") == 0, kv("rect_changes"));
            req(storm == 0, format!("storm_warns {storm}"));
            set("modifier_presses", mp.into());
            set("modifier_releases", mr.into());
            set("content_rebuilds", cr.into());
            set("noop_rebuilds", nr.into());
            set("storm_warns", storm.into());
        }
        other => {
            f.push(format!("unknown fixture {other}"));
            return (Value::Null, f);
        }
    }
    (Value::Object(c), f)
}

pub fn receipt_json(
    platform: &str,
    fix: &str,
    host: &str,
    res: &str,
    fail: &[String],
    counts: Value,
) -> Value {
    json!({
        "run": format!("{fix}-{host}"), "fixture": fix, "candidate": host,
        "platform": platform, "host": host, "input_method": "AppKit NSTextInputClient",
        "result": res, "failures": fail, "counts": counts,
    })
}

pub fn result_json(platform: &str, host: &str, worker_flags: &[&str], cells: Vec<Value>) -> Value {
    let ok = !cells.is_empty() && cells.iter().all(|c| c["result"] == "pass");
    json!({
        "schema": 1, "tool": "appkit-event-driver", "suite": "events",
        "platform": platform, "host": host, "result": if ok { "pass" } else { "fail" },
        "conditions": {
            "echo_commits": ECHO_COMMITS, "caret_ticks": CARET_TICKS,
            "caret_moves_per_tick": CARET_MOVES, "hold_repeats": HOLD_REPEATS,
            "repeat_delay_ms": REPEAT_DELAY_MS, "repeat_interval_ms": REPEAT_INTERVAL_MS,
            "repeat_settle_ms": SETTLE_MS, "same_configures": TOGGLES,
            "resize_modes": MODES, "modifier_pairs": MODIFIER_PAIRS,
            "worker_flags": worker_flags,
        },
        "cells": cells,
    })
}
