# The management lease of ORB-C13: one management connection at a time, one
# validated lease, and Stop, ORB-C15's ProgramStatus, ORB-C17's ReadVisible,
# ORB-C5's lease input and ORB-C3's AttachAllow only through it. Rust
# owns the record, the management socket and every identity check, reports
# each event, and carries out each answer; these pure functions decide every
# answer and are proved in PROOF.bend.

type Slot is Data:
  Free{}
  # A connection that has not acquired the lease yet.
  Negotiating{}
  Leased{}
  # A refused connection, which keeps the slot until its Failure is written
  # and it closes.
  Closing{}

type Signal is Data:
  # A new management connection.
  Connect{}
  # An Acquire request, and whether its identity matched the live run.
  Acquire{valid: Bool}
  Status{}
  Stop{}
  # A ProgramStatus request (ORB-C15), and whether the child still runs: once
  # a stop has begun or the child exited, its records end with the run.
  ProgramStatus{live: Bool}
  # A ReadVisible request (ORB-C17), whether the child still runs, and
  # whether a synchronized update is held; the host reports it only once none
  # is.
  Read{live: Bool, held: Bool}
  # A lease input request (ORB-C5), and whether the child still runs.
  Input{live: Bool}
  # An AttachAllow request naming the process ORB-C3 may admit.
  Allow{}
  # A whole request that does not decode, or one sent while another was in
  # flight (ORB-C17).
  Invalid{}
  # A request that breaks framing: a bad header, or a payload over its kind's
  # bound.
  Bad{}
  # The connection closed, or its one-second negotiation bound passed.
  Gone{}

type Answer is Data:
  # Start the one-second negotiation bound.
  Admit{}
  # Answer Busy and close the new connection.
  Busy{}
  Lease{}
  # Failure(InvalidIdentity), then close.
  RejectIdentity{}
  # Failure(InvalidRequest), then close.
  RejectRequest{}
  Report{}
  # Answer ProgramStatus with the terminal's records.
  Records{}
  # Answer Busy on the lease, which stays.
  Refuse{}
  # Answer Visible with the visible text (ORB-C17).
  Visible{}
  # Write the lease input and answer Input (ORB-C5).
  Write{}
  # Record the named process and answer Allowed (ORB-C13).
  Name{}
  # Answer Refused(InvalidRequest) on the lease, which stays.
  Decline{}
  # Stop the session through ORB-C12 and answer Stopped.
  Shutdown{}
  Quiet{}

# A request outside the lease: refused while negotiating, and answered
# nothing without a connection or on a refused one.
def unleased(slot: Slot) -> Answer:
  match slot:
    case Leased{}:
      Quiet{}
    case Free{}:
      Quiet{}
    case Negotiating{}:
      RejectRequest{}
    case Closing{}:
      Quiet{}

# A request is admitted only in the state it belongs to: Acquire while
# negotiating, Status, ProgramStatus, ReadVisible, lease input, AttachAllow
# and Stop under the lease. ProgramStatus and lease input are answered while
# the child runs and refused Busy after, and ReadVisible also while a
# synchronized update is held; none waits. A request that does not decode, or
# a pipelined one, is declined and keeps the lease.
def decide(signal: Signal, slot: Slot) -> Answer:
  match signal:
    case Connect{}:
      match slot:
        case Free{}:
          Admit{}
        case Negotiating{}:
          Busy{}
        case Leased{}:
          Busy{}
        case Closing{}:
          Busy{}
    case Acquire{valid}:
      match valid:
        case True{}:
          match slot:
            case Negotiating{}:
              Lease{}
            case Free{}:
              Quiet{}
            case Leased{}:
              RejectRequest{}
            case Closing{}:
              Quiet{}
        case False{}:
          match slot:
            case Negotiating{}:
              RejectIdentity{}
            case Free{}:
              Quiet{}
            case Leased{}:
              RejectRequest{}
            case Closing{}:
              Quiet{}
    case Status{}:
      match slot:
        case Leased{}:
          Report{}
        case Free{}:
          Quiet{}
        case Negotiating{}:
          RejectRequest{}
        case Closing{}:
          Quiet{}
    case Stop{}:
      match slot:
        case Leased{}:
          Shutdown{}
        case Free{}:
          Quiet{}
        case Negotiating{}:
          RejectRequest{}
        case Closing{}:
          Quiet{}
    case ProgramStatus{live}:
      match live:
        case True{}:
          match slot:
            case Leased{}:
              Records{}
            case Free{}:
              Quiet{}
            case Negotiating{}:
              RejectRequest{}
            case Closing{}:
              Quiet{}
        case False{}:
          match slot:
            case Leased{}:
              Refuse{}
            case Free{}:
              Quiet{}
            case Negotiating{}:
              RejectRequest{}
            case Closing{}:
              Quiet{}
    case Read{live, held}:
      match live held slot:
        case True{} False{} Leased{}:
          Visible{}
        case _ _ Leased{}:
          Refuse{}
        case _ _ other:
          unleased(other)
    case Input{live}:
      match live slot:
        case True{} Leased{}:
          Write{}
        case _ Leased{}:
          Refuse{}
        case _ other:
          unleased(other)
    case Allow{}:
      match slot:
        case Leased{}:
          Name{}
        case other:
          unleased(other)
    case Invalid{}:
      match slot:
        case Leased{}:
          Decline{}
        case other:
          unleased(other)
    case Bad{}:
      match slot:
        case Free{}:
          Quiet{}
        case Negotiating{}:
          RejectRequest{}
        case Leased{}:
          RejectRequest{}
        case Closing{}:
          Quiet{}
    case Gone{}:
      Quiet{}

