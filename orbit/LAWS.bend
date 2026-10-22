import Base
import ./src/lifecycle.bend as Orbit
import ./src/vertical.bend as Vertical
import ./src/admission.bend as Admission
import ./src/selection.bend as Selection
import ./src/management.bend as Management
import ./src/revision.bend as Revision
import ./src/boundary.bend as Boundary
import ./src/session.bend as Session
import ./src/status.bend as Status

law detach_preserves_pid:
  for pid: U32
  {Orbit.detach(Orbit.Attached{pid}) == Orbit.Detached{pid} : Orbit.Lifecycle}

law reconnect_preserves_pid:
  for pid: U32
  {Orbit.attach(Orbit.detach(Orbit.Attached{pid})) == Orbit.Attached{pid} : Orbit.Lifecycle}

law bounded_stop_preserves_pid:
  for pid: U32
  {Orbit.finish_stop(Orbit.begin_stop(Orbit.Attached{pid})) == Orbit.Stopped{pid} : Orbit.Lifecycle}

law pre_reap_events_preserve_stopped:
  for events: List<&2, Orbit.BeforeReap>
  for +state: Orbit.Lifecycle
  {Orbit.is_stopped(Orbit.replay(events, state)) == Orbit.is_stopped(state) : Bool}

law vertical_answers_are_accounted:
  for events: List<&2, Vertical.Signal>
  for +slot: Vertical.Slot
  {Vertical.run(events, slot, Vertical.Clean{Vertical.occupied(slot)}) == Vertical.Clean{Vertical.occupied(Vertical.replay(events, slot))} : Vertical.Audit}

law vertical_future_requests_fail:
  for slot: Vertical.Slot
  for routed: Bool
  for held: Bool
  {Vertical.answer(slot, Vertical.Request{True{}, routed, held}) == Vertical.RejectAhead{} : Vertical.Answer}

law vertical_routed_requests_answer_at_once:
  for slot: Vertical.Slot
  for held: Bool
  {Vertical.answer(slot, Vertical.Request{False{}, True{}, held}) == Vertical.Reply{} : Vertical.Answer}

law vertical_nothing_resolves_while_held:
  for slot: Vertical.Slot
  {Vertical.answer(slot, Vertical.Publish{False{}, True{}}) == Vertical.Quiet{} : Vertical.Answer}

law vertical_release_resolves_waiting:
  for routed: Bool
  {Vertical.answer(Vertical.Waiting{}, Vertical.Publish{routed, False{}}) == Vertical.Resolve{} : Vertical.Answer}

law admission_answers_are_accounted:
  for events: List<&2, Admission.Signal>
  for +slots: Admission.Slots
  {Admission.run(events, slots, Admission.Clean{slots}) == Admission.Clean{Admission.replay(events, slots)} : Admission.Audit}

law selection_answers_are_accounted:
  for events: List<&2, Selection.Signal>
  for +route: Selection.Route
  {Selection.run(events, route, Selection.Clean{route}) == Selection.Clean{Selection.replay(events, route)} : Selection.Audit}

law selection_shift_selects_on_the_host:
  for tracking: Bool
  {Selection.answer(Selection.Idle{}, Selection.Begin{False{}, tracking, True{}}) == Selection.HostPhase{} : Selection.Answer}

law selection_future_press_fails:
  for route: Selection.Route
  for tracking: Bool
  for shift: Bool
  {Selection.answer(route, Selection.Begin{True{}, tracking, shift}) == Selection.RejectAhead{} : Selection.Answer}

law management_answers_are_accounted:
  for events: List<&2, Management.Signal>
  for +slot: Management.Slot
  {Management.run(events, slot, Management.Clean{slot}) == Management.Clean{Management.replay(events, slot)} : Management.Audit}

law management_stop_needs_the_lease:
  for slot: Management.Slot
  {Management.is_shutdown(Management.answer(slot, Management.Stop{})) == Management.is_leased(slot) : Bool}

# ORB-C15: a ProgramStatus request gets the records exactly under the lease
# while the child runs.
law management_program_status_needs_a_live_lease:
  for slot: Management.Slot
  for live: Bool
  {Management.is_records(Management.answer(slot, Management.ProgramStatus{live})) == Bool.and(Management.is_leased(slot), live) : Bool}

law revision_next_is_ahead:
  for r: Revision.Revision
  {Revision.ahead(Revision.next(r), r) == Bool.not(Revision.is_last(r)) : Bool}

# The program status lifetime of ORB-C15 (../eon/orbit/docs/CONTRACTS.md), as
# src/status.bend decides it for each PTY turn.

def marks_drop(decision: Boundary.Decision) -> Bool:
  match decision:
    case Boundary.StatusLifetime{edge, marks}:
      marks
    case _:
      False{}

def edge_drops(decision: Boundary.Decision) -> Bool:
  match decision:
    case Boundary.StatusLifetime{edge, marks}:
      edge
    case _:
      False{}

def returns(foreground: Boundary.Foreground) -> Bool:
  match foreground:
    case Boundary.ForegroundChild{}:
      True{}
    case _:
      False{}

# One PTY turn as the policy hears it.
type Turn is Data:
  Turn{foreground: Boundary.Foreground, marks: Bool}

# Whether any of the turns drops at its foreground sample.
def edges(turns: List<&2, Turn>, +tracker: Status.Tracker) -> Bool:
  match turns:
    case Nil{}:
      False{}
    case Con{turn, rest}:
      match turn:
        case Turn{+foreground, +marks}:
          Bool.or(
            edge_drops(Status.lifetime(tracker, foreground, marks)),
            edges(rest, Status.after(tracker, foreground, marks))
          )

# A turn's prompt marks always drop the working and blocked records; a turn
# without one drops nothing at a mark.
law status_prompt_marks_drop:
  for tracker: Status.Tracker
  for foreground: Boundary.Foreground
  for marks: Bool
  {marks_drop(Status.lifetime(tracker, foreground, marks)) == marks : Bool}

# A turn drops them at its foreground sample exactly when the terminal has
# never sent a prompt mark and the sample finds the child's own group while
# the last sample that found a group found another.
law status_foreground_returns_drop:
  for marked: Bool
  for child: Bool
  for foreground: Boundary.Foreground
  for marks: Bool
  {edge_drops(Status.lifetime(Status.Tracker{marked, child}, foreground, marks)) == Bool.and(Bool.not(marked), Bool.and(Bool.not(child), returns(foreground))) : Bool}

# The drop is edge-triggered: a sample that finds the child's group right
# after one that found it drops nothing.
law status_foreground_drops_once:
  for tracker: Status.Tracker
  for first: Bool
  for marks: Bool
  {edge_drops(Status.lifetime(Status.after(tracker, Boundary.ForegroundChild{}, first), Boundary.ForegroundChild{}, marks)) == False{} : Bool}

# A sample that finds no group keeps the last sample, and a prompt mark sets
# the latch.
law status_unknown_samples_keep_the_last:
  for marked: Bool
  for child: Bool
  for marks: Bool
  {Status.after(Status.Tracker{marked, child}, Boundary.ForegroundUnknown{}, marks) == Status.Tracker{Bool.or(marked, marks), child} : Status.Tracker}

# After a turn with a prompt mark, no later turn of the run drops at its
# foreground sample.
law status_prompt_marks_end_foreground_drops:
  for turns: List<&2, Turn>
  for tracker: Status.Tracker
  for foreground: Boundary.Foreground
  {edges(turns, Status.after(tracker, foreground, True{})) == False{} : Bool}

# The lifecycle automaton of ORB-C1, C3 and C12 (../eon/orbit/docs/CONTRACTS.md),
# checked against step's decisions and kept apart from the policy.

