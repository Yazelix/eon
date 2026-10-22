import Base
import ./lifecycle.bend as Life
import ./admission.bend as Admission
import ./vertical.bend as Vertical
import ./selection.bend as Selection
import ./management.bend as Management
import ./revision.bend as Revision
import ./boundary.bend as Boundary
import ./status.bend as Status

# The session policy of the C-free host (docs/HOST.md). The host reports one
# fact per event with its flags, and step answers it with typed decisions
# (src/boundary.schema), from explicit state. It is pure, so PROOF.bend can
# prove laws over it; src/main.bend serves it.

# A vertical request with what the host needs to carry it out: the rows as a
# 16-bit two's complement and the tick's position as the bits of two F32s,
# which Bend never reads.
type Request is Data:
  PreviewRequest{direction: Boundary.Direction}
  ScrollRequest{rows: U32}
  LiveRequest{}
  TickRequest{x: U32, y: U32}

# The vertical request slot of ORB-C8: clear, or one request waiting for a
# synchronized update to end.
type Waiting is Data:
  Clear{}
  Waiting{request: Request}

# The pointer state of ORB-C9: the active route, whether a host selection is
# presented, and whether a finished host sequence froze a copy. Rust keeps
# libghostty's gesture and the frozen text.
type Pointer is Data:
  Pointer{route: Selection.Route, presented: Bool, copied: Bool}

type Session is Data:
  Session{
    life: Life.Lifecycle, written: Bool, observer: Bool, explicit: Bool, waiting: Waiting,
    pointer: Pointer, manage: Management.Slot
  }

# The presentation revision of ORB-C4, and whether a synchronized update was
# held when the last fact was decided.
type Shown is Data:
  Shown{revision: Revision.Revision, held: Bool}

# Bend's whole state: the session that decides, the revision that the
# stamp after each decision advances, and the program status lifetime tracker
# of ORB-C15 (src/status.bend).
type State is Data:
  State{session: Session, shown: Shown, status: Status.Tracker}

# The next state and the decisions that carry it out.
def Next() -> Type:
  Session & Boundary.Decisions()

def Stepped() -> Type:
  State & Boundary.Decisions()

# A publication's decisions and the vertical slot after them.
def Published() -> Type:
  Waiting & Boundary.Decisions()

# Publications name the first revision until stamp gives each the revision it
# presents.
def unstamped() -> Revision.Revision:
  Revision.Revision{0, 0}

# Decisions are carried out in order.
def then(first: Boundary.Decisions(), rest: Boundary.Decisions()) -> Boundary.Decisions():
  List.append(&2, Boundary.Decision, first, rest)

def reject(reason: Boundary.Reason) -> Boundary.Decisions():
  [Boundary.Reject{reason}]

def prefixed(first: Boundary.Decisions(), pair: Next()) -> Next():
  (session, rest) = pair
  (session, then(first, rest))

def followed(pair: Next(), last: Boundary.Decisions()) -> Next():
  (session, rest) = pair
  (session, then(rest, last))

# A viewport move clears the host selection and keeps the copy; it advances
# authority itself.
def moved(decision: Boundary.Decision) -> Boundary.Decisions():
  [Boundary.Unselect{False{}}, decision]

def is_attached(life: Life.Lifecycle) -> Bool:
  match life:
    case Life.Attached{pid}:
      True{}
    case _:
      False{}

# Whether the child still runs: no stop has begun and it has not exited.
def runs(life: Life.Lifecycle) -> Bool:
  match life:
    case Life.Detached{pid}:
      True{}
    case Life.Attached{pid}:
      True{}
    case Life.Stopping{pid}:
      False{}
    case Life.Stopped{pid}:
      False{}

# Metadata is queued only when the title or working directory changed since
# the metadata last queued. The first queued for an observer stays, and later
# metadata replaces what has not started, so a slow observer converges on the
# current state.
def metadata_when(changed: Bool, fresh: Bool) -> Boundary.Decisions():
  match changed:
    case True{}:
      [Boundary.Metadata{unstamped(), fresh}]
    case False{}:
      Nil{}

# The observer hears every change a frame or detached output carries.
def metadata_if(observer: Bool, +flags: Boundary.Flags) -> Boundary.Decisions():
  match observer:
    case True{}:
      metadata_when(Boundary.metadata_changed(flags), Boundary.metadata_fresh(flags))
    case False{}:
      Nil{}

# A scroll whose outcome could not be queued whole fails with Failure(Terminal)
# before the viewport moves, and the client stays attached (ORB-C8); a
# terminal-owned scroll needs no room.
def scroll_answer(routed: Bool, room: Bool, rows: U32) -> Boundary.Decisions():
  match routed:
    case True{}:
      [Boundary.ScrollAnswer{rows, True{}, unstamped()}]
    case False{}:
      match room:
        case True{}:
          moved(Boundary.ScrollAnswer{rows, False{}, unstamped()})
        case False{}:
          reject(Boundary.ReasonScrollNoRoom{})

# A return to live output fails while the terminal owns scrolling, and fails
# with Failure(Terminal) before the viewport moves when the client's queue could
# not take a whole frame; the client stays attached (ORB-C8).
def live_answer(owned: Bool, room: Bool) -> Boundary.Decisions():
  match owned:
    case True{}:
      reject(Boundary.ReasonLiveOwned{})
    case False{}:
      match room:
        case True{}:
          moved(Boundary.LiveAnswer{unstamped()})
        case False{}:
          reject(Boundary.ReasonLiveNoRoom{})

# The alternate screen and a terminal-routed wheel own scrolling, so live
# history is unavailable.
def owned(+flags: Boundary.Flags) -> Bool:
  Bool.or(Boundary.routed(flags), Boundary.alternate(flags))

# An upward selection tick fails with Failure(Terminal) before anything moves
# when the client's queue could not take a whole scroll outcome (ORB-C9). A
# tick that waited is checked again, since its gesture may have ended.
def tick_answer(room: Bool, stored: Bool, x: U32, y: U32) -> Boundary.Decisions():
  match room:
    case True{}:
      match stored:
        case True{}:
          [Boundary.Retick{x, y}]
        case False{}:
          [Boundary.TickAnswer{x, y, unstamped()}]
    case False{}:
      reject(Boundary.ReasonTickNoRoom{})

