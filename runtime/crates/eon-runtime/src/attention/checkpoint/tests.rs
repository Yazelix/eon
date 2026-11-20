use super::super::{HelperPending, OfferedNotice, STALE_AFTER};
use super::*;
use crate::checkpoint::update::tests::record;
use crate::checkpoint::{Run, RunKind};
use orbit_protocol::management::{Foreground, ProgramProcesses, ProgramState, ProgramStatus};
use std::sync::Arc;

const MS: Duration = Duration::from_millis(1);

/// Attention as polling leaves it at `now` for each (terminal, run): a fresh
/// idle reply with a raised error and a done record, an omitted record, a
/// recovered baseline for the first run, the next poll 400 ms ahead.
pub(crate) fn seeded(now: Instant, terminals: &[(&str, &str)]) -> Attention {
    let mut attention = Attention::new(true);
    attention.epoch = (now - 2000 * MS, 1_700_000_000_000);
    attention.next_poll = now + 400 * MS;
    for (index, (terminal, run)) in terminals.iter().enumerate() {
        let created = 10 * index as u64 + 2;
        attention.terminals.push(Terminal {
            session: (*terminal).into(),
            run: (*run).into(),
            update: created + 9,
            count: 3,
            background: 0,
            foreground: Foreground::Child,
            name: format!("title {index}"),
            records: vec![
                Record {
                    record: record(created, ProgramState::Error),
                    changed_at: now - 900 * MS,
                    raise: Some(Raise {
                        at: now - 800 * MS,
                        changed: created + 3,
                        seen: false,
                        notified: index == 0,
                    }),
                    blocked_notified: false,
                },
                Record {
                    record: record(created + 1, ProgramState::Done),
                    changed_at: now - 700 * MS,
                    raise: Some(Raise {
                        at: now - 600 * MS,
                        changed: created + 4,
                        seen: true,
                        notified: true,
                    }),
                    blocked_notified: true,
                },
            ],
            omitted: vec![Record {
                record: record(created + 2, ProgramState::Working),
                changed_at: now - 500 * MS,
                raise: None,
                blocked_notified: false,
            }],
            notices: PendingNotices::default(),
            replied: now - 100 * MS,
        });
    }
    if let Some((_, run)) = terminals.first() {
        attention.recovered.push(((*run).into(), Some(5)));
    }
    attention
}

fn runs() -> Vec<Run> {
    (1..=3)
        .map(|number| Run {
            kind: RunKind::Confirmed,
            terminal_id: format!("terminal-{number}"),
            run_id: format!("run-{number}"),
        })
        .collect()
}

/// terminal-3 is a pending tab's pair.
fn slots(runs: &[Run]) -> Vec<Slot<'_>> {
    runs.iter()
        .map(|run| Slot {
            run,
            pending: run.terminal_id == "terminal-3",
        })
        .collect()
}

const LIVE: [(&str, &str); 2] = [("terminal-1", "run-1"), ("terminal-2", "run-2")];

#[test]
fn a_zero_gap_restores_the_same_attention_and_drops_focus_viewing_and_cursor() {
    let runs = runs();
    let now = Instant::now();
    let mut source = seeded(now, &LIVE);
    source.focus = Some("terminal-1".into());
    source.viewing = true;
    let saved = source.checkpoint(now, &slots(&runs)).unwrap();
    assert_eq!(saved.next_poll_due_ns, 400_000_000);
    assert_eq!(saved.terminals[0].replied_age_ns, 100_000_000);
    assert_eq!(saved.epoch_age_ns, 2_000_000_000);

    let later = now + 5000 * MS;
    let restored = Attention::restore(&saved, later, Duration::ZERO).unwrap();
    assert!(restored.focus.is_none() && !restored.viewing && restored.cursor.is_none());
    assert!(restored.helper_worker.is_none());
    assert_eq!(restored.next_poll, later + 400 * MS);
    assert_eq!(restored.checkpoint(later, &slots(&runs)).unwrap(), saved);
}

#[test]
fn the_elapsed_gap_ages_past_facts_and_counts_the_poll_down() {
    let runs = runs();
    let now = Instant::now();
    let saved = seeded(now, &LIVE).checkpoint(now, &slots(&runs)).unwrap();
    let later = now + 10_000 * MS;

    let fresh = Attention::restore(&saved, later, Duration::ZERO).unwrap();
    assert_eq!(
        fresh.fresh_until(later),
        Some(later + STALE_AFTER - 100 * MS)
    );
    assert!(fresh.next_poll > later);

    let gap = STALE_AFTER + MS;
    let stale = Attention::restore(&saved, later, gap).unwrap();
    assert_eq!(stale.fresh_until(later), None);
    assert_eq!(stale.next_poll, later);
    assert_eq!(stale.terminals[0].replied, later - gap - 100 * MS);
    assert_eq!(stale.epoch.0, later - gap - 2000 * MS);

    // Part of the poll interval has passed.
    let part = Attention::restore(&saved, later, 300 * MS).unwrap();
    assert_eq!(part.next_poll, later + 100 * MS);
}

#[test]
fn an_overdue_poll_is_due_now() {
    let runs = runs();
    let now = Instant::now();
    let mut source = seeded(now, &LIVE);
    source.next_poll = now - 50 * MS;
    assert_eq!(
        source
            .checkpoint(now, &slots(&runs))
            .unwrap()
            .next_poll_due_ns,
        0
    );
}

fn unsettled(attention: &Attention, at: Instant, what: &'static str) {
    let runs = runs();
    assert_eq!(
        attention.checkpoint(at, &slots(&runs)),
        Err(Error::Unsettled(what))
    );
}

