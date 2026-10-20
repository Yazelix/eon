# The vertical request slot of ORB-C8. Rust reports facts and carries out each
# answer; these pure functions decide every answer and are proved in PROOF.bend.

type Slot is Data:
  Empty{}
  Waiting{}

type Signal is Data:
  Request{future: Bool, routed: Bool, held: Bool}
  Publish{routed: Bool, held: Bool}
  Detach{}

type Answer is Data:
  Reply{}
  Defer{}
  Resolve{}
  Discard{}
  RejectBusy{}
  RejectAhead{}
  Quiet{}

def slot_of(occupied: Bool) -> Slot:
  match occupied:
    case True{}:
      Waiting{}
    case False{}:
      Empty{}

def occupied(slot: Slot) -> Bool:
  match slot:
    case Empty{}:
      False{}
    case Waiting{}:
      True{}

# A future revision fails first, a terminal-routed request is answered at
# once, and otherwise one request may wait for a synchronized update to end.
def request(future: Bool, routed: Bool, slot: Slot, held: Bool) -> Answer:
  match future:
    case True{}:
      RejectAhead{}
    case False{}:
      match routed:
        case True{}:
          Reply{}
        case False{}:
          match slot:
            case Empty{}:
              match held:
                case True{}:
                  Defer{}
                case False{}:
                  Reply{}
            case Waiting{}:
              RejectBusy{}

# A published frame resolves the waiting request unless the update is still held.
def publish(slot: Slot, routed: Bool, held: Bool) -> Answer:
  match slot:
    case Empty{}:
      Quiet{}
    case Waiting{}:
      match routed:
        case True{}:
          Resolve{}
        case False{}:
          match held:
            case True{}:
              Quiet{}
            case False{}:
              Resolve{}

def detach(slot: Slot) -> Answer:
  match slot:
    case Empty{}:
      Quiet{}
    case Waiting{}:
      Discard{}

def answer(slot: Slot, signal: Signal) -> Answer:
  match signal:
    case Request{future, routed, held}:
      request(future, routed, slot, held)
    case Publish{routed, held}:
      publish(slot, routed, held)
    case Detach{}:
      detach(slot)

def after(slot: Slot, answer: Answer) -> Slot:
  match answer:
    case Defer{}:
      Waiting{}
    case Resolve{}:
      Empty{}
    case Discard{}:
      Empty{}
    case _:
      slot

def next(+slot: Slot, signal: Signal) -> Slot:
  after(slot, answer(slot, signal))

def replay(events: List<&2, Signal>, slot: Slot) -> Slot:
  match events:
    case Nil{}:
      slot
    case Con{event, rest}:
      replay(rest, next(slot, event))

# The specification the answers must satisfy: one waiting request at most, a
# resolve or discard only for a waiting request, and Busy only while one waits.
type Audit is Data:
  Clean{occupied: Bool}
  Broken{}

def audit_clean(occupied: Bool, answer: Answer) -> Audit:
  match occupied answer:
    case False{} Defer{}:
      Clean{True{}}
    case True{} Defer{}:
      Broken{}
    case False{} Resolve{}:
      Broken{}
    case True{} Resolve{}:
      Clean{False{}}
    case False{} Discard{}:
      Broken{}
    case True{} Discard{}:
      Clean{False{}}
    case False{} RejectBusy{}:
      Broken{}
    case True{} RejectBusy{}:
      Clean{True{}}
    case False{} Reply{}:
      Clean{False{}}
    case True{} Reply{}:
      Clean{True{}}
    case False{} RejectAhead{}:
      Clean{False{}}
    case True{} RejectAhead{}:
      Clean{True{}}
    case False{} Quiet{}:
      Clean{False{}}
    case True{} Quiet{}:
      Clean{True{}}

def audit(check: Audit, answer: Answer) -> Audit:
  match check:
    case Broken{}:
      Broken{}
    case Clean{occupied}:
      audit_clean(occupied, answer)

def run(events: List<&2, Signal>, +slot: Slot, check: Audit) -> Audit:
  match events:
    case Nil{}:
      check
    case Con{+event, rest}:
      run(rest, next(slot, event), audit(check, answer(slot, event)))