# Carries out a request the proved vertical policy (src/vertical.bend) replies
# to, or the waiting one it resolves (stored).
def answered(request: Request, stored: Bool, +flags: Boundary.Flags) -> Boundary.Decisions():
  match request:
    case PreviewRequest{direction}:
      [Boundary.PreviewAnswer{direction, Boundary.routed(flags), unstamped()}]
    case ScrollRequest{rows}:
      scroll_answer(Boundary.routed(flags), Boundary.room(flags), rows)
    case LiveRequest{}:
      live_answer(owned(flags), Boundary.frame_room(flags))
    case TickRequest{x, y}:
      tick_answer(Boundary.room(flags), stored, x, y)

# What the policy treats as routed: for a return to live, any terminal-owned
# scrolling, which fails at once; a tick is never routed.
def routing(request: Request, +flags: Boundary.Flags) -> Bool:
  match request:
    case LiveRequest{}:
      owned(flags)
    case TickRequest{x, y}:
      False{}
    case _:
      Boundary.routed(flags)

def resolved(answer: Vertical.Answer, request: Request, flags: Boundary.Flags) -> Published():
  match answer:
    case Vertical.Resolve{}:
      (Clear{}, answered(request, True{}, flags))
    case _:
      (Waiting{request}, Nil{})

# Every publication asks the policy, so a release resolves a waiting request;
# a clear slot has nothing to resolve (Vertical.publish answers Quiet).
def resolution(waiting: Waiting, +flags: Boundary.Flags) -> Published():
  match waiting:
    case Clear{}:
      (Clear{}, Nil{})
    case Waiting{+request}:
      resolved(
        Vertical.answer(Vertical.Waiting{}, Vertical.Publish{routing(request, flags), False{}}),
        request, flags
      )

def framed(observer: Bool, flags: Boundary.Flags, pair: Published()) -> Published():
  (waiting, rest) = pair
  (waiting, Boundary.Frame{unstamped()} <> then(metadata_if(observer, flags), rest))

# A published change: a frame for an attached client, metadata for the
# observer, and the policy's answer to a waiting request.
def published(
  attached: Bool, observer: Bool, waiting: Waiting, +flags: Boundary.Flags
) -> Published():
  match attached:
    case True{}:
      framed(observer, flags, resolution(waiting, flags))
    case False{}:
      (waiting, metadata_if(observer, flags))

# Frames and metadata wait while a synchronized update is open.
def publish(
  held: Bool, attached: Bool, observer: Bool, waiting: Waiting, flags: Boundary.Flags
) -> Published():
  match held:
    case True{}:
      (waiting, Nil{})
    case False{}:
      published(attached, observer, waiting, flags)

def release_publish(
  attached: Bool, held: Bool, observer: Bool, waiting: Waiting, flags: Boundary.Flags
) -> Published():
  match attached:
    case True{}:
      match held:
        case True{}:
          published(True{}, observer, waiting, flags)
        case False{}:
          (waiting, Nil{})
    case False{}:
      (waiting, metadata_if(observer, flags))

def led(first: Boundary.Decisions(), pair: Published()) -> Published():
  (waiting, rest) = pair
  (waiting, then(first, rest))

def tailed(pair: Published(), last: Boundary.Decisions()) -> Published():
  (waiting, rest) = pair
  (waiting, then(rest, last))

# Ends an open synchronized update and publishes the converged state.
def release(
  attached: Bool, +flags: Boundary.Flags, observer: Bool, waiting: Waiting
) -> Published():
  led(
    [Boundary.Release{}],
    release_publish(attached, Boundary.held(flags), observer, waiting, flags)
  )

# The session after a publication, with the decisions before it.
def settled(
  life: Life.Lifecycle, written: Bool, observer: Bool, explicit: Bool, pointer: Pointer,
  manage: Management.Slot, first: Boundary.Decisions(), pair: Published()
) -> Next():
  (waiting, rest) = pair
  (Session{life, written, observer, explicit, waiting, pointer, manage}, then(first, rest))

# Without a client, only the observer hears a change.
def unattached(held: Bool, observer: Bool, flags: Boundary.Flags) -> Boundary.Decisions():
  match held:
    case True{}:
      Nil{}
    case False{}:
      metadata_if(observer, flags)

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

def presented_of(pointer: Pointer) -> Bool:
  match pointer:
    case Pointer{route, presented, copied}:
      presented

# Resets libghostty's gesture and clears the host selection, keeping the copy;
# clearing a presented selection advances authority.
def unselect(+pointer: Pointer) -> Boundary.Decisions():
  [Boundary.Unselect{presented_of(pointer)}]

# Losing the client discards its waiting request, as the vertical policy's
# detach does, ends its pointer sequence, selection and copy, and refuses the
# terminal's clipboard writes from then on (ORB-C11).
def detached(
  life: Life.Lifecycle, observer: Bool, explicit: Bool, +pointer: Pointer,
  manage: Management.Slot, decisions: Boundary.Decisions()
) -> Next():
  (
    Session{
      Life.advance(life, Life.Detach{}), False{}, observer, explicit, Clear{},
      Pointer{Selection.Idle{}, False{}, False{}}, manage
    },
    then(unselect(pointer), Boundary.Capture{False{}} <> decisions)
  )

def with_route(route: Selection.Route, pointer: Pointer) -> Pointer:
  match pointer:
    case Pointer{old, presented, copied}:
      Pointer{route, presented, copied}

def on_route(route: Selection.Route, session: Session) -> Session:
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      Session{life, written, observer, explicit, waiting, with_route(route, pointer), manage}

# A closed PTY ends a sequence on either route and clears the host selection,
# keeping the copy.
def ended(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, +pointer, manage}:
      (
        Session{
          life, written, observer, explicit, waiting, with_route(Selection.Idle{}, pointer), manage
        },
        unselect(pointer)
      )

# Unselect leaves no host sequence or selection; a terminal sequence keeps its
# route.
def unselected(pointer: Pointer) -> Pointer:
  match pointer:
    case Pointer{route, presented, copied}:
      match route:
        case Selection.Terminal{}:
          Pointer{Selection.Terminal{}, False{}, copied}
        case _:
          Pointer{Selection.Idle{}, False{}, copied}

