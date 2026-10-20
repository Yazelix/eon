# Role admission of ORB-C3: at most one interactive client and one metadata
# observer. Rust decodes each peer's first message and carries out each answer;
# these pure functions decide every answer and are proved in PROOF.bend.

type Slots is Data:
  Slots{interactive: Bool, observer: Bool}

type Signal is Data:
  # A new peer's first message.
  Hello{}
  Observe{}
  Malformed{}
  Unsupported{}
  # The attached interactive client sent a role again.
  Repeat{}
  # The interactive client or the observer is gone.
  Left{}
  Unobserved{}
  # A connection arrived while another peer negotiates its role.
  Crowded{}
  # The metadata observer sent a message, which it may not.
  Spoke{}

type Answer is Data:
  Admit{}
  Observing{}
  Busy{}
  Reject{}
  Refuse{}
  Drop{}
  Quiet{}
  # Busy to the arriving connection; the negotiating peer keeps its turn.
  TurnAway{}
  # Failure(Protocol) to the observer, which then closes.
  Expel{}

# A role is admitted while its slot is free and is Busy otherwise. A repeated
# role ends the interactive attachment with Failure(Protocol), and a malformed
# first message gets Failure(Protocol); an unsupported version gets nothing.
# A connection that arrives while another peer negotiates is turned away with
# Busy at once, and an observer that sends a message ends with
# Failure(Protocol), as in the old Orbit.
def decide(signal: Signal, interactive: Bool, observer: Bool) -> Answer:
  match signal:
    case Hello{}:
      match interactive:
        case True{}:
          Busy{}
        case False{}:
          Admit{}
    case Observe{}:
      match observer:
        case True{}:
          Busy{}
        case False{}:
          Observing{}
    case Malformed{}:
      Refuse{}
    case Unsupported{}:
      Drop{}
    case Repeat{}:
      match interactive:
        case True{}:
          Reject{}
        case False{}:
          Quiet{}
    case Left{}:
      Quiet{}
    case Unobserved{}:
      Quiet{}
    case Crowded{}:
      TurnAway{}
    case Spoke{}:
      match observer:
        case True{}:
          Expel{}
        case False{}:
          Quiet{}

def answer(slots: Slots, signal: Signal) -> Answer:
  match slots:
    case Slots{interactive, observer}:
      decide(signal, interactive, observer)

def next(slots: Slots, signal: Signal) -> Slots:
  match slots:
    case Slots{interactive, observer}:
      match signal:
        case Hello{}:
          Slots{True{}, observer}
        case Observe{}:
          Slots{interactive, True{}}
        case Malformed{}:
          Slots{interactive, observer}
        case Unsupported{}:
          Slots{interactive, observer}
        case Repeat{}:
          Slots{False{}, observer}
        case Left{}:
          Slots{False{}, observer}
        case Unobserved{}:
          Slots{interactive, False{}}
        case Crowded{}:
          Slots{interactive, observer}
        case Spoke{}:
          Slots{interactive, False{}}

def replay(events: List<&2, Signal>, slots: Slots) -> Slots:
  match events:
    case Nil{}:
      slots
    case Con{event, rest}:
      replay(rest, next(slots, event))

# The specification the answers must satisfy, kept apart from the policy: a
# role is admitted only into a free slot, Busy only when the asked slot is
# taken, a valid request on a free slot is never turned away, a reject only
# ends an attached client, a bad first message is refused or dropped, and a
# connection that arrives during another's negotiation is turned away without
# changing either slot, and only an observer is expelled for a message, which
# frees its slot.
type Audit is Data:
  Clean{slots: Slots}
  Broken{}

def audit_slots(signal: Signal, answer: Answer, interactive: Bool, observer: Bool) -> Audit:
  match signal answer interactive observer:
    case Hello{} Admit{} False{} _:
      Clean{Slots{True{}, observer}}
    case Hello{} Busy{} True{} _:
      Clean{Slots{True{}, observer}}
    case Observe{} Observing{} _ False{}:
      Clean{Slots{interactive, True{}}}
    case Observe{} Busy{} _ True{}:
      Clean{Slots{interactive, True{}}}
    case Malformed{} Refuse{} _ _:
      Clean{Slots{interactive, observer}}
    case Unsupported{} Drop{} _ _:
      Clean{Slots{interactive, observer}}
    case Repeat{} Reject{} True{} _:
      Clean{Slots{False{}, observer}}
    case Repeat{} Quiet{} False{} _:
      Clean{Slots{False{}, observer}}
    case Left{} Quiet{} _ _:
      Clean{Slots{False{}, observer}}
    case Unobserved{} Quiet{} _ _:
      Clean{Slots{interactive, False{}}}
    case Crowded{} TurnAway{} _ _:
      Clean{Slots{interactive, observer}}
    case Spoke{} Expel{} _ True{}:
      Clean{Slots{interactive, False{}}}
    case Spoke{} Quiet{} _ False{}:
      Clean{Slots{interactive, False{}}}
    case _ _ _ _:
      Broken{}

def audit(check: Audit, signal: Signal, answer: Answer) -> Audit:
  match check:
    case Broken{}:
      Broken{}
    case Clean{slots}:
      match slots:
        case Slots{interactive, observer}:
          audit_slots(signal, answer, interactive, observer)

def run(events: List<&2, Signal>, +slots: Slots, check: Audit) -> Audit:
  match events:
    case Nil{}:
      check
    case Con{+event, rest}:
      run(rest, next(slots, event), audit(check, event, answer(slots, event)))
