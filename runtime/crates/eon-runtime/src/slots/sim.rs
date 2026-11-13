use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    now: u64,
    count: u16,
    nix_max_running: u16,
    invalid_policy: bool,
    guard: Gate,
    quiet: Gate,
}

impl Frame {
    fn clear(count: u16, nix_max_running: u16) -> Self {
        Self {
            now: 0,
            count,
            nix_max_running,
            invalid_policy: false,
            guard: Gate::Clear,
            quiet: Gate::Clear,
        }
    }
}

fn apply(queue: &mut Queue, input: Input, frame: Frame) -> Effect {
    let mut policy = crate::fixtures::inputs().slot_policy;
    policy.count = frame.count;
    policy.nix_max_running = frame.nix_max_running;
    queue.apply(
        input,
        Observations {
            now: frame.now,
            policy: if frame.invalid_policy {
                Err("invalid fixture policy")
            } else {
                Ok(&policy)
            },
            guard: frame.guard,
            quiet: frame.quiet,
        },
    )
}

fn enqueue(queue: &mut Queue, class: SlotClass, frame: Frame) -> u64 {
    match apply(queue, Input::Enqueue(class), frame) {
        Effect::Enqueued(sequence) => sequence,
        other => panic!("enqueue refused: {other:?}"),
    }
}

#[test]
fn registration_sequence_orders_equal_or_reversed_times_and_every_reentry() {
    let mut queue = Queue::default();
    let mut frame = Frame::clear(2, 1);
    frame.now = 20;
    let first = enqueue(&mut queue, SlotClass::Heavy, frame);
    let second = enqueue(&mut queue, SlotClass::Heavy, frame);
    frame.now = 10;
    let third = enqueue(&mut queue, SlotClass::Nix, frame);
    assert_eq!(
        apply(&mut queue, Input::Cancel(first), frame),
        Effect::Cancelled(first)
    );
    let reentered = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert!(first < second && second < third && third < reentered);
    frame.now = 30;
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(second)
    );
    assert_eq!(queue.tickets[&second].queued_at, 20);
    assert_eq!(queue.tickets[&second].phase, Phase::Held { granted_at: 30 });
    assert_eq!(
        apply(&mut queue, Input::ProvenEnded(second), frame),
        Effect::Released(second)
    );
    let after_release = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert!(after_release > reentered);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(third)
    );
    assert_eq!(
        apply(&mut queue, Input::WrapperLost(third), frame),
        Effect::Unknown(third)
    );
    let after_loss = enqueue(&mut queue, SlotClass::Nix, frame);
    assert!(after_loss > after_release);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(reentered)
    );
}

#[test]
fn cancellation_on_either_side_of_grant_never_releases_a_holder() {
    let mut queue = Queue::default();
    let frame = Frame::clear(1, 1);
    let cancelled = enqueue(&mut queue, SlotClass::Heavy, frame);
    let holder = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert_eq!(
        apply(&mut queue, Input::Cancel(cancelled), frame),
        Effect::Cancelled(cancelled)
    );
    let waiting = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(holder)
    );
    let held = queue.clone();
    assert_eq!(
        apply(&mut queue, Input::Cancel(holder), frame),
        Effect::Refused(Refusal::AlreadyGranted(holder))
    );
    assert_eq!(queue, held);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Held(HoldReason::Capacity)
    );
    assert_eq!(
        apply(&mut queue, Input::ProvenEnded(holder), frame),
        Effect::Released(holder)
    );
    let released = queue.clone();
    assert_eq!(
        apply(&mut queue, Input::ProvenEnded(holder), frame),
        Effect::Refused(Refusal::MissingTicket(holder))
    );
    assert_eq!(queue, released);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(waiting)
    );
}

