# Pointer sequences of ORB-C9: one left-pointer sequence at a time, routed when
# it begins to host selection or to the terminal, and kept on that route
# through Finish or Cancel. The worker's session (src/main.bend) stores the
# route, and Rust carries out each answer; these pure functions decide every
# answer and are proved in PROOF.bend.

type Route is Data:
  Idle{}
  Host{}
  Terminal{}

type Signal is Data:
  # A press from a presented revision: whether that revision is ahead of
  # authority, whether the terminal tracks the mouse, and whether Shift is held.
  Begin{future: Bool, tracking: Bool, shift: Bool}
  Update{}
  Finish{}
  Cancel{}
  # A request for the frozen copy, and whether one exists.
  Copy{copied: Bool}
  # The attachment ended, the surface resized, or the PTY closed.
  Reset{}

type Answer is Data:
  # Failure(InvalidInput): a future revision, a press during a sequence, or
  # an update or finish with none.
  RejectAhead{}
  RejectActive{}
  RejectIdle{}
  RejectNoCopy{}
  # Carry the phase out on the sequence's route.
  HostPhase{}
  TerminalPhase{}
  Cancelled{}
  Copied{}
  Quiet{}

# Host selection unless the terminal tracks the mouse and Shift is not held.
def route_of(shift: Bool, tracking: Bool) -> Route:
  match shift:
    case True{}:
      Host{}
    case False{}:
      match tracking:
        case True{}:
          Terminal{}
        case False{}:
          Host{}

def phase(route: Route) -> Answer:
  match route:
    case Idle{}:
      RejectIdle{}
    case Host{}:
      HostPhase{}
    case Terminal{}:
      TerminalPhase{}

def begin(future: Bool, route: Route, tracking: Bool, shift: Bool) -> Answer:
  match future:
    case True{}:
      RejectAhead{}
    case False{}:
      match route:
        case Idle{}:
          phase(route_of(shift, tracking))
        case Host{}:
          RejectActive{}
        case Terminal{}:
          RejectActive{}

def answer(route: Route, signal: Signal) -> Answer:
  match signal:
    case Begin{future, tracking, shift}:
      begin(future, route, tracking, shift)
    case Update{}:
      phase(route)
    case Finish{}:
      phase(route)
    case Cancel{}:
      Cancelled{}
    case Copy{copied}:
      match copied:
        case True{}:
          Copied{}
        case False{}:
          RejectNoCopy{}
    case Reset{}:
      Quiet{}

# The route after Rust carries out the answer.
def after(answer: Answer, signal: Signal, route: Route) -> Route:
  match answer:
    case HostPhase{}:
      match signal:
        case Finish{}:
          Idle{}
        case _:
          Host{}
    case TerminalPhase{}:
      match signal:
        case Finish{}:
          Idle{}
        case _:
          Terminal{}
    case Cancelled{}:
      Idle{}
    case Quiet{}:
      Idle{}
    case _:
      route

def next(+route: Route, +signal: Signal) -> Route:
  after(answer(route, signal), signal, route)

def replay(events: List<&2, Signal>, +route: Route) -> Route:
  match events:
    case Nil{}:
      route
    case Con{+event, rest}:
      replay(rest, next(route, event))

# The specification the answers must satisfy, kept apart from the policy: a
# phase runs only on the route its sequence began with, a sequence begins only
# from idle and from a presented revision, on the host whenever Shift is held
# or the terminal does not track the mouse, a press during a sequence and a
# phase with none are refused, and only an existing copy is returned.
type Audit is Data:
  Clean{route: Route}
  Broken{}

def audit_begin(
  route: Route, future: Bool, tracking: Bool, shift: Bool, answer: Answer
) -> Audit:
  match route future tracking shift answer:
    case _ True{} _ _ RejectAhead{}:
      Clean{route}
    case Host{} False{} _ _ RejectActive{}:
      Clean{Host{}}
    case Terminal{} False{} _ _ RejectActive{}:
      Clean{Terminal{}}
    case Idle{} False{} _ True{} HostPhase{}:
      Clean{Host{}}
    case Idle{} False{} False{} False{} HostPhase{}:
      Clean{Host{}}
    case Idle{} False{} True{} False{} TerminalPhase{}:
      Clean{Terminal{}}
    case _ _ _ _ _:
      Broken{}

def audit_route(signal: Signal, route: Route, answer: Answer) -> Audit:
  match signal:
    case Begin{future, tracking, shift}:
      audit_begin(route, future, tracking, shift, answer)
    case Update{}:
      match route answer:
        case Idle{} RejectIdle{}:
          Clean{Idle{}}
        case Host{} HostPhase{}:
          Clean{Host{}}
        case Terminal{} TerminalPhase{}:
          Clean{Terminal{}}
        case _ _:
          Broken{}
    case Finish{}:
      match route answer:
        case Idle{} RejectIdle{}:
          Clean{Idle{}}
        case Host{} HostPhase{}:
          Clean{Idle{}}
        case Terminal{} TerminalPhase{}:
          Clean{Idle{}}
        case _ _:
          Broken{}
    case Cancel{}:
      match answer:
        case Cancelled{}:
          Clean{Idle{}}
        case _:
          Broken{}
    case Copy{copied}:
      match copied answer:
        case True{} Copied{}:
          Clean{route}
        case False{} RejectNoCopy{}:
          Clean{route}
        case _ _:
          Broken{}
    case Reset{}:
      match answer:
        case Quiet{}:
          Clean{Idle{}}
        case _:
          Broken{}

def audit(check: Audit, signal: Signal, answer: Answer) -> Audit:
  match check:
    case Broken{}:
      Broken{}
    case Clean{route}:
      audit_route(signal, route, answer)

def run(events: List<&2, Signal>, +route: Route, check: Audit) -> Audit:
  match events:
    case Nil{}:
      check
    case Con{+event, rest}:
      run(rest, next(route, event), audit(check, event, answer(route, event)))