# The decisions these contracts constrain. Step's other decisions are outside
# them, and the audit ignores them.
type Move is Data:
  Attached{}
  Observed{}
  Busied{}
  RefusedPeer{}
  DroppedPeer{}
  TurnedAway{}
  Expelled{}
  Unobserved{}
  # Detach or Dismiss: the attachment ends.
  Ended{}
  Reaping{}
  StopBegun{}
  Finishing{status: U32}

def move(decision: Boundary.Decision, rest: List<&2, Move>) -> List<&2, Move>:
  match decision:
    case Boundary.Attach{}:
      Attached{} <> rest
    case Boundary.Observing{}:
      Observed{} <> rest
    case Boundary.Busy{}:
      Busied{} <> rest
    case Boundary.Refuse{}:
      RefusedPeer{} <> rest
    case Boundary.Drop{}:
      DroppedPeer{} <> rest
    case Boundary.TurnAway{}:
      TurnedAway{} <> rest
    case Boundary.RefuseObserver{}:
      Expelled{} <> rest
    case Boundary.DropObserver{}:
      Unobserved{} <> rest
    case Boundary.Detach{}:
      Ended{} <> rest
    case Boundary.Dismiss{}:
      Ended{} <> rest
    case Boundary.Reap{}:
      Reaping{} <> rest
    case Boundary.ManageStop{}:
      StopBegun{} <> rest
    case Boundary.Finish{status, explicit}:
      Finishing{status} <> rest
    case _:
      rest

def moves(decisions: List<&2, Boundary.Decision>) -> List<&2, Move>:
  match decisions:
    case Nil{}:
      Nil{}
    case Con{decision, rest}:
      move(decision, moves(rest))

type Phase is Data:
  # The child runs; whether a client and an observer are attached.
  Live{client: Bool, observer: Bool}
  # A stop began and waits for the host to reap the child.
  Stopping{}
  # The run finished.
  Stopped{}
  # The host sent a fact its protocol rules out in this phase; nothing after
  # it is checked.
  Outside{}
  # A decision broke the contracts.
  Broken{}

# ORB-C3 admission, as src/admission.bend's audit states it: the peer's
# moves read as one admission answer.
def answer_of(first: Move, rest: List<&2, Move>) -> Maybe<&2, Admission.Answer>:
  match first rest:
    case Attached{} Nil{}:
      Some{Admission.Admit{}}
    case Observed{} Nil{}:
      Some{Admission.Observing{}}
    case Busied{} Nil{}:
      Some{Admission.Busy{}}
    case RefusedPeer{} Nil{}:
      Some{Admission.Refuse{}}
    case DroppedPeer{} Nil{}:
      Some{Admission.Drop{}}
    case TurnedAway{} Nil{}:
      Some{Admission.TurnAway{}}
    case Expelled{} Nil{}:
      Some{Admission.Expel{}}
    case Ended{} Nil{}:
      Some{Admission.Reject{}}
    case _ _:
      None{}

def as_answer(moves: List<&2, Move>) -> Maybe<&2, Admission.Answer>:
  match moves:
    case Nil{}:
      Some{Admission.Quiet{}}
    case Con{first, rest}:
      answer_of(first, rest)

def admitted(check: Admission.Audit) -> Phase:
  match check:
    case Admission.Clean{slots}:
      match slots:
        case Admission.Slots{client, observer}:
          Live{client, observer}
    case Admission.Broken{}:
      Broken{}

def admission(
  signal: Admission.Signal, answer: Maybe<&2, Admission.Answer>, client: Bool, observer: Bool
) -> Phase:
  match answer:
    case Some{given}:
      admitted(Admission.audit_slots(signal, given, client, observer))
    case None{}:
      Broken{}

def peer_signal(message: Boundary.FirstMessage) -> Admission.Signal:
  match message:
    case Boundary.FirstMessageHello{}:
      Admission.Hello{}
    case Boundary.FirstMessageObserveMetadata{}:
      Admission.Observe{}
    case Boundary.FirstMessageRefused{}:
      Admission.Malformed{}
    case Boundary.FirstMessageUnsupported{}:
      Admission.Unsupported{}

# A fact that changes no role and begins no stop.
def quiet(moves: List<&2, Move>, phase: Phase) -> Phase:
  match moves:
    case Nil{}:
      phase
    case _:
      Broken{}

def only(moves: List<&2, Move>, expected: Move, phase: Phase) -> Phase:
  match moves:
    case Con{first, rest}:
      match first expected:
        case Unobserved{} Unobserved{}:
          quiet(rest, phase)
        case StopBegun{} StopBegun{}:
          quiet(rest, phase)
        case _ _:
          Broken{}
    case Nil{}:
      Broken{}

# ORB-C1: a fact from the attached client may end the attachment and does
# nothing else to the lifecycle; the child keeps running.
def may_end(moves: List<&2, Move>, observer: Bool) -> Phase:
  match moves:
    case Nil{}:
      Live{True{}, observer}
    case Con{first, rest}:
      match first rest:
        case Ended{} Nil{}:
          Live{False{}, observer}
        case _ _:
          Broken{}

# ORB-C1: a client that closes is detached, not stopped.
def must_end(moves: List<&2, Move>, observer: Bool) -> Phase:
  match moves:
    case Con{first, rest}:
      match first rest:
        case Ended{} Nil{}:
          Live{False{}, observer}
        case _ _:
          Broken{}
    case Nil{}:
      Broken{}

# Facts the host sends only from the attached client.
def from_client(moves: List<&2, Move>, client: Bool, observer: Bool) -> Phase:
  match client:
    case True{}:
      may_end(moves, observer)
    case False{}:
      Outside{}

# ORB-C12: the child's exit begins a stop that waits on the reap, after
# ending the attachment if its last output overflowed the clipboard bound.
def reaping(moves: List<&2, Move>) -> Phase:
  match moves:
    case Con{first, rest}:
      match first rest:
        case Reaping{} Nil{}:
          Stopping{}
        case _ _:
          Broken{}
    case Nil{}:
      Broken{}

def exit_begun(moves: List<&2, Move>, client: Bool) -> Phase:
  match moves:
    case Con{first, rest}:
      match first client:
        case Reaping{} _:
          quiet(rest, Stopping{})
        case Ended{} True{}:
          reaping(rest)
        case _ _:
          Broken{}
    case Nil{}:
      Broken{}

# ORB-C12: a management Stop may begin the stop (the lease decides whether,
# ORB-C13); no other management event does.
def managed(event: Boundary.ManagementEvent, moves: List<&2, Move>, phase: Phase) -> Phase:
  match event:
    case Boundary.ManagementEventStop{}:
      match moves:
        case Nil{}:
          phase
        case _:
          only(moves, StopBegun{}, Stopping{})
    case _:
      quiet(moves, phase)