#[test]
fn blocked_nix_head_wins_when_its_class_capacity_returns() {
    let mut queue = Queue::default();
    let frame = Frame::clear(2, 1);
    let nix_holder = enqueue(&mut queue, SlotClass::Nix, frame);
    let nix_head = enqueue(&mut queue, SlotClass::Nix, frame);
    let heavy = enqueue(&mut queue, SlotClass::Heavy, frame);
    let next_heavy = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(nix_holder)
    );
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(heavy)
    );
    apply(&mut queue, Input::ProvenEnded(heavy), frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(next_heavy)
    );
    enqueue(&mut queue, SlotClass::Heavy, frame);
    enqueue(&mut queue, SlotClass::Heavy, frame);
    apply(&mut queue, Input::ProvenEnded(nix_holder), frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(nix_head)
    );
    assert_eq!(
        queue
            .tickets
            .values()
            .filter(|ticket| ticket.phase != Phase::Queued)
            .count(),
        2
    );
}

#[test]
fn invalid_policy_and_held_or_unknown_gates_preserve_waiters() {
    let mut queue = Queue::default();
    let frame = Frame::clear(2, 1);
    enqueue(&mut queue, SlotClass::Heavy, frame);
    let before = queue.clone();
    for (frame, reason) in [
        (
            Frame {
                invalid_policy: true,
                ..frame
            },
            HoldReason::InvalidPolicy("invalid fixture policy".into()),
        ),
        (
            Frame {
                guard: Gate::Held,
                ..frame
            },
            HoldReason::GuardHeld,
        ),
        (
            Frame {
                guard: Gate::Unknown,
                ..frame
            },
            HoldReason::GuardUnknown,
        ),
        (
            Frame {
                quiet: Gate::Held,
                ..frame
            },
            HoldReason::QuietHeld,
        ),
        (
            Frame {
                quiet: Gate::Unknown,
                ..frame
            },
            HoldReason::QuietUnknown,
        ),
    ] {
        assert_eq!(
            apply(&mut queue, Input::TryGrant, frame),
            Effect::Held(reason)
        );
        assert_eq!(queue, before);
    }
    assert_eq!(
        apply(&mut Queue::default(), Input::TryGrant, frame),
        Effect::Held(HoldReason::Empty)
    );
}

#[test]
fn unknown_work_counts_against_global_and_nix_limits_until_proven_ended() {
    let mut queue = Queue::default();
    let frame = Frame::clear(2, 1);
    let lost = enqueue(&mut queue, SlotClass::Nix, frame);
    let waiting_nix = enqueue(&mut queue, SlotClass::Nix, frame);
    let heavy = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(lost)
    );
    for _ in 0..2 {
        assert_eq!(
            apply(&mut queue, Input::WrapperLost(lost), frame),
            Effect::Unknown(lost)
        );
    }
    assert_eq!(
        apply(&mut queue, Input::Cancel(lost), frame),
        Effect::Refused(Refusal::AlreadyGranted(lost))
    );
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(heavy)
    );
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Held(HoldReason::Capacity)
    );
    apply(&mut queue, Input::ProvenEnded(heavy), frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Held(HoldReason::NixCapacity)
    );
    apply(&mut queue, Input::ProvenEnded(lost), frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(waiting_nix)
    );
}

#[test]
fn reducing_capacity_never_preempts_and_only_holds_the_affected_class() {
    let mut queue = Queue::default();
    let mut frame = Frame::clear(3, 2);
    let first = enqueue(&mut queue, SlotClass::Nix, frame);
    let second = enqueue(&mut queue, SlotClass::Nix, frame);
    let heavy = enqueue(&mut queue, SlotClass::Heavy, frame);
    let next_heavy = enqueue(&mut queue, SlotClass::Heavy, frame);
    apply(&mut queue, Input::TryGrant, frame);
    apply(&mut queue, Input::TryGrant, frame);
    frame.nix_max_running = 1;
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(heavy)
    );
    frame.count = 1;
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Held(HoldReason::Capacity)
    );
    apply(&mut queue, Input::ProvenEnded(first), frame);
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Held(HoldReason::Capacity)
    );
    frame.count = 3;
    assert_eq!(
        apply(&mut queue, Input::TryGrant, frame),
        Effect::Granted(next_heavy)
    );
    assert!(queue.tickets.contains_key(&second));
}