#[test]
fn unsettled_attention_refuses_capture() {
    let now = Instant::now();

    let mut helper = seeded(now, &LIVE);
    helper.helper_pending.push(HelperPending {
        sample: 1,
        session: "terminal-1".into(),
        run: "run-1".into(),
        status: ProgramStatus {
            count: 0,
            background: 0,
            processes: ProgramProcesses {
                background: Vec::new(),
                complete: false,
                foreground: None,
            },
            foreground: Foreground::Unknown,
            update: 0,
            terminal_title: String::new(),
            omitted: Vec::new(),
            records: Vec::new(),
        },
        replied: now,
        deadline: now,
    });
    unsettled(&helper, now, "an attention helper sample is in flight");

    let queued = Arc::new(NoticeDispatch::default());
    let mut notice = seeded(now, &LIVE);
    notice.terminals[1].notices.0.push(OfferedNotice {
        dispatch: Arc::downgrade(&queued),
        finish: None,
    });
    unsettled(&notice, now, "a desktop notice is still queued");
    // A dispatched, a cancelled and a dropped notice are history.
    assert!(queued.begin());
    let cancelled = Arc::new(NoticeDispatch::default());
    assert!(cancelled.cancel());
    for dispatch in [&cancelled, &Arc::new(NoticeDispatch::default())] {
        notice.terminals[1].notices.0.push(OfferedNotice {
            dispatch: Arc::downgrade(dispatch),
            finish: Some(4),
        });
    }
    notice.checkpoint(now, &slots(&runs())).unwrap();

    let no_run = "attention holds a run no terminal has";
    unsettled(&seeded(now, &[("terminal-1", "run-old")]), now, no_run);
    unsettled(&seeded(now, &[("terminal-9", "run-9")]), now, no_run);
    // A pending tab's pair has no attention.
    unsettled(&seeded(now, &[("terminal-3", "run-3")]), now, no_run);
    // The addendum's stale `recovered`, which carries only a run.
    let mut recovered = seeded(now, &LIVE);
    recovered.recovered.push(("run-gone".into(), None));
    unsettled(&recovered, now, no_run);
    // A recovered run may be a pending pair's.
    recovered.recovered.pop();
    recovered.recovered.push(("run-3".into(), Some(1)));
    recovered.checkpoint(now, &slots(&runs())).unwrap();
}

#[test]
fn a_past_fact_after_the_anchor_refuses_capture_in_every_field() {
    let now = Instant::now();
    let future = now + MS;
    let cases: [fn(&mut Attention, Instant); 6] = [
        |a, at| a.terminals[0].records[0].changed_at = at,
        |a, at| a.terminals[0].records[0].raise.as_mut().unwrap().at = at,
        |a, at| a.terminals[1].omitted[0].changed_at = at,
        |a, at| {
            a.terminals[1].omitted[0].raise = Some(Raise {
                at,
                changed: 1,
                seen: false,
                notified: false,
            });
        },
        |a, at| a.terminals[0].replied = at,
        |a, at| a.epoch.0 = at,
    ];
    for set in cases {
        let mut attention = seeded(now, &LIVE);
        set(&mut attention, future);
        unsettled(&attention, now, "an attention time lies after the capture");
        // At the anchor itself, the age is zero.
        set(&mut attention, now);
        attention.checkpoint(now, &slots(&runs())).unwrap();
    }
}

#[test]
fn a_distance_over_the_stamp_bound_refuses_capture() {
    let runs = runs();
    let now = Instant::now();
    let over = Duration::from_nanos(MAX_STAMP_NS + 1);
    let mut poll = seeded(now, &LIVE);
    poll.next_poll = now + over;
    assert_eq!(
        poll.checkpoint(now, &slots(&runs)),
        Err(Error::Unsettled("the next attention poll is out of range"))
    );
    // At the bound itself both fit.
    poll.next_poll = now + Duration::from_nanos(MAX_STAMP_NS);
    poll.checkpoint(now, &slots(&runs)).unwrap();

    // Ages past the bound: captured that far after every past fact. (An
    // instant that far before now does not exist on a booted machine.)
    let attention = seeded(now, &LIVE);
    let earliest = now - 2000 * MS;
    let late = earliest + over;
    unsettled(&attention, late, "an attention time lies after the capture");
    let mut fits = seeded(now, &LIVE);
    fits.next_poll = late;
    fits.checkpoint(earliest + Duration::from_nanos(MAX_STAMP_NS), &slots(&runs))
        .unwrap();
}

#[test]
fn restore_refuses_times_it_cannot_represent() {
    let runs = runs();
    let now = Instant::now();
    let saved = seeded(now, &LIVE).checkpoint(now, &slots(&runs)).unwrap();
    let out_of_range = Err(Error::Unrestorable("an attention time is out of range"));
    assert_eq!(
        Attention::restore(&saved, now, Duration::MAX).map(|_| ()),
        out_of_range
    );

    // An instant within MAX_STAMP_NS of the platform's last one, where a valid
    // due no longer fits.
    let mut ceiling = now;
    let mut step = Duration::from_secs(u64::MAX);
    while step >= Duration::from_secs(1) {
        if let Some(next) = ceiling.checked_add(step) {
            ceiling = next;
        } else {
            step /= 2;
        }
    }
    let mut far = saved.clone();
    far.next_poll_due_ns = MAX_STAMP_NS;
    assert_eq!(
        Attention::restore(&far, ceiling, Duration::ZERO).map(|_| ()),
        out_of_range
    );
}