def live(
  fact: Boundary.Fact, moves: List<&2, Move>, client: Bool, observer: Bool
) -> Phase:
  match fact:
    # ORB-C3, reusing the admission audit.
    case Boundary.Peer{message}:
      admission(peer_signal(message), as_answer(moves), client, observer)
    case Boundary.Crowded{}:
      admission(Admission.Crowded{}, as_answer(moves), client, observer)
    case Boundary.ObserverMessage{}:
      admission(Admission.Spoke{}, as_answer(moves), client, observer)
    case Boundary.RoleRepeated{}:
      admission(Admission.Repeat{}, as_answer(moves), client, observer)
    # A gone observer frees its slot; the host may report it more than once.
    case Boundary.ObserverGone{loss}:
      only(moves, Unobserved{}, Live{client, False{}})
    case Boundary.Closed{full}:
      match full client:
        case True{} True{}:
          must_end(moves, observer)
        case False{} True{}:
          may_end(moves, observer)
        case _ False{}:
          Outside{}
    case Boundary.Applied{effect}:
      from_client(moves, client, observer)
    case Boundary.Pressured{}:
      from_client(moves, client, observer)
    case Boundary.Refused{version}:
      from_client(moves, client, observer)
    case Boundary.PreviewVertical{revision, direction}:
      from_client(moves, client, observer)
    case Boundary.ScrollVertical{revision, rows}:
      from_client(moves, client, observer)
    case Boundary.ReturnToLive{}:
      from_client(moves, client, observer)
    case Boundary.Wheel{input}:
      from_client(moves, client, observer)
    case Boundary.Selection{kind, revision}:
      from_client(moves, client, observer)
    case Boundary.Selected{outcome, presented}:
      from_client(moves, client, observer)
    case Boundary.Tick{state, x, y}:
      from_client(moves, client, observer)
    # Output may end an attachment whose clipboard writes overflowed.
    case Boundary.Output{foreground, marks}:
      match client:
        case True{}:
          may_end(moves, observer)
        case False{}:
          quiet(moves, Live{False{}, observer})
    # ORB-C1: a closed PTY alone does not stop the session.
    case Boundary.PtyClosed{}:
      quiet(moves, Live{client, observer})
    case Boundary.Expired{}:
      quiet(moves, Live{client, observer})
    # The child is ready before any client attaches.
    case Boundary.Started{pid}:
      match client:
        case False{}:
          quiet(moves, Live{False{}, observer})
        case True{}:
          Outside{}
    case Boundary.ChildExited{drained}:
      exit_begun(moves, client)
    case Boundary.Management{event}:
      managed(event, moves, Live{client, observer})
    # ORB-C15: a ProgramStatus answer changes no lifecycle.
    case Boundary.ProgramStatus{foreground}:
      quiet(moves, Live{client, observer})
    # The host reports a reap only after Bend asks for one.
    case _:
      Outside{}

# ORB-C12: the run finishes only once the direct child is reaped, with its
# status; until then only an observer's loss and a tick or host phase that
# was already queued arrive, and none changes the lifecycle.
def same_status(equal: Bool, rest: List<&2, Move>) -> Phase:
  match equal:
    case True{}:
      quiet(rest, Stopped{})
    case False{}:
      Broken{}

def finished(moves: List<&2, Move>, status: U32) -> Phase:
  match moves:
    case Con{first, rest}:
      match first:
        case Finishing{code}:
          same_status(U32.is_eq(code, status), rest)
        case _:
          Broken{}
    case Nil{}:
      Broken{}

def stopping(fact: Boundary.Fact, moves: List<&2, Move>) -> Phase:
  match fact:
    case Boundary.Reaped{status}:
      finished(moves, status)
    case Boundary.ExitedBeforeStop{status}:
      finished(moves, status)
    case Boundary.ObserverGone{loss}:
      only(moves, Unobserved{}, Stopping{})
    case Boundary.Tick{state, x, y}:
      quiet(moves, Stopping{})
    case Boundary.Selected{outcome, presented}:
      quiet(moves, Stopping{})
    case _:
      Outside{}

def check(phase: Phase, fact: Boundary.Fact, moves: List<&2, Move>) -> Phase:
  match phase:
    case Live{client, observer}:
      live(fact, moves, client, observer)
    case Stopping{}:
      stopping(fact, moves)
    # The host sends nothing for Bend to decide after the finish.
    case Stopped{}:
      Outside{}
    case Outside{}:
      Outside{}
    case Broken{}:
      Broken{}

# The phase a session state starts the automaton in.
def phase(state: Session.State) -> Phase:
  match state:
    case Session.State{session, shown, status}:
      match session:
        case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
          match life:
            case Orbit.Detached{pid}:
              Live{False{}, observer}
            case Orbit.Attached{pid}:
              Live{True{}, observer}
            case Orbit.Stopping{pid}:
              Stopping{}
            case Orbit.Stopped{pid}:
              Stopped{}

def after(pair: Session.Stepped()) -> Session.State:
  (state, decisions) = pair
  state

def decided(pair: Session.Stepped()) -> List<&2, Boundary.Decision>:
  (state, decisions) = pair
  decisions

def run(lines: List<&2, Boundary.Line>, +state: Session.State, phase: Phase) -> Phase:
  match lines:
    case Nil{}:
      phase
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          run(
            rest, after(Session.step(fact, flags, state)),
            check(phase, fact, moves(decided(Session.step(fact, flags, state))))
          )

def kept(phase: Phase) -> Bool:
  match phase:
    case Broken{}:
      False{}
    case _:
      True{}

law lifecycle_follows_its_contracts:
  for lines: List<&2, Boundary.Line>
  for +state: Session.State
  {kept(run(lines, state, phase(state))) == True{} : Bool}

# The view automaton of ORB-C8 and C9 (../eon/orbit/docs/CONTRACTS.md), with
# the advance of authority's revision (ORB-C4) they name, checked against
# step's decisions. Step must carry out the proved vertical and selection
# policies' answers; the waiting slot follows the vertical answers its audit
# checks, the pointer route follows the selection audit, and the copy and the
# revision follow the contracts.

# The decisions these contracts constrain. Step's other decisions are outside
# them, and the automaton ignores them.
type Act is Data:
  # A complete frame for the client, and the end of a synchronized update.
  Framed{}
  Released{}
  # The host selection is cleared; whether authority advances for it.
  Cleared{advance: Bool}
  # The viewport moves: a commit, a return to live, a wheel or a key.
  Moved{}
  # A vertical answer that moves nothing: a preview, a terminal-owned commit,
  # a tick for the host to apply, or a failure before anything moves.
  Answered{}
  # A request from a revision ahead of authority, and a vertical request
  # while one waits.
  TooEarly{}
  Occupied{}
  # A wheel the terminal owns.
  WheelRouted{}
  # A press during a sequence, a phase without one, and a copy without one.
  InSequence{}
  NoSequence{}
  Uncopied{}
  # A phase refused for its position or the client's room before anything
  # moves.
  Unplaced{}
  # A pointer phase carried out on the host or the terminal, and the copied
  # text for the standard clipboard or the selection clipboard.
  OnHost{}
  OnTerminal{}
  CopiedOut{}
  SelectionCopied{}
  # Any other failure.
  Refusal{}
  # The client attaches, its attachment ends, or a stop begins.
  Joined{}
  Departed{}
  Halted{}

def reason_act(reason: Boundary.Reason) -> Act:
  match reason:
    case Boundary.ReasonVerticalBusy{}:
      Occupied{}
    case Boundary.ReasonVerticalAhead{}:
      TooEarly{}
    case Boundary.ReasonScrollNoRoom{}:
      Answered{}
    case Boundary.ReasonLiveOwned{}:
      Answered{}
    case Boundary.ReasonLiveNoRoom{}:
      Answered{}
    case Boundary.ReasonTickNoRoom{}:
      Answered{}
    case Boundary.ReasonSelectionAhead{}:
      TooEarly{}
    case Boundary.ReasonSelectionActive{}:
      InSequence{}
    case Boundary.ReasonUpdateIdle{}:
      NoSequence{}
    case Boundary.ReasonFinishIdle{}:
      NoSequence{}
    case Boundary.ReasonNoCopy{}:
      Uncopied{}
    case Boundary.ReasonSelectionOutside{}:
      Unplaced{}
    case Boundary.ReasonSelectionNoRoom{}:
      Unplaced{}
    case Boundary.ReasonPointerNoRoom{}:
      Unplaced{}
    case _:
      Refusal{}

# A terminal-owned commit or wheel moves nothing.
def committed(routed: Bool, owned: Act) -> Act:
  match routed:
    case True{}:
      owned
    case False{}:
      Moved{}

def copied(selection: Bool) -> Act:
  match selection:
    case True{}:
      SelectionCopied{}
    case False{}:
      CopiedOut{}