def answer(slot: Slot, signal: Signal) -> Answer:
  decide(signal, slot)

def is_shutdown(answer: Answer) -> Bool:
  match answer:
    case Shutdown{}:
      True{}
    case _:
      False{}

def is_records(answer: Answer) -> Bool:
  match answer:
    case Records{}:
      True{}
    case _:
      False{}

def is_text(answer: Answer) -> Bool:
  match answer:
    case Visible{}:
      True{}
    case _:
      False{}

def is_write(answer: Answer) -> Bool:
  match answer:
    case Write{}:
      True{}
    case _:
      False{}

def is_name(answer: Answer) -> Bool:
  match answer:
    case Name{}:
      True{}
    case _:
      False{}

def is_leased(slot: Slot) -> Bool:
  match slot:
    case Leased{}:
      True{}
    case _:
      False{}

# The slot after Rust carries out the answer: a refused connection holds the
# slot until it closes, a busy one never took the slot,
# and a stop keeps the lease until the run ends.
def after(answer: Answer, signal: Signal, slot: Slot) -> Slot:
  match answer:
    case Admit{}:
      Negotiating{}
    case Lease{}:
      Leased{}
    case RejectIdentity{}:
      Closing{}
    case RejectRequest{}:
      Closing{}
    case Busy{}:
      slot
    case Report{}:
      slot
    case Records{}:
      slot
    case Refuse{}:
      slot
    case Visible{}:
      slot
    case Write{}:
      slot
    case Name{}:
      slot
    case Decline{}:
      slot
    case Shutdown{}:
      slot
    case Quiet{}:
      match signal:
        case Gone{}:
          Free{}
        case _:
          slot

def next(+slot: Slot, +signal: Signal) -> Slot:
  after(answer(slot, signal), signal, slot)

def replay(events: List<&2, Signal>, +slot: Slot) -> Slot:
  match events:
    case Nil{}:
      slot
    case Con{+event, rest}:
      replay(rest, next(slot, event))

# The specification the answers must satisfy, kept apart from the policy: a
# connection is admitted only into a free slot and is Busy otherwise, a lease
# is granted only for a valid identity during negotiation, Status and Stop are
# carried out only under the lease, ProgramStatus is answered only under the
# lease while the child runs and refused Busy under it after, ReadVisible
# likewise and also refused while a synchronized update is held, lease input
# is written only under the lease while the child runs and refused Busy under
# it after, AttachAllow names a process only under the lease, a request that
# does not decode or was pipelined is declined under the lease and keeps it, an
# invalid identity or a request out of place is refused and holds the slot
# until it closes, a refused connection is answered nothing more, and nothing
# is answered without a connection.
type Audit is Data:
  Clean{slot: Slot}
  Broken{}