def unselected_session(session: Session) -> Session:
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      Session{life, written, observer, explicit, waiting, unselected(pointer), manage}

# Carries the proved admission answer (src/admission.bend) into decisions.
def admitted(
  answer: Admission.Answer, life: Life.Lifecycle, written: Bool, +observer: Bool,
  explicit: Bool, waiting: Waiting, pointer: Pointer, manage: Management.Slot,
  +flags: Boundary.Flags
) -> Next():
  match answer:
    # An attached client receives the terminal's clipboard writes (ORB-C11).
    case Admission.Admit{}:
      settled(
        Life.advance(life, Life.Attach{}), False{}, observer, explicit, pointer, manage,
        [Boundary.Attach{}, Boundary.Capture{True{}}],
        publish(Boundary.held(flags), True{}, observer, waiting, flags)
      )
    case Admission.Observing{}:
      (
        Session{life, written, True{}, explicit, waiting, pointer, manage},
        Boundary.Observing{} <> unattached(Boundary.held(flags), True{}, flags)
      )
    case Admission.Busy{}:
      (Session{life, written, observer, explicit, waiting, pointer, manage}, [Boundary.Busy{}])
    case Admission.Refuse{}:
      (Session{life, written, observer, explicit, waiting, pointer, manage}, [Boundary.Refuse{}])
    case Admission.Drop{}:
      (Session{life, written, observer, explicit, waiting, pointer, manage}, [Boundary.Drop{}])
    case Admission.Reject{}:
      detached(
        life, observer, explicit, pointer, manage,
        [Boundary.Reject{Boundary.ReasonRoleRepeated{}}, Boundary.Dismiss{}]
      )
    case Admission.Quiet{}:
      (Session{life, written, observer, explicit, waiting, pointer, manage}, Nil{})
    case Admission.TurnAway{}:
      (Session{life, written, observer, explicit, waiting, pointer, manage}, [Boundary.TurnAway{}])
    case Admission.Expel{}:
      (
        Session{life, written, False{},
        explicit, waiting, pointer, manage}, [Boundary.RefuseObserver{}]
      )