def act(decision: Boundary.Decision, rest: List<&2, Act>) -> List<&2, Act>:
  match decision:
    case Boundary.Frame{revision}:
      Framed{} <> rest
    case Boundary.Release{}:
      Released{} <> rest
    case Boundary.Unselect{advance}:
      Cleared{advance} <> rest
    case Boundary.Bottom{}:
      Moved{} <> rest
    case Boundary.LiveAnswer{revision}:
      Moved{} <> rest
    case Boundary.ScrollAnswer{rows, routed, revision}:
      committed(routed, Answered{}) <> rest
    case Boundary.WheelAnswer{routed, revision}:
      committed(routed, WheelRouted{}) <> rest
    case Boundary.PreviewAnswer{direction, routed, revision}:
      Answered{} <> rest
    case Boundary.TickAnswer{x, y, revision}:
      Answered{} <> rest
    case Boundary.Retick{x, y}:
      Answered{} <> rest
    case Boundary.Reject{reason}:
      reason_act(reason) <> rest
    case Boundary.SelectHost{}:
      OnHost{} <> rest
    case Boundary.SelectTerminal{}:
      OnTerminal{} <> rest
    case Boundary.Copied{selection}:
      copied(selection) <> rest
    case Boundary.Attach{}:
      Joined{} <> rest
    case Boundary.Detach{}:
      Departed{} <> rest
    case Boundary.Dismiss{}:
      Departed{} <> rest
    case Boundary.Reap{}:
      Halted{} <> rest
    case Boundary.ManageStop{}:
      Halted{} <> rest
    case _:
      rest

def acts(decisions: List<&2, Boundary.Decision>) -> List<&2, Act>:
  match decisions:
    case Nil{}:
      Nil{}
    case Con{decision, rest}:
      act(decision, acts(rest))

# The vertical request that waits, by kind: once a frame resolves it, its kind
# decides whether its answer may move.
type Pending is Data:
  Unrequested{}
  PreviewPending{}
  ScrollPending{}
  LivePending{}
  TickPending{}

def occupies(waiting: Pending) -> Bool:
  match waiting:
    case Unrequested{}:
      False{}
    case _:
      True{}

# ORB-C8: the alternate screen and a terminal-routed wheel own scrolling, so a
# return to live counts them as routed.
def terminal_owned(+flags: Boundary.Flags) -> Bool:
  Bool.or(Boundary.routed(flags), Boundary.alternate(flags))

# ORB-C8: whether a request's answer must not move. A preview never moves, nor
# a tick, which the host applies; a scroll moves only while the terminal does
# not own scrolling and the client has room for its outcome, and a return to
# live only while the terminal does not own scrolling and the client has room
# for a frame.
def unmoving(kind: Pending, +flags: Boundary.Flags) -> Bool:
  match kind:
    case Unrequested{}:
      True{}
    case PreviewPending{}:
      True{}
    case ScrollPending{}:
      Bool.or(Boundary.routed(flags), Bool.not(Boundary.room(flags)))
    case LivePending{}:
      Bool.or(terminal_owned(flags), Bool.not(Boundary.frame_room(flags)))
    case TickPending{}:
      True{}

type View is Data:
  # Whether a client is attached, which vertical request waits, the pointer
  # route, whether a copy exists, authority's revision, and whether a
  # synchronized update was held at the last fact.
  Viewing{
    client: Bool, waiting: Pending, route: Selection.Route, copied: Bool,
    revision: Revision.Revision, held: Bool
  }
  # A stop began; the lifecycle law covers what follows.
  Ending{}
  # The host sent a fact its protocol rules out; nothing after it is checked.
  Unordered{}
  # A decision broke the contracts.
  Violated{}

# How the next act is read: as carrying out the fact, as the answer to a
# vertical request, which may not move when it must be still, or as the move
# that answer's clear announced. Once a request is answered, nothing else in
# the fact clears or moves (ORB-C8).
type Pace is Data:
  Plain{answered: Bool}
  Answering{still: Bool}
  Moving{}

# ORB-C9: a clear ends a host sequence; a terminal sequence keeps its route,
# since only the host selection clears.
def cleared_route(route: Selection.Route) -> Selection.Route:
  match route:
    case Selection.Idle{}:
      Selection.Idle{}
    case Selection.Host{}:
      Selection.Idle{}
    case Selection.Terminal{}:
      Selection.Terminal{}

def advanced(advance: Bool, revision: Revision.Revision) -> Revision.Revision:
  match advance:
    case True{}:
      Revision.next(revision)
    case False{}:
      revision

def settled_view(
  owed: Bool, halted: Bool, client: Bool, waiting: Pending, route: Selection.Route, copied: Bool,
  revision: Revision.Revision, holding: Bool
) -> View:
  match owed:
    case True{}:
      Violated{}
    case False{}:
      match halted:
        case True{}:
          Ending{}
        case False{}:
          Viewing{client, waiting, route, copied, revision, holding}

# Reads the acts in order. A frame never shows while a synchronized update is
# held (ORB-C4); a frame published while a request waits resolves it, so its
# answer comes next (ORB-C8); a viewport move advances authority once, and a
# clear advances it as it says; a copy goes out only when one is owed, to the
# selection clipboard (ORB-C9); losing the client discards its request,
# sequence and copy. Resolving tells whether the waiting request's answer must
# be still (ORB-C8).
def fold(
  acts: List<&2, Act>, pace: Pace, owed: Bool, halted: Bool, client: Bool, waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision, holding: Bool,
  +resolving: Bool
) -> View:
  match acts:
    case Nil{}:
      match pace:
        case Plain{answered}:
          settled_view(owed, halted, client, waiting, route, copied, revision, holding)
        case _:
          Violated{}
    case Con{first, rest}:
      match first pace:
        case Framed{} Plain{answered}:
          match waiting holding:
            case _ True{}:
              Violated{}
            case Unrequested{} False{}:
              fold(
                rest, Plain{answered}, owed, halted, client, Unrequested{}, route, copied, revision,
                False{}, resolving
              )
            case _ False{}:
              fold(
                rest, Answering{resolving}, owed, halted, client, Unrequested{}, route, copied,
                revision, False{}, resolving
              )
        case Released{} Plain{answered}:
          fold(rest, Plain{answered}, owed, halted, client, waiting, route, copied, revision, False{}, resolving)
        case Cleared{advance} Plain{answered}:
          match answered:
            case True{}:
              Violated{}
            case False{}:
              fold(
                rest, Plain{False{}}, owed, halted, client, waiting, cleared_route(route), copied,
                advanced(advance, revision), holding, resolving
              )
        case Moved{} Plain{answered}:
          match answered:
            case True{}:
              Violated{}
            case False{}:
              fold(
                rest, Plain{False{}}, owed, halted, client, waiting, route, copied,
                Revision.next(revision), holding, resolving
              )
        case SelectionCopied{} Plain{answered}:
          match owed:
            case True{}:
              fold(rest, Plain{answered}, False{}, halted, client, waiting, route, copied, revision, holding, resolving)
            case False{}:
              Violated{}
        case Joined{} Plain{answered}:
          fold(rest, Plain{answered}, owed, halted, True{}, waiting, route, copied, revision, holding, resolving)
        case Departed{} Plain{answered}:
          fold(
            rest, Plain{answered}, owed, halted, False{}, Unrequested{}, Selection.Idle{}, False{},
            revision, holding, resolving
          )
        case Halted{} Plain{answered}:
          fold(rest, Plain{answered}, owed, True{}, client, waiting, route, copied, revision, holding, resolving)
        case Refusal{} Plain{answered}:
          fold(rest, Plain{answered}, owed, halted, client, waiting, route, copied, revision, holding, resolving)
        case WheelRouted{} Plain{answered}:
          fold(rest, Plain{answered}, owed, halted, client, waiting, route, copied, revision, holding, resolving)
        case Answered{} Answering{still}:
          fold(rest, Plain{True{}}, owed, halted, client, waiting, route, copied, revision, holding, resolving)
        case Cleared{advance} Answering{still}:
          match still:
            case True{}:
              Violated{}
            case False{}:
              fold(
                rest, Moving{}, owed, halted, client, waiting, cleared_route(route), copied,
                advanced(advance, revision), holding, resolving
              )
        case Moved{} Moving{}:
          fold(rest, Plain{True{}}, owed, halted, client, waiting, route, copied, Revision.next(revision), holding, resolving)
        case _ _:
          Violated{}