#[test]
fn unsupported_class_exhaustion_and_invalid_operations_are_typed_refusals() {
    let mut queue = Queue {
        last_sequence: u64::MAX - 1,
        ..Queue::default()
    };
    let frame = Frame::clear(1, 1);
    assert_eq!(
        apply(&mut queue, Input::Enqueue(SlotClass::Priority), frame),
        Effect::Refused(Refusal::UnsupportedClass(SlotClass::Priority))
    );
    let last = enqueue(&mut queue, SlotClass::Heavy, frame);
    assert_eq!(last, u64::MAX);
    let before = queue.clone();
    for (input, refusal) in [
        (Input::Enqueue(SlotClass::Heavy), Refusal::SequenceExhausted),
        (Input::Cancel(0), Refusal::MissingTicket(0)),
        (Input::WrapperLost(last), Refusal::NotGranted(last)),
        (Input::ProvenEnded(last), Refusal::NotGranted(last)),
    ] {
        assert_eq!(apply(&mut queue, input, frame), Effect::Refused(refusal));
        assert_eq!(queue, before);
    }
}

struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((value ^ (value >> 31)) % bound as u64) as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Step {
    input: Input,
    frame: Frame,
    effect: Effect,
}

#[derive(Debug, PartialEq, Eq)]
struct Run {
    trace: Vec<Step>,
    active_end: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct Failure {
    seed: u64,
    property: &'static str,
    trace: Vec<Step>,
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    Capacity,
    Fifo,
}

// Mutate both decision and state as a broken transition would, never production code.
fn plant(queue: &mut Queue, effect: Effect, frame: Frame, fault: Fault) -> Effect {
    match (fault, &effect) {
        (Fault::Capacity, Effect::Held(HoldReason::Capacity)) => {
            if let Some((&sequence, ticket)) = queue.tickets.iter_mut().find(|(_, ticket)| {
                ticket.phase == Phase::Queued && ticket.class == SlotClass::Heavy
            }) {
                ticket.phase = Phase::Held {
                    granted_at: frame.now,
                };
                return Effect::Granted(sequence);
            }
        }
        (Fault::Fifo, Effect::Granted(sequence)) => {
            let class = queue.tickets[sequence].class;
            let younger = queue.tickets.iter().find_map(|(&other, ticket)| {
                (other > *sequence && ticket.phase == Phase::Queued && ticket.class == class)
                    .then_some(other)
            });
            if let Some(younger) = younger {
                queue.tickets.get_mut(sequence).unwrap().phase = Phase::Queued;
                queue.tickets.get_mut(&younger).unwrap().phase = Phase::Held {
                    granted_at: frame.now,
                };
                return Effect::Granted(younger);
            }
        }
        _ => {}
    }
    effect
}

#[derive(Default)]
struct Oracle {
    last_sequence: u64,
    // The oracle follows the operation log in registration order, not core selection.
    records: Vec<(u64, SlotClass, Phase)>,
}

impl Oracle {
    fn check(&mut self, step: &Step, queue: &Queue) -> Result<(), &'static str> {
        let Step {
            input,
            frame,
            effect,
        } = step;
        let holders = self
            .records
            .iter()
            .filter(|record| record.2 != Phase::Queued)
            .count();
        let nix = self
            .records
            .iter()
            .filter(|record| record.1 == SlotClass::Nix && record.2 != Phase::Queued)
            .count();
        match effect {
            Effect::Enqueued(sequence) => {
                let Input::Enqueue(class) = input else {
                    return Err("enqueue effect");
                };
                if *class == SlotClass::Priority || *sequence <= self.last_sequence {
                    return Err("sequence");
                }
                self.last_sequence = *sequence;
                self.records.push((*sequence, *class, Phase::Queued));
            }
            Effect::Granted(sequence) => {
                if *input != Input::TryGrant
                    || frame.invalid_policy
                    || frame.guard != Gate::Clear
                    || frame.quiet != Gate::Clear
                {
                    return Err("admission gate");
                }
                let Some(index) = self
                    .records
                    .iter()
                    .position(|record| record.0 == *sequence && record.2 == Phase::Queued)
                else {
                    return Err("unique reservation");
                };
                let class = self.records[index].1;
                if self.records[..index]
                    .iter()
                    .any(|record| record.1 == class && record.2 == Phase::Queued)
                {
                    return Err("class FIFO");
                }
                if holders + 1 > frame.count as usize
                    || (class == SlotClass::Nix && nix + 1 > frame.nix_max_running as usize)
                {
                    return Err("capacity");
                }
                if self.records[..index].iter().any(|record| {
                    record.2 == Phase::Queued
                        && (record.1 == SlotClass::Heavy || nix < frame.nix_max_running as usize)
                }) {
                    return Err("oldest eligible head");
                }
                self.records[index].2 = Phase::Held {
                    granted_at: frame.now,
                };
            }
            Effect::Cancelled(sequence) | Effect::Released(sequence) => {
                let Some(index) = self.records.iter().position(|record| record.0 == *sequence)
                else {
                    return Err("missing removal");
                };
                let queued = self.records[index].2 == Phase::Queued;
                if !matches!((input, effect, queued), (Input::Cancel(id), Effect::Cancelled(_), true) | (Input::ProvenEnded(id), Effect::Released(_), false) if id == sequence)
                {
                    return Err("unsafe release");
                }
                self.records.remove(index);
            }
            Effect::Unknown(sequence) => {
                let Some(record) = self.records.iter_mut().find(|record| record.0 == *sequence)
                else {
                    return Err("missing unknown");
                };
                if *input != Input::WrapperLost(*sequence) {
                    return Err("unknown effect");
                }
                let (Phase::Held { granted_at } | Phase::Unknown { granted_at }) = record.2 else {
                    return Err("unknown waiter");
                };
                record.2 = Phase::Unknown { granted_at };
            }
            Effect::Held(_) => {
                if *input != Input::TryGrant {
                    return Err("hold effect");
                }
                if !frame.invalid_policy
                    && frame.guard == Gate::Clear
                    && frame.quiet == Gate::Clear
                    && holders < frame.count as usize
                    && self.records.iter().any(|record| {
                        record.2 == Phase::Queued
                            && (record.1 == SlotClass::Heavy
                                || nix < frame.nix_max_running as usize)
                    })
                {
                    return Err("progress");
                }
            }
            Effect::Refused(_) => {
                let valid = match input {
                    Input::Enqueue(class) => *class == SlotClass::Priority,
                    Input::TryGrant => false,
                    Input::Cancel(id) => !self
                        .records
                        .iter()
                        .any(|record| record.0 == *id && record.2 == Phase::Queued),
                    Input::WrapperLost(id) | Input::ProvenEnded(id) => !self
                        .records
                        .iter()
                        .any(|record| record.0 == *id && record.2 != Phase::Queued),
                };
                if !valid {
                    return Err("unexpected refusal");
                }
            }
        }
        if self.last_sequence != queue.last_sequence
            || self.records.len() != queue.tickets.len()
            || self.records.iter().any(|(id, class, phase)| {
                !queue
                    .tickets
                    .get(id)
                    .is_some_and(|ticket| ticket.class == *class && ticket.phase == *phase)
            })
        {
            return Err("state/effect agreement");
        }
        Ok(())
    }
}