def admit(signal: Admission.Signal, flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{+life, written, +observer, explicit, waiting, pointer, manage}:
      admitted(
        Admission.answer(Admission.Slots{is_attached(life), observer}, signal),
        life, written, observer, explicit, waiting, pointer, manage, flags
      )

def started(pid: U32, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      (Session{Life.Detached{pid}, written, observer, explicit, waiting, pointer, manage}, Nil{})

# The terminal's clipboard writes from a PTY read go to the attached client
# first, before the frame from the same read, even while a synchronized update
# holds that frame (ORB-C11).
def clipboard_if(writes: Bool) -> Boundary.Decisions():
  match writes:
    case True{}:
      [Boundary.Clipboard{}]
    case False{}:
      Nil{}

# Every PTY read publishes a frame to an attached client, and the observer
# hears the change with or without one. A read whose clipboard writes passed
# their bound ends an attached client's attachment instead, without sending any
# of them; the observer still hears the change. Without
# a client there is no attachment to end.
def output_writes(overflowed: Bool, +flags: Boundary.Flags, session: Session) -> Next():
  match overflowed:
    case True{}:
      match session:
        case Session{life, written, +observer, explicit, waiting, pointer, manage}:
          detached(
            life, observer, explicit, pointer, manage,
            Boundary.Detach{} <> unattached(Boundary.held(flags), observer, flags)
          )
    case False{}:
      match session:
        case Session{+life, written, +observer, explicit, waiting, pointer, manage}:
          settled(
            life, written, observer, explicit, pointer, manage,
            clipboard_if(Boundary.clipboard(flags)),
            publish(Boundary.held(flags), is_attached(life), observer, waiting, flags)
          )

def output(+flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{+life, written, observer, explicit, waiting, pointer, manage}:
      output_writes(
        Bool.and(Boundary.clipboard_overflow(flags), is_attached(life)), flags,
        Session{life, written, observer, explicit, waiting, pointer, manage}
      )

# The watchdog ended a synchronized update that the child left open.
def expired(flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{+life, written, +observer, explicit, waiting, pointer, manage}:
      settled(
        life, written, observer, explicit, pointer, manage, Nil{},
        release(is_attached(life), flags, observer, waiting)
      )

# The stop releases a held update first, and waits for the host to reap the
# child before anyone hears Exited.
def exit_stop(flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{+life, written, +observer, explicit, waiting, pointer, manage}:
      settled(
        Life.advance(life, Life.BeginStop{}), written, observer, explicit, pointer, manage, Nil{},
        tailed(release(is_attached(life), flags, observer, waiting), [Boundary.Reap{}])
      )

def wrote(+published: Bool, +flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{life, written, +observer, explicit, waiting, pointer, manage}:
      settled(
        life, True{}, observer, explicit, pointer, manage, [Boundary.Accepted{}],
        publish(
          Bool.or(Bool.not(published), Boundary.held(flags)), True{}, observer, waiting, flags
        )
      )

# Input the PTY queue cannot hold fails with Failure(Terminal) and ends only
# this attachment.
def overflowed(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      detached(
        life, observer, explicit, pointer, manage,
        [Boundary.Reject{Boundary.ReasonPtyFull{}}, Boundary.Dismiss{}]
      )

# A key that wrote PTY bytes returns a scrolled-back primary viewport to live
# output and publishes it (ORB-C8).
def returned_live(+flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{life, written, +observer, explicit, waiting, pointer, manage}:
      settled(
        life, True{}, observer, explicit, pointer, manage,
        Boundary.Accepted{} <> moved(Boundary.Bottom{}),
        publish(Boundary.held(flags), True{}, observer, waiting, flags)
      )

def keyed(above: Bool, flags: Boundary.Flags, session: Session) -> Next():
  match above:
    case True{}:
      returned_live(flags, session)
    case False{}:
      wrote(False{}, flags, session)

# Plain input was queued for the PTY or a resize was applied: it is accepted,
# a resize publishes a frame, and either keeps a half-closed client attached
# for later output. A request that came after the PTY closed fails and keeps
# the client.
def applied(
  effect: Boundary.Effect, +flags: Boundary.Flags, session: Session
) -> Next():
  match effect:
    case Boundary.EffectQuiet{}:
      (session, [Boundary.Accepted{}])
    case Boundary.EffectWrite{}:
      wrote(False{}, flags, session)
    # A resize ends a sequence on either route and clears the host selection,
    # keeping the copy; the resize advances authority.
    case Boundary.EffectResize{}:
      prefixed(
        [Boundary.Unselect{False{}}], wrote(True{}, flags, on_route(Selection.Idle{}, session))
      )
    case Boundary.EffectFull{}:
      overflowed(session)
    case Boundary.EffectKey{}:
      keyed(Boundary.above_live(flags), flags, session)
    case Boundary.EffectClosed{}:
      (session, reject(Boundary.ReasonPtyClosed{}))

def closed_when(
  full: Bool, written: Bool, life: Life.Lifecycle, observer: Bool, explicit: Bool,
  waiting: Waiting, pointer: Pointer, manage: Management.Slot
) -> Next():
  match full:
    case False{}:
      match written:
        case True{}:
          (Session{life, True{}, observer, explicit, waiting, pointer, manage}, Nil{})
        case False{}:
          detached(life, observer, explicit, pointer, manage, [Boundary.Detach{}])
    case True{}:
      detached(life, observer, explicit, pointer, manage, [Boundary.Detach{}])

# A full close ends the attachment; a half-close does only before any accepted
# input or resize.
def closed(full: Bool, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      closed_when(full, written, life, observer, explicit, waiting, pointer, manage)

# ORB-C7: a client whose output passed its bound is detached, not waited on.
def pressured(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      detached(life, observer, explicit, pointer, manage, [Boundary.Detach{}])

# The finish of a stop through the lease is explicit (ORB-C13).
def reaped(code: U32, session: Session) -> Next():
  match session:
    case Session{life, written, observer, +explicit, waiting, pointer, manage}:
      (
        Session{Life.finish_stop(life), written, observer, explicit, waiting, pointer, manage},
        [Boundary.Finish{code, explicit}]
      )

# The lease's stop found the child already exited, so the run ends naturally.
def exited_before_stop(code: U32, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      reaped(code, Session{life, written, observer, False{}, waiting, pointer, manage})

def refused_with(session: Session, decisions: Boundary.Decisions()) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      detached(life, observer, explicit, pointer, manage, decisions)

# A request that fails to decode gets Failure(Protocol), or nothing for another
# ORBS version, and ends only this attachment.
def refused(version: Bool, session: Session) -> Next():
  match version:
    case False{}:
      refused_with(session, [Boundary.RefuseRequest{}, Boundary.Dismiss{}])
    case True{}:
      refused_with(session, [Boundary.Detach{}])

# The observer closed, failed, or could not take its metadata (ORB-C7): Bend
# drops it rather than wait on it.
def observer_gone(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      (
        Session{life, written, False{}, explicit, waiting, pointer, manage},
        [Boundary.DropObserver{}]
      )

def slot(waiting: Waiting) -> Vertical.Slot:
  match waiting:
    case Clear{}:
      Vertical.Empty{}
    case Waiting{request}:
      Vertical.Waiting{}

# The policy defers a request only into a clear slot.
def deferred(request: Request, session: Session) -> Session:
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      Session{life, written, observer, explicit, Waiting{request}, pointer, manage}

# Carries the proved vertical answer (src/vertical.bend) to a new request into
# the session, where a deferred request waits. The policy answers a request
# with none of the others.
def requested(
  answer: Vertical.Answer, request: Request, flags: Boundary.Flags, session: Session
) -> Next():
  match answer:
    case Vertical.Reply{}:
      (session, answered(request, False{}, flags))
    case Vertical.Defer{}:
      (deferred(request, session), Nil{})
    case Vertical.RejectBusy{}:
      (session, reject(Boundary.ReasonVerticalBusy{}))
    case Vertical.RejectAhead{}:
      (session, reject(Boundary.ReasonVerticalAhead{}))
    case _:
      (session, Nil{})

# Every vertical request asks the proved policy.
def vertical(
  +request: Request, future: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match session:
    case Session{life, written, observer, explicit, +waiting, pointer, manage}:
      requested(
        Vertical.answer(
          slot(waiting),
          Vertical.Request{future, routing(request, flags), Boundary.held(flags)}
        ),
        request, flags, Session{life, written, observer, explicit, waiting, pointer, manage}
      )

# A terminal-routed wheel sends its input, which keeps a half-closed client
# attached for later output, and answers TerminalRouted.
def routed_into(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      (
        Session{life, True{},
        observer, explicit, waiting, pointer, manage}, [Boundary.WheelAnswer{True{}, unstamped()}]
      )

# A full PTY input queue ends the attachment like other input.
def routed_wheel(input: Boundary.WheelInput, session: Session) -> Next():
  match input:
    case Boundary.WheelInputFits{}:
      routed_into(session)
    case Boundary.WheelInputEmpty{}:
      (session, reject(Boundary.ReasonWheelOutside{}))
    case Boundary.WheelInputNoOutcomeRoom{}:
      (session, reject(Boundary.ReasonWheelNoRoom{}))
    case Boundary.WheelInputPtyFull{}:
      overflowed(session)

# Otherwise the wheel moves the viewport one row, unless a synchronized update
# is open or the client's queue could not take a frame. Unlike a vertical
# request, a wheel is never deferred.
def viewport_wheel(held: Bool, room: Bool) -> Boundary.Decisions():
  match held:
    case True{}:
      reject(Boundary.ReasonWheelSynchronized{})
    case False{}:
      match room:
        case True{}:
          moved(Boundary.WheelAnswer{False{}, unstamped()})
        case False{}:
          reject(Boundary.ReasonWheelNoRoom{})

# A physical vertical wheel goes to the terminal while mouse tracking or
# alternate-screen mode 1007 owns it (ORB-C8).
def wheeled(
  routed: Bool, input: Boundary.WheelInput, +flags: Boundary.Flags, session: Session
) -> Next():
  match routed:
    case True{}:
      routed_wheel(input, session)
    case False{}:
      (session, viewport_wheel(Boundary.held(flags), Boundary.frame_room(flags)))

# The host reports a pointer request with the facts the selection policy needs
# in its flags; whether a copy exists, and whether a press is ahead of
# authority, are Bend's.
def selection_signal(
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

def idle_reason(kind: Boundary.SelectionKind) -> Boundary.Reason:
  match kind:
    case Boundary.SelectionKindUpdate{}:
      Boundary.ReasonUpdateIdle{}
    case _:
      Boundary.ReasonFinishIdle{}

# A host phase needs its position in the viewport and room for its answer and
# frame before the gesture moves; Rust applies it and reports how it went. The
# sequence takes the route the policy gives it once the phase is carried out.
def host_phase(
  outside: Bool, room: Bool, next: Selection.Route, session: Session
) -> Next():
  match outside:
    case True{}:
      (session, reject(Boundary.ReasonSelectionOutside{}))
    case False{}:
      match room:
        case True{}:
          (on_route(next, session), [Boundary.SelectHost{}])
        case False{}:
          (session, reject(Boundary.ReasonSelectionNoRoom{}))

def pointer_response(kind: Boundary.SelectionKind) -> Boundary.Decision:
  match kind:
    case Boundary.SelectionKindFinish{}:
      Boundary.SelectionFinished{unstamped()}
    case _:
      Boundary.Accepted{}

# Publishes the current terminal as any change does.
def republished(+flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{+life, written, +observer, explicit, waiting, pointer, manage}:
      settled(
        life, written, observer, explicit, pointer, manage, Nil{},
        publish(Boundary.held(flags), is_attached(life), observer, waiting, flags)
      )

# A press on the terminal's route resets the gesture and clears the host
# selection and its copy; clearing a presented selection advances authority
# and publishes a frame.
def pressed_terminal(presented: Bool, +flags: Boundary.Flags, session: Session) -> Next():
  match presented:
    case True{}:
      republished(flags, session)
    case False{}:
      (session, Nil{})

def forgot(session: Session) -> Session:
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      match pointer:
        case Pointer{route, presented, copied}:
          Session{
            life, written, observer, explicit, waiting, Pointer{route, False{}, False{}}, manage
          }

def terminal_press(+presented: Bool, +flags: Boundary.Flags, session: Session) -> Next():
  prefixed(
    [Boundary.Unselect{presented}, Boundary.SelectTerminal{}, Boundary.Accepted{}],
    pressed_terminal(presented, flags, forgot(session))
  )

def terminal_sent(
  +kind: Boundary.SelectionKind, presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match kind:
    case Boundary.SelectionKindBegin{}:
      terminal_press(presented, flags, session)
    case _:
      (session, [Boundary.SelectTerminal{}, pointer_response(kind)])

# The room a terminal phase needs: its answer, and a frame when a press clears
# a presented host selection.
def terminal_room(kind: Boundary.SelectionKind, presented: Bool, +flags: Boundary.Flags) -> Bool:
  match kind:
    case Boundary.SelectionKindBegin{}:
      match presented:
        case True{}:
          Boundary.result_frame_room(flags)
        case False{}:
          Boundary.terminal_room(flags)
    case _:
      Boundary.terminal_room(flags)

# A terminal phase sends the pointer as mouse input. Input the PTY queue
# cannot hold fails with Failure(Terminal) and ends the attachment, as other
# input does.
def terminal_phase(
  room: Bool, full: Bool, +kind: Boundary.SelectionKind, presented: Bool,
  +flags: Boundary.Flags, next: Selection.Route, session: Session
) -> Next():
  match room:
    case False{}:
      (session, reject(Boundary.ReasonPointerNoRoom{}))
    case True{}:
      match full:
        case True{}:
          overflowed(session)
        case False{}:
          terminal_sent(kind, presented, flags, on_route(next, session))

# A Cancel ends the sequence on either route and clears the host selection,
# keeping the copy; it advances authority, so the frame that follows is a new
# revision.
def cancelled(
  room: Bool, +flags: Boundary.Flags, next: Selection.Route, session: Session
) -> Next():
  match room:
    case True{}:
      prefixed(
        [Boundary.Unselect{True{}}, Boundary.Accepted{}],
        republished(flags, on_route(next, session))
      )
    case False{}:
      (session, reject(Boundary.ReasonSelectionNoRoom{}))

# Carries the proved selection answer (src/selection.bend) into decisions, and
# the route it leads to into the session once the answer is carried out.
def selected_answer(
  answer: Selection.Answer, next: Selection.Route, +kind: Boundary.SelectionKind,
  +presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match answer:
    case Selection.RejectAhead{}:
      (session, reject(Boundary.ReasonSelectionAhead{}))
    case Selection.RejectActive{}:
      (session, reject(Boundary.ReasonSelectionActive{}))
    case Selection.RejectIdle{}:
      (session, reject(idle_reason(kind)))
    case Selection.RejectNoCopy{}:
      (session, reject(Boundary.ReasonNoCopy{}))
    case Selection.HostPhase{}:
      host_phase(Boundary.outside(flags), Boundary.host_room(flags), next, session)
    case Selection.TerminalPhase{}:
      terminal_phase(
        terminal_room(kind, presented, flags), Boundary.pty_full(flags), kind, presented,
        flags, next, session
      )
    case Selection.Cancelled{}:
      cancelled(Boundary.result_frame_room(flags), flags, next, session)
    case Selection.Copied{}:
      (session, [Boundary.Copied{False{}}])
    case Selection.Quiet{}:
      (session, Nil{})

def pointed(
  +route: Selection.Route, +signal: Selection.Signal, +kind: Boundary.SelectionKind,
  presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  selected_answer(
    Selection.answer(route, signal), Selection.next(route, signal), kind, presented, flags,
    session
  )

def pointer(
  +kind: Boundary.SelectionKind, future: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      match pointer:
        case Pointer{+route, +presented, +copied}:
          pointed(
            route, selection_signal(kind, copied, future, flags), kind, presented, flags,
            Session{
              life, written, observer, explicit, waiting, Pointer{route, presented, copied}, manage
            }
          )

def finished_host(copied: Bool, pair: Next()) -> Next():
  match copied:
    case True{}:
      prefixed([Boundary.SelectionFinished{unstamped()}], followed(pair, [Boundary.Copied{True{}}]))
    case False{}:
      prefixed([Boundary.SelectionFinished{unstamped()}], pair)

# What a host phase or tick leaves: the presentation libghostty reports; a
# press forgets the last copy and a finish keeps one exactly when a selection
# is presented; a failure cleared the sequence and the selection.
def shown(pointer: Pointer, outcome: Boundary.Selected, +presented: Bool) -> Pointer:
  match pointer:
    case Pointer{route, old, copied}:
      match outcome:
        case Boundary.SelectedApplied{}:
          Pointer{route, presented, copied}
        case Boundary.SelectedFinished{}:
          Pointer{route, presented, presented}
        case Boundary.SelectedFailed{}:
          Pointer{Selection.Idle{}, False{}, copied}
        case Boundary.SelectedBegan{}:
          Pointer{route, presented, False{}}
        case Boundary.SelectedTicked{}:
          Pointer{route, presented, copied}

# A host phase that applied is answered and published; a finished one names
# the revision of its frame and then returns the frozen text for the selection
# clipboard (ORB-C9). An applied tick has already sent its outcome.
def host_answered(
  outcome: Boundary.Selected, presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match outcome:
    case Boundary.SelectedApplied{}:
      prefixed([Boundary.Accepted{}], republished(flags, session))
    case Boundary.SelectedBegan{}:
      prefixed([Boundary.Accepted{}], republished(flags, session))
    case Boundary.SelectedFinished{}:
      finished_host(presented, republished(flags, session))
    case Boundary.SelectedFailed{}:
      (session, reject(Boundary.ReasonSelectionFailed{}))
    case Boundary.SelectedTicked{}:
      (session, Nil{})

def host_selected(
  +outcome: Boundary.Selected, +presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      host_answered(
        outcome, presented, flags,
        Session{
          life, written, observer, explicit, waiting, shown(pointer, outcome, presented), manage
        }
      )

# An upward selection tick runs only for a held host gesture on the primary
# screen.
def holds_gesture(route: Selection.Route, alternate: Bool) -> Bool:
  match route:
    case Selection.Host{}:
      Bool.not(alternate)
    case _:
      False{}

# A held tick runs only from the top row while its anchor is valid; a lost
# anchor ends the sequence. Otherwise it is a vertical request like a scroll,
# deferred while a synchronized update is open (ORB-C9).
def tick_held(
  held: Bool, state: Boundary.TickState, x: U32, y: U32, flags: Boundary.Flags,
  session: Session
) -> Next():
  match held:
    case False{}:
      (session, reject(Boundary.ReasonTickNoGesture{}))
    case True{}:
      match state:
        case Boundary.TickStateFits{}:
          vertical(TickRequest{x, y}, False{}, flags, session)
        case Boundary.TickStateNotTopRow{}:
          (session, reject(Boundary.ReasonTickNotTopRow{}))
        case Boundary.TickStateAnchorLost{}:
          followed(ended(session), reject(Boundary.ReasonAnchorLost{}))

def ticked(
  state: Boundary.TickState, x: U32, y: U32, +flags: Boundary.Flags, session: Session
) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      match pointer:
        case Pointer{+route, presented, copied}:
          tick_held(
            holds_gesture(route, Boundary.alternate(flags)), state, x, y, flags,
            Session{
              life, written, observer, explicit, waiting, Pointer{route, presented, copied}, manage
            }
          )

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

# A stop through the lease begins the reap-gated stop: the host hangs up the
# child with the bounded shutdown of ORB-C12 and reports its reap, and the
# session remembers that the stop is explicit.
def shut_down(session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      (
        Session{
          Life.advance(life, Life.BeginStop{}), written, observer, True{}, waiting, pointer, manage
        },
        [Boundary.ManageStop{}]
      )

# Carries the proved management answer (src/management.bend) into decisions.
def managed(answer: Management.Answer, session: Session) -> Next():
  match answer:
    case Management.Admit{}:
      (session, [Boundary.ManageAdmit{}])
    case Management.Busy{}:
      (session, [Boundary.ManageBusy{}])
    case Management.Lease{}:
      (session, [Boundary.ManageLease{}])
    case Management.RejectIdentity{}:
      (session, [Boundary.ManageRejectIdentity{}])
    case Management.RejectRequest{}:
      (session, [Boundary.ManageReject{}])
    case Management.Report{}:
      (session, [Boundary.ManageStatus{}])
    case Management.Records{}:
      (session, [Boundary.ManageProgramStatus{}])
    case Management.Refuse{}:
      (session, [Boundary.ManageRefuse{}])
    case Management.Shutdown{}:
      shut_down(session)
    case Management.Quiet{}:
      (session, Nil{})

# Bend's session holds the management slot; the proved policy
# (src/management.bend) answers each event and gives the slot after Rust
# carries out the answer.
def management(+signal: Management.Signal, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, +manage}:
      managed(
        Management.answer(manage, signal),
        Session{life, written, observer, explicit, waiting, pointer, Management.next(manage, signal)}
      )

# A ProgramStatus request (ORB-C15) is answered while the child runs; its
# foreground sample's lifetime decision already leads the step.
def program_status(session: Session) -> Next():
  match session:
    case Session{+life, written, observer, explicit, waiting, pointer, manage}:
      management(
        Management.ProgramStatus{runs(life)},
        Session{life, written, observer, explicit, waiting, pointer, manage}
      )

def drained_stop(flags: Boundary.Flags, pair: Next()) -> Next():
  (session, first) = pair
  prefixed(first, exit_stop(flags, session))

# The last output, which the host drained from the PTY, and a cleared
# presented selection publish as any output does, as one change of presented
# state: the clear advances authority only without drained output, which
# advanced it already. Then the stop begins.
def exit_cleared(
  drained: Bool, presented: Bool, +flags: Boundary.Flags, session: Session
) -> Next():
  match drained:
    case True{}:
      prefixed([Boundary.Unselect{False{}}], drained_stop(flags, output(flags, session)))
    case False{}:
      match presented:
        case True{}:
          prefixed([Boundary.Unselect{True{}}], drained_stop(flags, output(flags, session)))
        case False{}:
          prefixed([Boundary.Unselect{False{}}], exit_stop(flags, session))

# The direct child exited and the host reaped it, even while a descendant
# holds the PTY open (ORB-C12). The exit ends a pointer sequence on either
# route and clears the host selection and its copy (ORB-C9).
def exited(drained: Bool, +flags: Boundary.Flags, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      exit_cleared(
        drained, presented_of(pointer), flags,
        Session{
          life, written, observer, explicit, waiting, Pointer{Selection.Idle{}, False{}, False{}},
          manage
        }
      )

# A request names the revision of the frame it was made on; it is from the
# future when that revision is ahead of authority. Bend compares them once per
# fact, so decide sees only the answer.
def future_of(fact: Boundary.Fact, authority: Revision.Revision) -> Bool:
  match fact:
    case Boundary.PreviewVertical{revision, direction}:
      Revision.ahead(revision, authority)
    case Boundary.ScrollVertical{revision, rows}:
      Revision.ahead(revision, authority)
    case Boundary.Selection{kind, revision}:
      Revision.ahead(revision, authority)
    case _:
      False{}

def decide(
  fact: Boundary.Fact, +flags: Boundary.Flags, future: Bool, session: Session
) -> Next():
  match fact:
    case Boundary.Started{pid}:
      started(pid, session)
    case Boundary.Output{foreground, marks}:
      output(flags, session)
    # The PTY closed while the direct child lives: nothing more is read or
    # written, a pointer sequence ends, and the run waits for the child's
    # exit (ORB-C1).
    case Boundary.PtyClosed{}:
      ended(session)
    case Boundary.Peer{message}:
      admit(peer_signal(message), flags, session)
    case Boundary.Applied{effect}:
      applied(effect, flags, session)
    case Boundary.Closed{full}:
      closed(full, session)
    case Boundary.Pressured{}:
      pressured(session)
    case Boundary.Reaped{status}:
      reaped(status, session)
    case Boundary.ExitedBeforeStop{status}:
      exited_before_stop(status, session)
    case Boundary.RoleRepeated{}:
      admit(Admission.Repeat{}, flags, session)
    case Boundary.Refused{version}:
      refused(version, session)
    case Boundary.ObserverGone{loss}:
      observer_gone(session)
    case Boundary.Expired{}:
      expired(flags, session)
    case Boundary.PreviewVertical{revision, direction}:
      vertical(PreviewRequest{direction}, future, flags, session)
    case Boundary.ScrollVertical{revision, rows}:
      vertical(ScrollRequest{rows}, future, flags, session)
    case Boundary.ReturnToLive{}:
      vertical(LiveRequest{}, False{}, flags, session)
    case Boundary.Wheel{input}:
      wheeled(Boundary.routed(flags), input, flags, session)
    case Boundary.Selection{kind, revision}:
      pointer(kind, future, flags, session)
    case Boundary.Selected{outcome, presented}:
      host_selected(outcome, presented, flags, session)
    case Boundary.Tick{state, x, y}:
      ticked(state, x, y, flags, session)
    case Boundary.Management{event}:
      management(management_signal(event), session)
    case Boundary.ProgramStatus{foreground}:
      program_status(session)
    case Boundary.ChildExited{drained}:
      exited(drained, flags, session)
    case Boundary.Crowded{}:
      admit(Admission.Crowded{}, flags, session)
    case Boundary.ObserverMessage{}:
      admit(Admission.Spoke{}, flags, session)
    # serve answers these itself.
    case Boundary.NotReaped{}:
      (session, Nil{})
    case Boundary.Finished{}:
      (session, Nil{})
    case Boundary.Fault{pid}:
      (session, Nil{})

def carried_one(decision: Boundary.Decision, pointer: Pointer) -> Pointer:
  match decision:
    case Boundary.Unselect{advance}:
      unselected(pointer)
    case _:
      pointer

# The pointer state after the decisions are carried out: every Unselect, such
# as one before a viewport move a publication resolves, ends a host sequence
# and clears the selection.
def carried(decisions: Boundary.Decisions(), pointer: Pointer) -> Pointer:
  match decisions:
    case Nil{}:
      pointer
    case Con{decision, rest}:
      carried(rest, carried_one(decision, pointer))

def carry_pointer(+decisions: List<&2, Boundary.Decision>, session: Session) -> Next():
  match session:
    case Session{life, written, observer, explicit, waiting, pointer, manage}:
      (
        Session{life, written, observer, explicit, waiting, carried(decisions, pointer), manage},
        decisions
      )

def settle(pair: Next()) -> Next():
  (session, decisions) = pair
  carry_pointer(decisions, session)

# Output that left the gesture without an anchor, or newly enabled mouse
# tracking, ends a host sequence and clears the host selection, keeping the
# copy; compatible output keeps them. The host notes it
# with the fact that follows the output, and the clear does not advance
# authority.
def anchored(
  unanchored: Bool, fact: Boundary.Fact, +flags: Boundary.Flags, future: Bool,
  session: Session
) -> Next():
  match unanchored:
    case True{}:
      prefixed(
        [Boundary.Unselect{False{}}],
        settle(decide(fact, flags, future, unselected_session(session)))
      )
    case False{}:
      settle(decide(fact, flags, future, session))

# Authority advances once for each change of presented state (ORB-C4). A
# fact the host applied before Bend answers advances it first: a PTY turn that
# fed output to the terminal, unless a synchronized update was held before it,
# so the update's frame takes the revision its first turn advanced to; a
# resize; and a host phase or tick that applied.
def output_advanced(held: Bool, revision: Revision.Revision) -> Revision.Revision:
  match held:
    case True{}:
      revision
    case False{}:
      Revision.next(revision)

def resized(effect: Boundary.Effect, revision: Revision.Revision) -> Revision.Revision:
  match effect:
    case Boundary.EffectResize{}:
      Revision.next(revision)
    case _:
      revision

def applied_phase(outcome: Boundary.Selected, revision: Revision.Revision) -> Revision.Revision:
  match outcome:
    case Boundary.SelectedFailed{}:
      revision
    case _:
      Revision.next(revision)

def drained_advanced(
  drained: Bool, held: Bool, revision: Revision.Revision
) -> Revision.Revision:
  match drained:
    case True{}:
      output_advanced(held, revision)
    case False{}:
      revision

def fact_advanced(
  fact: Boundary.Fact, held: Bool, revision: Revision.Revision
) -> Revision.Revision:
  match fact:
    case Boundary.Output{foreground, marks}:
      output_advanced(held, revision)
    case Boundary.ChildExited{drained}:
      drained_advanced(drained, held, revision)
    case Boundary.Applied{effect}:
      resized(effect, revision)
    case Boundary.Selected{outcome, presented}:
      applied_phase(outcome, revision)
    case _:
      revision

def moved_unless(routed: Bool, revision: Revision.Revision) -> Revision.Revision:
  match routed:
    case True{}:
      revision
    case False{}:
      Revision.next(revision)

# Then each decision that changes presented state advances it as it is carried
# out: a clear of a presented selection and every viewport move.
def decision_advanced(
  decision: Boundary.Decision, revision: Revision.Revision
) -> Revision.Revision:
  match decision:
    case Boundary.Unselect{advance}:
      moved_unless(Bool.not(advance), revision)
    case Boundary.Bottom{}:
      Revision.next(revision)
    case Boundary.LiveAnswer{old}:
      Revision.next(revision)
    case Boundary.ScrollAnswer{rows, routed, old}:
      moved_unless(routed, revision)
    case Boundary.WheelAnswer{routed, old}:
      moved_unless(routed, revision)
    case _:
      revision

# A publication names the revision it presents. A tick's outcome names the
# revision its move advances to once the tick applies (fact Selected Ticked).
def presented_at(decision: Boundary.Decision, +revision: Revision.Revision) -> Boundary.Decision:
  match decision:
    case Boundary.Frame{old}:
      Boundary.Frame{revision}
    case Boundary.Metadata{old, ordered}:
      Boundary.Metadata{revision, ordered}
    case Boundary.PreviewAnswer{direction, routed, old}:
      Boundary.PreviewAnswer{direction, routed, revision}
    case Boundary.WheelAnswer{routed, old}:
      Boundary.WheelAnswer{routed, revision}
    case Boundary.ScrollAnswer{rows, routed, old}:
      Boundary.ScrollAnswer{rows, routed, revision}
    case Boundary.LiveAnswer{old}:
      Boundary.LiveAnswer{revision}
    case Boundary.SelectionFinished{old}:
      Boundary.SelectionFinished{revision}
    case Boundary.TickAnswer{x, y, old}:
      Boundary.TickAnswer{x, y, Revision.next(revision)}
    case other:
      other

def stamp(decisions: Boundary.Decisions(), +revision: Revision.Revision) -> Boundary.Decisions():
  match decisions:
    case Nil{}:
      Nil{}
    case Con{+decision, rest}:
      Con{
        presented_at(decision, decision_advanced(decision, revision)),
        stamp(rest, decision_advanced(decision, revision))
      }

def stamped_revision(
  decisions: Boundary.Decisions(), revision: Revision.Revision
) -> Revision.Revision:
  match decisions:
    case Nil{}:
      revision
    case Con{+decision, rest}:
      stamped_revision(rest, decision_advanced(decision, revision))

# A Release ends the synchronized update the host held.
def released(decision: Boundary.Decision, held: Bool) -> Bool:
  match decision:
    case Boundary.Release{}:
      False{}
    case _:
      held

def held_after(decisions: Boundary.Decisions(), held: Bool) -> Bool:
  match decisions:
    case Nil{}:
      held
    case Con{+decision, rest}:
      held_after(rest, released(decision, held))

def stamped(
  +revision: Revision.Revision, held: Bool, +decisions: List<&2, Boundary.Decision>,
  status: Status.Tracker, session: Session
) -> Stepped():
  (
    State{
      session, Shown{stamped_revision(decisions, revision), held_after(decisions, held)}, status
    },
    stamp(decisions, revision)
  )

def begun(
  +revision: Revision.Revision, held: Bool, lifetime: Boundary.Decisions(),
  status: Status.Tracker, pair: Next()
) -> Stepped():
  (session, decisions) = pair
  stamped(revision, held, then(lifetime, decisions), status, session)

def presented(
  +fact: Boundary.Fact, +flags: Boundary.Flags, shown: Shown, lifetime: Boundary.Decisions(),
  status: Status.Tracker, session: Session
) -> Stepped():
  match shown:
    case Shown{+revision, held}:
      begun(
        fact_advanced(fact, held, revision), Boundary.held(flags), lifetime, status,
        anchored(Boundary.unanchored(flags), fact, flags, future_of(fact, revision), session)
      )

# A PTY turn's program status lifetime (ORB-C15) leads its decisions and moves
# the tracker, and so does a ProgramStatus request's foreground sample, which
# carries no prompt marks; every other fact keeps the tracker.
def lifetime_of(
  fact: Boundary.Fact, +status: Status.Tracker
) -> Status.Tracker & Boundary.Decisions():
  match fact:
    case Boundary.Output{+foreground, +marks}:
      (Status.after(status, foreground, marks), [Status.lifetime(status, foreground, marks)])
    case Boundary.ProgramStatus{+foreground}:
      (Status.after(status, foreground, False{}), [Status.lifetime(status, foreground, False{})])
    case _:
      (status, Nil{})

def tracked(
  pair: Status.Tracker & Boundary.Decisions(), +fact: Boundary.Fact, +flags: Boundary.Flags,
  shown: Shown, session: Session
) -> Stepped():
  (status, lifetime) = pair
  presented(fact, flags, shown, lifetime, status, session)

def step(+fact: Boundary.Fact, +flags: Boundary.Flags, state: State) -> Stepped():
  match state:
    case State{session, shown, status}:
      tracked(lifetime_of(fact, status), fact, flags, shown, session)