# The acts that carry out a fact, from an update the host reports open or not.
def plain(
  acts: List<&2, Act>, +flags: Boundary.Flags, owed: Bool, client: Bool, +waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  fold(
    acts, Plain{False{}}, owed, False{}, client, waiting, route, copied, revision,
    Boundary.held(flags), unmoving(waiting, flags)
  )

# Whether an act is the one expected.
def leads(expected: Act, first: Act) -> Bool:
  match expected first:
    case TooEarly{} TooEarly{}:
      True{}
    case Occupied{} Occupied{}:
      True{}
    case InSequence{} InSequence{}:
      True{}
    case NoSequence{} NoSequence{}:
      True{}
    case Uncopied{} Uncopied{}:
      True{}
    case CopiedOut{} CopiedOut{}:
      True{}
    case Refusal{} Refusal{}:
      True{}
    case _ _:
      False{}

# The acts carry out the fact, if the check before them passed.
def plain_if(
  yes: Bool, acts: List<&2, Act>, +flags: Boundary.Flags, client: Bool, waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match yes:
    case True{}:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)
    case False{}:
      Violated{}

# The acts begin with the expected one, and the rest carry out the fact.
def led(
  expected: Act, acts: List<&2, Act>, +flags: Boundary.Flags, client: Bool, waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match acts:
    case Con{first, rest}:
      plain_if(leads(expected, first), rest, flags, client, waiting, route, copied, revision)
    case Nil{}:
      Violated{}

# ORB-C8: a deferred request is answered nothing until a frame resolves it.
def deferred(
  kind: Pending, acts: List<&2, Act>, +flags: Boundary.Flags, client: Bool,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match acts:
    case Nil{}:
      plain(Nil{}, flags, False{}, client, kind, route, copied, revision)
    case Con{first, rest}:
      Violated{}

def replied(
  answer: Vertical.Answer, still: Bool, kind: Pending, acts: List<&2, Act>,
  +flags: Boundary.Flags, client: Bool, +waiting: Pending, route: Selection.Route,
  copied: Bool, revision: Revision.Revision
) -> View:
  match answer:
    case Vertical.RejectAhead{}:
      led(TooEarly{}, acts, flags, client, waiting, route, copied, revision)
    case Vertical.RejectBusy{}:
      led(Occupied{}, acts, flags, client, waiting, route, copied, revision)
    case Vertical.Defer{}:
      deferred(kind, acts, flags, client, route, copied, revision)
    case Vertical.Reply{}:
      fold(
        acts, Answering{still}, False{}, False{}, client, waiting, route, copied, revision,
        Boundary.held(flags), unmoving(waiting, flags)
      )
    case _:
      Violated{}

# The audit checks the answer against the slot; a deferred request takes the
# slot, and every other answer keeps it.
def audited(
  check: Vertical.Audit, answer: Vertical.Answer, still: Bool, kind: Pending,
  acts: List<&2, Act>, +flags: Boundary.Flags, client: Bool, waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match check:
    case Vertical.Clean{occupied}:
      replied(answer, still, kind, acts, flags, client, waiting, route, copied, revision)
    case Vertical.Broken{}:
      Violated{}

def requested(
  +answer: Vertical.Answer, still: Bool, kind: Pending, acts: List<&2, Act>,
  +flags: Boundary.Flags, client: Bool, +waiting: Pending, route: Selection.Route,
  copied: Bool, revision: Revision.Revision
) -> View:
  audited(
    Vertical.audit_clean(occupies(waiting), answer), answer, still, kind, acts, flags, client,
    waiting, route, copied, revision
  )

# What the policy treats as routed: for a return to live, any terminal-owned
# scrolling; a tick is never routed.
def routing(kind: Pending, +flags: Boundary.Flags) -> Bool:
  match kind:
    case LivePending{}:
      terminal_owned(flags)
    case TickPending{}:
      False{}
    case _:
      Boundary.routed(flags)

# ORB-C8: a vertical request gets the proved policy's answer: a future
# revision fails, a terminal-routed request is answered at once without
# moving, and one request may wait while an update is held. A request's
# answer moves only when it may.
def request(
  +kind: Pending, future: Bool, +flags: Boundary.Flags, acts: List<&2, Act>, client: Bool,
  +waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  requested(
    Vertical.answer(
      Vertical.slot_of(occupies(waiting)),
      Vertical.Request{future, routing(kind, flags), Boundary.held(flags)}
    ),
    unmoving(kind, flags), kind, acts, flags, client, waiting, route, copied, revision
  )

def holds_gesture(route: Selection.Route, alternate: Bool) -> Bool:
  match route:
    case Selection.Host{}:
      Bool.not(alternate)
    case _:
      False{}

# ORB-C9: an upward tick runs only for a held host gesture on the primary
# screen and from the top row, as a vertical request that is never routed; a
# lost anchor ends the sequence.
def tick_with(
  gesture: Bool, state: Boundary.TickState, +flags: Boundary.Flags, acts: List<&2, Act>,
  client: Bool, waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match gesture:
    case False{}:
      led(Refusal{}, acts, flags, client, waiting, route, copied, revision)
    case True{}:
      match state:
        case Boundary.TickStateFits{}:
          request(TickPending{}, False{}, flags, acts, client, waiting, route, copied, revision)
        case Boundary.TickStateNotTopRow{}:
          led(Refusal{}, acts, flags, client, waiting, route, copied, revision)
        case Boundary.TickStateAnchorLost{}:
          plain(acts, flags, False{}, client, waiting, Selection.Idle{}, copied, revision)

def tick(
  state: Boundary.TickState, +flags: Boundary.Flags, acts: List<&2, Act>, client: Bool,
  waiting: Pending, +route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  tick_with(
    holds_gesture(route, Boundary.alternate(flags)), state, flags, acts, client, waiting, route,
    copied, revision
  )

def unmoved(acts: List<&2, Act>) -> Bool:
  match acts:
    case Nil{}:
      True{}
    case Con{first, rest}:
      match first:
        case Moved{}:
          False{}
        case _:
          unmoved(rest)

# ORB-C8: a terminal-owned wheel moves nothing, nor one during a held update
# or without room for a frame.
def wheel_still(+flags: Boundary.Flags) -> Bool:
  Bool.or(Boundary.routed(flags), Bool.or(Boundary.held(flags), Bool.not(Boundary.frame_room(flags))))

def wheel(
  still: Bool, +acts: List<&2, Act>, +flags: Boundary.Flags, client: Bool, waiting: Pending,
  route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match still:
    case True{}:
      plain_if(unmoved(acts), acts, flags, client, waiting, route, copied, revision)
    case False{}:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)

def pointer_signal(
  kind: Boundary.SelectionKind, copied: Bool, future: Bool, +flags: Boundary.Flags
) -> Selection.Signal:
  match kind:
    case Boundary.SelectionKindBegin{}:
      Selection.Begin{future, Boundary.tracking(flags), Boundary.shift(flags)}
    case Boundary.SelectionKindUpdate{}:
      Selection.Update{}
    case Boundary.SelectionKindFinish{}:
      Selection.Finish{}
    case Boundary.SelectionKindCancel{}:
      Selection.Cancel{}
    case Boundary.SelectionKindCopy{}:
      Selection.Copy{copied}

# A host phase is carried out on the route the audit gives, or refused before
# anything moves, keeping the route it had.
def on_host(
  acts: List<&2, Act>, next: Selection.Route, before: Selection.Route, +flags: Boundary.Flags,
  client: Bool, waiting: Pending, copied: Bool, revision: Revision.Revision
) -> View:
  match acts:
    case Con{first, rest}:
      match first:
        case OnHost{}:
          plain(rest, flags, False{}, client, waiting, next, copied, revision)
        case Unplaced{}:
          plain(rest, flags, False{}, client, waiting, before, copied, revision)
        case _:
          Violated{}
    case Nil{}:
      Violated{}

def is_begin(kind: Boundary.SelectionKind) -> Bool:
  match kind:
    case Boundary.SelectionKindBegin{}:
      True{}
    case _:
      False{}

# Input the PTY cannot take ends the attachment, which first clears the host
# selection.
def overflowed(
  advance: Bool, rest: List<&2, Act>, before: Selection.Route, +flags: Boundary.Flags,
  client: Bool, waiting: Pending, copied: Bool, revision: Revision.Revision
) -> View:
  led(
    Refusal{}, rest, flags, client, waiting, cleared_route(before), copied,
    advanced(advance, revision)
  )

def pressed(
  advance: Bool, rest: List<&2, Act>, next: Selection.Route, before: Selection.Route,
  +flags: Boundary.Flags, client: Bool, waiting: Pending, copied: Bool,
  revision: Revision.Revision
) -> View:
  match rest:
    case Con{second, more}:
      match second:
        case OnTerminal{}:
          plain(
            more, flags, False{}, client, waiting, cleared_route(next), False{},
            advanced(advance, revision)
          )
        case _:
          overflowed(advance, rest, before, flags, client, waiting, copied, revision)
    case Nil{}:
      Violated{}

# A terminal press clears the host selection and its copy before it reaches
# the terminal; any phase may be refused before anything moves, or end the
# attachment when the PTY cannot take it.
def on_terminal(
  begin: Bool, acts: List<&2, Act>, next: Selection.Route, before: Selection.Route,
  +flags: Boundary.Flags, client: Bool, waiting: Pending, copied: Bool,
  revision: Revision.Revision
) -> View:
  match begin:
    case True{}:
      match acts:
        case Con{first, rest}:
          match first:
            case Cleared{advance}:
              pressed(advance, rest, next, before, flags, client, waiting, copied, revision)
            case Unplaced{}:
              plain(rest, flags, False{}, client, waiting, before, copied, revision)
            case _:
              Violated{}
        case Nil{}:
          Violated{}
    case False{}:
      match acts:
        case Con{first, rest}:
          match first:
            case OnTerminal{}:
              plain(rest, flags, False{}, client, waiting, next, copied, revision)
            case Unplaced{}:
              plain(rest, flags, False{}, client, waiting, before, copied, revision)
            case Cleared{advance}:
              overflowed(advance, rest, before, flags, client, waiting, copied, revision)
            case _:
              Violated{}
        case Nil{}:
          Violated{}

# A Cancel clears the host selection and ends the sequence, or is refused
# before anything moves.
def cancelled(
  acts: List<&2, Act>, next: Selection.Route, before: Selection.Route, +flags: Boundary.Flags,
  client: Bool, waiting: Pending, copied: Bool, revision: Revision.Revision
) -> View:
  match acts:
    case Con{first, rest}:
      match first:
        case Cleared{advance}:
          plain(
            rest, flags, False{}, client, waiting, cleared_route(next), copied,
            advanced(advance, revision)
          )
        case Unplaced{}:
          plain(rest, flags, False{}, client, waiting, before, copied, revision)
        case _:
          Violated{}
    case Nil{}:
      Violated{}

def carried_out(
  check: Selection.Audit, answer: Selection.Answer, kind: Boundary.SelectionKind,
  acts: List<&2, Act>, before: Selection.Route, +flags: Boundary.Flags, client: Bool,
  waiting: Pending, copied: Bool, revision: Revision.Revision
) -> View:
  match check:
    case Selection.Broken{}:
      Violated{}
    case Selection.Clean{next}:
      match answer:
        case Selection.RejectAhead{}:
          led(TooEarly{}, acts, flags, client, waiting, next, copied, revision)
        case Selection.RejectActive{}:
          led(InSequence{}, acts, flags, client, waiting, next, copied, revision)
        case Selection.RejectIdle{}:
          led(NoSequence{}, acts, flags, client, waiting, next, copied, revision)
        case Selection.RejectNoCopy{}:
          led(Uncopied{}, acts, flags, client, waiting, next, copied, revision)
        case Selection.Copied{}:
          led(CopiedOut{}, acts, flags, client, waiting, next, copied, revision)
        case Selection.HostPhase{}:
          on_host(acts, next, before, flags, client, waiting, copied, revision)
        case Selection.TerminalPhase{}:
          on_terminal(is_begin(kind), acts, next, before, flags, client, waiting, copied, revision)
        case Selection.Cancelled{}:
          cancelled(acts, next, before, flags, client, waiting, copied, revision)
        case Selection.Quiet{}:
          plain(acts, flags, False{}, client, waiting, next, copied, revision)

def answered_pointer(
  +answer: Selection.Answer, +signal: Selection.Signal, kind: Boundary.SelectionKind,
  acts: List<&2, Act>, +route: Selection.Route, +flags: Boundary.Flags, client: Bool,
  waiting: Pending, copied: Bool, revision: Revision.Revision
) -> View:
  carried_out(
    Selection.audit_route(signal, route, answer), answer, kind, acts, route, flags, client,
    waiting, copied, revision
  )

def pointed(
  +signal: Selection.Signal, kind: Boundary.SelectionKind, acts: List<&2, Act>,
  +route: Selection.Route, +flags: Boundary.Flags, client: Bool, waiting: Pending, copied: Bool,
  revision: Revision.Revision
) -> View:
  answered_pointer(
    Selection.answer(route, signal), signal, kind, acts, route, flags, client, waiting, copied,
    revision
  )

# ORB-C9: a pointer phase gets the proved selection policy's answer, on the
# route its audit keeps; a copy is returned only when one exists.
def pointing(
  +kind: Boundary.SelectionKind, future: Bool, +flags: Boundary.Flags, acts: List<&2, Act>,
  client: Bool, waiting: Pending, route: Selection.Route, +copied: Bool,
  revision: Revision.Revision
) -> View:
  pointed(
    pointer_signal(kind, copied, future, flags), kind, acts, route, flags, client, waiting,
    copied, revision
  )

# ORB-C9: what the host reports of a host phase: a finish freezes a copy
# exactly when a selection is presented and returns it once, a press forgets
# the copy, and a failure ends the sequence.
def host_outcome(
  outcome: Boundary.Selected, +presented: Bool, +flags: Boundary.Flags, acts: List<&2, Act>,
  client: Bool, waiting: Pending, route: Selection.Route, copied: Bool,
  revision: Revision.Revision
) -> View:
  match outcome:
    case Boundary.SelectedApplied{}:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)
    case Boundary.SelectedFinished{}:
      plain(acts, flags, presented, client, waiting, route, presented, revision)
    case Boundary.SelectedFailed{}:
      led(Refusal{}, acts, flags, client, waiting, Selection.Idle{}, copied, revision)
    case Boundary.SelectedBegan{}:
      plain(acts, flags, False{}, client, waiting, route, False{}, revision)
    case Boundary.SelectedTicked{}:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)

# Facts the host sends only from the attached client.
def client_only(client: Bool, view: View) -> View:
  match client:
    case True{}:
      view
    case False{}:
      Unordered{}

# A resize ends a sequence on either route.
def applied(
  effect: Boundary.Effect, +flags: Boundary.Flags, acts: List<&2, Act>, client: Bool,
  waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match effect:
    case Boundary.EffectResize{}:
      plain(acts, flags, False{}, client, waiting, Selection.Idle{}, copied, revision)
    case _:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)

def by_fact(
  fact: Boundary.Fact, +flags: Boundary.Flags, acts: List<&2, Act>, future: Bool, +client: Bool,
  waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match fact:
    case Boundary.Started{pid}:
      match client:
        case True{}:
          Unordered{}
        case False{}:
          plain(acts, flags, False{}, False{}, waiting, route, copied, revision)
    # A closed PTY ends a sequence on either route.
    case Boundary.PtyClosed{}:
      plain(acts, flags, False{}, client, waiting, Selection.Idle{}, copied, revision)
    case Boundary.Applied{effect}:
      client_only(client, applied(effect, flags, acts, client, waiting, route, copied, revision))
    case Boundary.Closed{full}:
      client_only(client, plain(acts, flags, False{}, client, waiting, route, copied, revision))
    case Boundary.Pressured{}:
      client_only(client, plain(acts, flags, False{}, client, waiting, route, copied, revision))
    case Boundary.Refused{version}:
      client_only(client, plain(acts, flags, False{}, client, waiting, route, copied, revision))
    case Boundary.PreviewVertical{named, direction}:
      client_only(
        client, request(PreviewPending{}, future, flags, acts, client, waiting, route, copied, revision)
      )
    case Boundary.ScrollVertical{named, rows}:
      client_only(
        client, request(ScrollPending{}, future, flags, acts, client, waiting, route, copied, revision)
      )
    case Boundary.ReturnToLive{}:
      client_only(
        client, request(LivePending{}, False{}, flags, acts, client, waiting, route, copied, revision)
      )
    case Boundary.Wheel{input}:
      client_only(
        client, wheel(wheel_still(flags), acts, flags, client, waiting, route, copied, revision)
      )
    case Boundary.Selection{kind, named}:
      client_only(client, pointing(kind, future, flags, acts, client, waiting, route, copied, revision))
    case Boundary.Selected{outcome, presented}:
      client_only(
        client, host_outcome(outcome, presented, flags, acts, client, waiting, route, copied, revision)
      )
    case Boundary.Tick{state, x, y}:
      client_only(client, tick(state, flags, acts, client, waiting, route, copied, revision))
    # The host reports these only after Bend asks, and Fault only in place of
    # Started, for a worker that dies at once.
    case Boundary.Reaped{status}:
      Unordered{}
    case Boundary.ExitedBeforeStop{status}:
      Unordered{}
    case Boundary.NotReaped{}:
      Unordered{}
    case Boundary.Finished{}:
      Unordered{}
    case Boundary.Fault{pid}:
      Unordered{}
    case _:
      plain(acts, flags, False{}, client, waiting, route, copied, revision)

# ORB-C9: output that left no valid anchor, or newly enabled mouse tracking,
# clears the host selection before the fact is decided.
def unanchored_acts(
  acts: List<&2, Act>, fact: Boundary.Fact, +flags: Boundary.Flags, future: Bool, client: Bool,
  waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision
) -> View:
  match acts:
    case Con{first, rest}:
      match first:
        case Cleared{advance}:
          by_fact(
            fact, flags, rest, future, client, waiting, cleared_route(route), copied,
            advanced(advance, revision)
          )
        case _:
          Violated{}
    case Nil{}:
      Violated{}

def anchoring(
  unanchored: Bool, fact: Boundary.Fact, +flags: Boundary.Flags, acts: List<&2, Act>,
  future: Bool, client: Bool, waiting: Pending, route: Selection.Route, copied: Bool,
  revision: Revision.Revision
) -> View:
  match unanchored:
    case True{}:
      unanchored_acts(acts, fact, flags, future, client, waiting, route, copied, revision)
    case False{}:
      by_fact(fact, flags, acts, future, client, waiting, route, copied, revision)

def fed(held: Bool, revision: Revision.Revision) -> Revision.Revision:
  match held:
    case True{}:
      revision
    case False{}:
      Revision.next(revision)

# ORB-C4: a fact the host applied before Bend answers changes presented state
# once: a PTY turn that fed output to the terminal, unless an update was held
# before it, a resize, and a host phase or tick that applied.
def fact_advance(
  fact: Boundary.Fact, held: Bool, revision: Revision.Revision
) -> Revision.Revision:
  match fact:
    case Boundary.Output{foreground, marks}:
      fed(held, revision)
    case Boundary.ChildExited{drained}:
      match drained:
        case True{}:
          fed(held, revision)
        case False{}:
          revision
    case Boundary.Applied{effect}:
      match effect:
        case Boundary.EffectResize{}:
          Revision.next(revision)
        case _:
          revision
    case Boundary.Selected{outcome, presented}:
      match outcome:
        case Boundary.SelectedFailed{}:
          revision
        case _:
          Revision.next(revision)
    case _:
      revision

def viewing(
  +fact: Boundary.Fact, +flags: Boundary.Flags, acts: List<&2, Act>, future: Bool, client: Bool,
  waiting: Pending, route: Selection.Route, copied: Bool, revision: Revision.Revision, held: Bool
) -> View:
  anchoring(
    Boundary.unanchored(flags), fact, flags, acts, future, client, waiting, route, copied,
    fact_advance(fact, held, revision)
  )

# Once a stop began, the host sends only the reap, an observer's loss and a
# tick or host phase that was already queued.
def ended_view(fact: Boundary.Fact) -> View:
  match fact:
    case Boundary.Reaped{status}:
      Ending{}
    case Boundary.ExitedBeforeStop{status}:
      Ending{}
    case Boundary.ObserverGone{loss}:
      Ending{}
    case Boundary.Tick{state, x, y}:
      Ending{}
    case Boundary.Selected{outcome, presented}:
      Ending{}
    case _:
      Unordered{}

def view_check(
  view: View, fact: Boundary.Fact, +flags: Boundary.Flags, acts: List<&2, Act>, future: Bool
) -> View:
  match view:
    case Viewing{client, waiting, route, copied, revision, held}:
      viewing(fact, flags, acts, future, client, waiting, route, copied, revision, held)
    case Ending{}:
      ended_view(fact)
    case Unordered{}:
      Unordered{}
    case Violated{}:
      Violated{}

def pending(waiting: Session.Waiting) -> Pending:
  match waiting:
    case Session.Clear{}:
      Unrequested{}
    case Session.Waiting{request}:
      match request:
        case Session.PreviewRequest{direction}:
          PreviewPending{}
        case Session.ScrollRequest{rows}:
          ScrollPending{}
        case Session.LiveRequest{}:
          LivePending{}
        case Session.TickRequest{x, y}:
          TickPending{}

def viewed(
  client: Bool, waiting: Session.Waiting, pointer: Session.Pointer, shown: Session.Shown
) -> View:
  match pointer:
    case Session.Pointer{route, presented, copied}:
      match shown:
        case Session.Shown{revision, held}:
          Viewing{client, pending(waiting), route, copied, revision, held}

# The view a session state starts the automaton in.
def view(state: Session.State) -> View:
  match state:
    case Session.State{session, shown, status}:
      match session:
        case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
          match life:
            case Orbit.Detached{pid}:
              viewed(False{}, waiting, pointer, shown)
            case Orbit.Attached{pid}:
              viewed(True{}, waiting, pointer, shown)
            case Orbit.Stopping{pid}:
              Ending{}
            case Orbit.Stopped{pid}:
              Ending{}

# Authority as the automaton keeps it.
def view_revision(view: View) -> Revision.Revision:
  match view:
    case Viewing{client, waiting, route, copied, revision, held}:
      revision
    case _:
      Revision.Revision{0, 0}

# A request names the revision of the frame it was made on; it is from the
# future when that revision is ahead of the automaton's authority, so a step
# whose revision departs from the automaton's answers some request wrongly.
def ahead_of(fact: Boundary.Fact, authority: Revision.Revision) -> Bool:
  match fact:
    case Boundary.PreviewVertical{named, direction}:
      Revision.ahead(named, authority)
    case Boundary.ScrollVertical{named, rows}:
      Revision.ahead(named, authority)
    case Boundary.Selection{kind, named}:
      Revision.ahead(named, authority)
    case _:
      False{}

def view_run(lines: List<&2, Boundary.Line>, +state: Session.State, +view: View) -> View:
  match lines:
    case Nil{}:
      view
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          view_run(
            rest, after(Session.step(fact, flags, state)),
            view_check(
              view, fact, flags, acts(decided(Session.step(fact, flags, state))),
              ahead_of(fact, view_revision(view))
            )
          )

def view_kept(view: View) -> Bool:
  match view:
    case Violated{}:
      False{}
    case _:
      True{}

law scrolling_and_selection_follow_their_contracts:
  for lines: List<&2, Boundary.Line>
  for +state: Session.State
  {view_kept(view_run(lines, state, view(state))) == True{} : Bool}

# The management automaton of ORB-C13 (../eon/orbit/docs/CONTRACTS.md),
# checked against step's decisions. Step must carry out the proved management
# policy's answer, and the slot follows the policy's audit
# (src/management.bend).

# The management answers among step's decisions. Step's other decisions are
# outside this contract, and the automaton ignores them.
def reply(
  decision: Boundary.Decision, rest: List<&2, Management.Answer>
) -> List<&2, Management.Answer>:
  match decision:
    case Boundary.ManageAdmit{}:
      Management.Admit{} <> rest
    case Boundary.ManageBusy{}:
      Management.Busy{} <> rest
    case Boundary.ManageLease{}:
      Management.Lease{} <> rest
    case Boundary.ManageRejectIdentity{}:
      Management.RejectIdentity{} <> rest
    case Boundary.ManageReject{}:
      Management.RejectRequest{} <> rest
    case Boundary.ManageStatus{}:
      Management.Report{} <> rest
    case Boundary.ManageProgramStatus{}:
      Management.Records{} <> rest
    case Boundary.ManageRefuse{}:
      Management.Refuse{} <> rest
    case Boundary.ManageStop{}:
      Management.Shutdown{} <> rest
    case _:
      rest

def replies(decisions: List<&2, Boundary.Decision>) -> List<&2, Management.Answer>:
  match decisions:
    case Nil{}:
      Nil{}
    case Con{decision, rest}:
      reply(decision, replies(rest))

type Lease is Data:
  # The management slot, as the management audit keeps it.
  Leasing{slot: Management.Slot}
  # A stop began; the lifecycle law covers what follows.
  Ceased{}
  # The host sent a fact its protocol rules out; nothing after it is checked.
  Disordered{}
  # A decision broke the contract.
  Breached{}

def management_signal(event: Boundary.ManagementEvent) -> Management.Signal:
  match event:
    case Boundary.ManagementEventConnect{}:
      Management.Connect{}
    case Boundary.ManagementEventAcquireValid{}:
      Management.Acquire{True{}}
    case Boundary.ManagementEventAcquireInvalid{}:
      Management.Acquire{False{}}
    case Boundary.ManagementEventStatus{}:
      Management.Status{}
    case Boundary.ManagementEventStop{}:
      Management.Stop{}
    case Boundary.ManagementEventBad{}:
      Management.Bad{}
    case Boundary.ManagementEventGone{}:
      Management.Gone{}

# An accepted Stop begins the stop (ORB-C12); any other answer the audit
# passes keeps the slot it gives.
def audited_lease(check: Management.Audit, answer: Management.Answer) -> Lease:
  match check:
    case Management.Broken{}:
      Breached{}
    case Management.Clean{slot}:
      match answer:
        case Management.Shutdown{}:
          Ceased{}
        case _:
          Leasing{slot}

# A management event gets at most one answer, and the management audit checks
# it from the slot the automaton keeps.
def answered_event(
  signal: Management.Signal, slot: Management.Slot, replies: List<&2, Management.Answer>
) -> Lease:
  match replies:
    case Nil{}:
      audited_lease(Management.audit_slot(signal, slot, Management.Quiet{}), Management.Quiet{})
    case Con{+answer, rest}:
      match rest:
        case Nil{}:
          audited_lease(Management.audit_slot(signal, slot, answer), answer)
        case _:
          Breached{}

# Any other fact answers nothing on the management socket.
def unanswered(replies: List<&2, Management.Answer>, lease: Lease) -> Lease:
  match replies:
    case Nil{}:
      lease
    case _:
      Breached{}

def on_lease(
  fact: Boundary.Fact, replies: List<&2, Management.Answer>, slot: Management.Slot
) -> Lease:
  match fact:
    case Boundary.Management{event}:
      answered_event(management_signal(event), slot, replies)
    # The lease is kept only while the child runs, so a ProgramStatus request
    # here is live.
    case Boundary.ProgramStatus{foreground}:
      answered_event(Management.ProgramStatus{True{}}, slot, replies)
    # ORB-C12: the child's exit begins the stop.
    case Boundary.ChildExited{drained}:
      unanswered(replies, Ceased{})
    # The host reports a reap only after Bend asks for one.
    case Boundary.Reaped{status}:
      Disordered{}
    case Boundary.ExitedBeforeStop{status}:
      Disordered{}
    case _:
      unanswered(replies, Leasing{slot})

# Once a stop began, the host sends only the reap, an observer's loss and a
# tick or host phase that was already queued.
def ceased(fact: Boundary.Fact) -> Lease:
  match fact:
    case Boundary.Reaped{status}:
      Ceased{}
    case Boundary.ExitedBeforeStop{status}:
      Ceased{}
    case Boundary.ObserverGone{loss}:
      Ceased{}
    case Boundary.Tick{state, x, y}:
      Ceased{}
    case Boundary.Selected{outcome, presented}:
      Ceased{}
    case _:
      Disordered{}

def lease_check(
  lease: Lease, fact: Boundary.Fact, replies: List<&2, Management.Answer>
) -> Lease:
  match lease:
    case Leasing{slot}:
      on_lease(fact, replies, slot)
    case Ceased{}:
      ceased(fact)
    case Disordered{}:
      Disordered{}
    case Breached{}:
      Breached{}

# The lease state a session state starts the automaton in.
def lease(state: Session.State) -> Lease:
  match state:
    case Session.State{session, shown, status}:
      match session:
        case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
          match life:
            case Orbit.Detached{pid}:
              Leasing{manage}
            case Orbit.Attached{pid}:
              Leasing{manage}
            case Orbit.Stopping{pid}:
              Ceased{}
            case Orbit.Stopped{pid}:
              Ceased{}

def lease_run(lines: List<&2, Boundary.Line>, +state: Session.State, lease: Lease) -> Lease:
  match lines:
    case Nil{}:
      lease
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          lease_run(
            rest, after(Session.step(fact, flags, state)),
            lease_check(lease, fact, replies(decided(Session.step(fact, flags, state))))
          )

def lease_kept(lease: Lease) -> Bool:
  match lease:
    case Breached{}:
      False{}
    case _:
      True{}

law management_follows_its_contract:
  for lines: List<&2, Boundary.Line>
  for +state: Session.State
  {lease_kept(lease_run(lines, state, lease(state))) == True{} : Bool}