fn execute(
    queue: &mut Queue,
    oracle: &mut Oracle,
    trace: &mut Vec<Step>,
    seed: u64,
    input: Input,
    frame: Frame,
    fault: Fault,
) -> Result<(), Failure> {
    let effect = apply(queue, input, frame);
    let effect = plant(queue, effect, frame, fault);
    trace.push(Step {
        input,
        frame,
        effect,
    });
    oracle
        .check(trace.last().unwrap(), queue)
        .map_err(|property| Failure {
            seed,
            property,
            trace: trace.clone(),
        })
}

fn request(random: &mut Random, queue: &Queue) -> Input {
    let kind = random.below(8);
    if kind <= 2 {
        return Input::Enqueue(
            [SlotClass::Heavy, SlotClass::Nix, SlotClass::Priority][random.below(3)],
        );
    }
    if kind >= 6 {
        return Input::TryGrant;
    }
    let live: Vec<_> = queue
        .tickets
        .iter()
        .filter_map(|(&id, ticket)| match (kind, ticket.phase) {
            (3, Phase::Queued)
            | (4, Phase::Held { .. })
            | (5, Phase::Held { .. } | Phase::Unknown { .. }) => Some(id),
            _ => None,
        })
        .collect();
    let id = if !live.is_empty() && random.below(4) != 0 {
        live[random.below(live.len())]
    } else {
        1 + random.below(queue.last_sequence as usize + 2) as u64
    };
    match kind {
        3 => Input::Cancel(id),
        4 => Input::WrapperLost(id),
        _ => Input::ProvenEnded(id),
    }
}