def audit_acquire(valid: Bool, slot: Slot, answer: Answer) -> Audit:
  match valid slot answer:
    case True{} Negotiating{} Lease{}:
      Clean{Leased{}}
    case False{} Negotiating{} RejectIdentity{}:
      Clean{Closing{}}
    case _ Leased{} RejectRequest{}:
      Clean{Closing{}}
    case _ Free{} Quiet{}:
      Clean{Free{}}
    case _ Closing{} Quiet{}:
      Clean{Closing{}}
    case _ _ _:
      Broken{}

def audit_program(live: Bool, slot: Slot, answer: Answer) -> Audit:
  match live slot answer:
    case True{} Leased{} Records{}:
      Clean{Leased{}}
    case False{} Leased{} Refuse{}:
      Clean{Leased{}}
    case _ Negotiating{} RejectRequest{}:
      Clean{Closing{}}
    case _ Free{} Quiet{}:
      Clean{Free{}}
    case _ Closing{} Quiet{}:
      Clean{Closing{}}
    case _ _ _:
      Broken{}

def audit_read(live: Bool, held: Bool, slot: Slot, answer: Answer) -> Audit:
  match live held slot answer:
    case True{} False{} Leased{} Visible{}:
      Clean{Leased{}}
    case _ True{} Leased{} Refuse{}:
      Clean{Leased{}}
    case False{} _ Leased{} Refuse{}:
      Clean{Leased{}}
    case _ _ Negotiating{} RejectRequest{}:
      Clean{Closing{}}
    case _ _ Free{} Quiet{}:
      Clean{Free{}}
    case _ _ Closing{} Quiet{}:
      Clean{Closing{}}
    case _ _ _ _:
      Broken{}

def audit_input(live: Bool, slot: Slot, answer: Answer) -> Audit:
  match live slot answer:
    case True{} Leased{} Write{}:
      Clean{Leased{}}
    case False{} Leased{} Refuse{}:
      Clean{Leased{}}
    case _ Negotiating{} RejectRequest{}:
      Clean{Closing{}}
    case _ Free{} Quiet{}:
      Clean{Free{}}
    case _ Closing{} Quiet{}:
      Clean{Closing{}}
    case _ _ _:
      Broken{}

def audit_slot(signal: Signal, slot: Slot, answer: Answer) -> Audit:
  match signal:
    case Acquire{valid}:
      audit_acquire(valid, slot, answer)
    case ProgramStatus{live}:
      audit_program(live, slot, answer)
    case Read{live, held}:
      audit_read(live, held, slot, answer)
    case Input{live}:
      audit_input(live, slot, answer)
    case _:
      match signal slot answer:
        case Connect{} Free{} Admit{}:
          Clean{Negotiating{}}
        case Connect{} Negotiating{} Busy{}:
          Clean{Negotiating{}}
        case Connect{} Leased{} Busy{}:
          Clean{Leased{}}
        case Connect{} Closing{} Busy{}:
          Clean{Closing{}}
        case Status{} Leased{} Report{}:
          Clean{Leased{}}
        case Status{} Negotiating{} RejectRequest{}:
          Clean{Closing{}}
        case Stop{} Leased{} Shutdown{}:
          Clean{Leased{}}
        case Stop{} Negotiating{} RejectRequest{}:
          Clean{Closing{}}
        case Allow{} Leased{} Name{}:
          Clean{Leased{}}
        case Allow{} Negotiating{} RejectRequest{}:
          Clean{Closing{}}
        case Invalid{} Leased{} Decline{}:
          Clean{Leased{}}
        case Invalid{} Negotiating{} RejectRequest{}:
          Clean{Closing{}}
        case Bad{} Negotiating{} RejectRequest{}:
          Clean{Closing{}}
        case Bad{} Leased{} RejectRequest{}:
          Clean{Closing{}}
        case Gone{} _ Quiet{}:
          Clean{Free{}}
        case _ Free{} Quiet{}:
          Clean{Free{}}
        case _ Closing{} Quiet{}:
          Clean{Closing{}}
        case _ _ _:
          Broken{}

def audit(check: Audit, signal: Signal, answer: Answer) -> Audit:
  match check:
    case Broken{}:
      Broken{}
    case Clean{slot}:
      audit_slot(signal, slot, answer)

def run(events: List<&2, Signal>, +slot: Slot, check: Audit) -> Audit:
  match events:
    case Nil{}:
      check
    case Con{+event, rest}:
      run(rest, next(slot, event), audit(check, event, answer(slot, event)))
