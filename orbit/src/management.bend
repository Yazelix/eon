# The management lease of ORB-C13: one management connection at a time, one
# validated lease, and Stop and ORB-C15's ProgramStatus only through it. Rust
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
  # A request that failed to decode, passed its bound, or was pipelined.
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
  # Stop the session through ORB-C12 and answer Stopped.
  Shutdown{}
  Quiet{}

# A request is admitted only in the state it belongs to: Acquire while
# negotiating, Status, ProgramStatus and Stop under the lease. ProgramStatus
# is answered while the child runs and refused Busy after; it never waits.
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
# lease while the child runs and refused Busy under it after, an invalid
# identity or a request out of place is refused and holds the slot until it
# closes, a refused connection is answered nothing more, and nothing is
# answered without a connection.
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

def audit_slot(signal: Signal, slot: Slot, answer: Answer) -> Audit:
  match signal:
    case Acquire{valid}:
      audit_acquire(valid, slot, answer)
    case ProgramStatus{live}:
      audit_program(live, slot, answer)
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