#[test]
fn requests_target_live_tickets_despite_long_finished_history() {
    let mut queue = Queue::default();
    let frame = Frame::clear(2, 1);
    let held = enqueue(&mut queue, SlotClass::Heavy, frame);
    let unknown = enqueue(&mut queue, SlotClass::Nix, frame);
    let queued = enqueue(&mut queue, SlotClass::Heavy, frame);
    apply(&mut queue, Input::TryGrant, frame);
    apply(&mut queue, Input::TryGrant, frame);
    apply(&mut queue, Input::WrapperLost(unknown), frame);
    queue.last_sequence = 10000;
    let mut random = Random(1);
    let mut totals = [0; 3];
    let mut hits = [0; 3];
    for _ in 0..1000 {
        let (kind, live) = match request(&mut random, &queue) {
            Input::Cancel(id) => (0, id == queued),
            Input::WrapperLost(id) => (1, id == held),
            Input::ProvenEnded(id) => (2, id == held || id == unknown),
            _ => continue,
        };
        totals[kind] += 1;
        hits[kind] += usize::from(live);
    }
    for kind in 0..3 {
        assert!(
            totals[kind] > 0 && hits[kind] * 2 > totals[kind],
            "kind {kind}: hits {hits:?}, totals {totals:?}"
        );
    }
}

#[test]
fn oracle_accepts_heavy_grants_while_nix_excess_drains() {
    let mut queue = Queue::default();
    let mut oracle = Oracle::default();
    let mut trace = Vec::new();
    let mut frame = Frame::clear(3, 2);
    for input in [
        Input::Enqueue(SlotClass::Nix),
        Input::Enqueue(SlotClass::Nix),
        Input::Enqueue(SlotClass::Heavy),
        Input::TryGrant,
        Input::TryGrant,
    ] {
        execute(
            &mut queue,
            &mut oracle,
            &mut trace,
            0,
            input,
            frame,
            Fault::None,
        )
        .unwrap();
    }
    frame.nix_max_running = 1;
    execute(
        &mut queue,
        &mut oracle,
        &mut trace,
        0,
        Input::TryGrant,
        frame,
        Fault::None,
    )
    .unwrap();
    assert_eq!(trace.last().unwrap().effect, Effect::Granted(3));
}

fn schedule(seed: u64, fault: Fault) -> Result<Run, Failure> {
    let mut random = Random(seed);
    let mut queue = Queue::default();
    let mut oracle = Oracle::default();
    let mut trace = Vec::new();
    let mut pending = Vec::new();
    let mut frame = Frame::clear(2, 1);
    // A small initial burst guarantees both planted controls encounter their trigger.
    for input in [Input::Enqueue(SlotClass::Heavy); 3]
        .into_iter()
        .chain([Input::TryGrant; 3])
    {
        frame.now += random.below(5) as u64;
        execute(
            &mut queue,
            &mut oracle,
            &mut trace,
            seed,
            input,
            frame,
            fault,
        )?;
    }
    for _ in 0..256 {
        frame.now = frame.now.saturating_add_signed(random.below(11) as i64 - 3);
        if random.below(12) == 0 {
            frame.count = 1 + random.below(3) as u16;
            frame.nix_max_running = 1 + random.below(frame.count as usize) as u16;
        }
        frame.invalid_policy = random.below(8) == 0;
        for gate in [&mut frame.guard, &mut frame.quiet] {
            *gate = match random.below(8) {
                0 => Gate::Held,
                1 => Gate::Unknown,
                _ => Gate::Clear,
            };
        }
        pending.push(request(&mut random, &queue));
        // A contended admission lock delays a request; registration happens on completion.
        if random.below(4) != 0 {
            for _ in 0..(1 + random.below(3)).min(pending.len()) {
                let input = pending.remove(random.below(pending.len()));
                execute(
                    &mut queue,
                    &mut oracle,
                    &mut trace,
                    seed,
                    input,
                    frame,
                    fault,
                )?;
            }
        }
    }
    while !pending.is_empty() {
        let input = pending.remove(random.below(pending.len()));
        execute(
            &mut queue,
            &mut oracle,
            &mut trace,
            seed,
            input,
            frame,
            fault,
        )?;
    }
    let active_end = trace.len();
    frame = Frame {
        now: frame.now + 1,
        ..Frame::clear(2, 1)
    };
    // Holders eventually end and all observations clear: finite queued work must drain.
    for _ in 0..1024 {
        if queue.tickets.is_empty() {
            return Ok(Run { trace, active_end });
        }
        let holder = queue
            .tickets
            .iter()
            .find_map(|(&id, ticket)| (ticket.phase != Phase::Queued).then_some(id));
        let input = holder.map_or(Input::TryGrant, Input::ProvenEnded);
        execute(
            &mut queue,
            &mut oracle,
            &mut trace,
            seed,
            input,
            frame,
            fault,
        )?;
    }
    Err(Failure {
        seed,
        property: "finite drain",
        trace,
    })
}

#[test]
fn seeded_schedules_preserve_fifo_capacity_and_replay() {
    for seed in 0..64 {
        let first = schedule(seed, Fault::None).unwrap_or_else(|failure| panic!("{failure:#?}"));
        let backwards = first.trace[..first.active_end]
            .windows(2)
            .filter(|steps| steps[1].frame.now < steps[0].frame.now)
            .count();
        assert!(backwards > 0, "seed {seed} never moved time backwards");
        let active = effect_counts(&first.trace[..first.active_end]);
        assert!(
            active[1] > 2 && active[3] > 0 && active[4] > 0,
            "seed {seed} lacks active grant/loss/completion coverage: {active:?}"
        );
        eprintln!(
            "seed={seed} active[e,g,c,u,r,h,f]={:?} drain[e,g,c,u,r,h,f]={:?} backwards={backwards}",
            effect_counts(&first.trace[..first.active_end]),
            effect_counts(&first.trace[first.active_end..])
        );
        assert_eq!(schedule(seed, Fault::None).unwrap(), first, "seed {seed}");
    }
}

fn effect_counts(trace: &[Step]) -> [usize; 7] {
    let mut counts = [0; 7];
    for step in trace {
        counts[match step.effect {
            Effect::Enqueued(_) => 0,
            Effect::Granted(_) => 1,
            Effect::Cancelled(_) => 2,
            Effect::Unknown(_) => 3,
            Effect::Released(_) => 4,
            Effect::Held(_) => 5,
            Effect::Refused(_) => 6,
        }] += 1;
    }
    counts
}

#[test]
fn planted_capacity_and_fifo_bugs_are_caught_and_the_failure_replays() {
    for (fault, property) in [(Fault::Capacity, "capacity"), (Fault::Fifo, "class FIFO")] {
        let failure =
            schedule(1, fault).expect_err("the oracle must catch the planted transition bug");
        assert_eq!(failure.property, property, "{failure:#?}");
        assert_eq!(schedule(1, fault).unwrap_err(), failure);
        eprintln!("planted {property}: {failure:#?}");
    }
}
