import Base
import ./LAWS.bend as Laws
import ./src/lifecycle.bend as Orbit
import ./src/vertical.bend as Vertical
import ./src/admission.bend as Admission
import ./src/selection.bend as Selection
import ./src/management.bend as Management
import ./src/revision.bend as Revision
import ./proof/word.bend as Words
import ./src/session.bend as Session
import ./src/boundary.bend as Boundary
import ./src/status.bend as Status
import ./proof/step.bend as Step
import ./proof/view.bend as Views
import ./proof/lease.bend as Leases

def Laws.detach_preserves_pid(pid):
  {==}

def Laws.reconnect_preserves_pid(pid):
  {==}

def Laws.bounded_stop_preserves_pid(pid):
  {==}

def advance_preserves_stopped(state: Orbit.Lifecycle, event: Orbit.BeforeReap) ->
  {Orbit.is_stopped(Orbit.advance(state, event)) == Orbit.is_stopped(state) : Bool}:
  match state event:
    case Orbit.Detached{pid} Orbit.Attach{}:
      {==}
    case Orbit.Detached{pid} Orbit.Detach{}:
      {==}
    case Orbit.Detached{pid} Orbit.BeginStop{}:
      {==}
    case Orbit.Attached{pid} Orbit.Attach{}:
      {==}
    case Orbit.Attached{pid} Orbit.Detach{}:
      {==}
    case Orbit.Attached{pid} Orbit.BeginStop{}:
      {==}
    case Orbit.Stopping{pid} Orbit.Attach{}:
      {==}
    case Orbit.Stopping{pid} Orbit.Detach{}:
      {==}
    case Orbit.Stopping{pid} Orbit.BeginStop{}:
      {==}
    case Orbit.Stopped{pid} Orbit.Attach{}:
      {==}
    case Orbit.Stopped{pid} Orbit.Detach{}:
      {==}
    case Orbit.Stopped{pid} Orbit.BeginStop{}:
      {==}

def Laws.pre_reap_events_preserve_stopped(events, state):
  match events:
    case Nil{}:
      {==}
    case Con{+event, rest}:
      Equal.trans(
        Bool,
        Orbit.is_stopped(Orbit.replay(rest, Orbit.advance(state, event))),
        Orbit.is_stopped(Orbit.advance(state, event)),
        Orbit.is_stopped(state),
        Laws.pre_reap_events_preserve_stopped(rest, Orbit.advance(state, event)),
        advance_preserves_stopped(state, event)
      )

def Laws.vertical_future_requests_fail(slot, routed, held):
  {==}

def Laws.vertical_routed_requests_answer_at_once(slot, held):
  {==}

def Laws.vertical_nothing_resolves_while_held(slot):
  match slot:
    case Vertical.Empty{}:
      {==}
    case Vertical.Waiting{}:
      {==}

def Laws.vertical_release_resolves_waiting(routed):
  match routed:
    case True{}:
      {==}
    case False{}:
      {==}

def Laws.vertical_answers_are_accounted(events, slot):
  match events:
    case Nil{}:
      {==}
    case Con{event, rest}:
      match event:
        case Vertical.Request{future, routed, held}:
          match future routed held:
            case True{} _ _:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
            case False{} True{} _:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
            case False{} False{} True{}:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
            case False{} False{} False{}:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
        case Vertical.Publish{routed, held}:
          match routed held:
            case True{} _:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
            case False{} True{}:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Waiting{})
            case False{} False{}:
              match slot:
                case Vertical.Empty{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
                case Vertical.Waiting{}:
                  Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
        case Vertical.Detach{}:
          match slot:
            case Vertical.Empty{}:
              Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})
            case Vertical.Waiting{}:
              Laws.vertical_answers_are_accounted(rest, Vertical.Empty{})

# Every signal from every slot state: the audit replays the answer to the
# slots the policy reaches, so induction carries Clean through the list.
def Laws.admission_answers_are_accounted(events, slots):
  match events:
    case Nil{}:
      {==}
    case Con{event, rest}:
      match event:
        case Admission.Hello{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
        case Admission.Observe{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
        case Admission.Malformed{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Unsupported{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Repeat{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Left{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Unobserved{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Crowded{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, True{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, True{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
        case Admission.Spoke{}:
          match slots:
            case Admission.Slots{interactive, observer}:
              match interactive observer:
                case True{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case True{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{True{}, False{}})
                case False{} True{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})
                case False{} False{}:
                  Laws.admission_answers_are_accounted(rest, Admission.Slots{False{}, False{}})

# Every signal from every route: the audit replays the answer to the route
# the policy reaches, so induction carries Clean through the list.
def Laws.selection_answers_are_accounted(events, route):
  match events:
    case Nil{}:
      {==}
    case Con{event, rest}:
      match event:
        case Selection.Begin{future, tracking, shift}:
          match future:
            case True{}:
              match route:
                case Selection.Idle{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Idle{})
                case Selection.Host{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Host{})
                case Selection.Terminal{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
            case False{}:
              match tracking shift:
                case True{} True{}:
                  match route:
                    case Selection.Idle{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Host{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Terminal{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
                case True{} False{}:
                  match route:
                    case Selection.Idle{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
                    case Selection.Host{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Terminal{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
                case False{} True{}:
                  match route:
                    case Selection.Idle{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Host{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Terminal{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
                case False{} False{}:
                  match route:
                    case Selection.Idle{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Host{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Host{})
                    case Selection.Terminal{}:
                      Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
        case Selection.Update{}:
          match route:
            case Selection.Idle{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Host{}:
              Laws.selection_answers_are_accounted(rest, Selection.Host{})
            case Selection.Terminal{}:
              Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
        case Selection.Finish{}:
          match route:
            case Selection.Idle{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Host{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Terminal{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
        case Selection.Cancel{}:
          match route:
            case Selection.Idle{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Host{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Terminal{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
        case Selection.Copy{copied}:
          match copied:
            case True{}:
              match route:
                case Selection.Idle{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Idle{})
                case Selection.Host{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Host{})
                case Selection.Terminal{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
            case False{}:
              match route:
                case Selection.Idle{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Idle{})
                case Selection.Host{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Host{})
                case Selection.Terminal{}:
                  Laws.selection_answers_are_accounted(rest, Selection.Terminal{})
        case Selection.Reset{}:
          match route:
            case Selection.Idle{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Host{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})
            case Selection.Terminal{}:
              Laws.selection_answers_are_accounted(rest, Selection.Idle{})

def Laws.selection_shift_selects_on_the_host(tracking):
  match tracking:
    case True{}:
      {==}
    case False{}:
      {==}

def Laws.selection_future_press_fails(route, tracking, shift):
  match route:
    case Selection.Idle{}:
      {==}
    case Selection.Host{}:
      {==}
    case Selection.Terminal{}:
      {==}

# Every signal from every slot: the audit replays the answer to the slot
# the policy reaches, so induction carries Clean through the list.
def Laws.management_answers_are_accounted(events, slot):
  match events:
    case Nil{}:
      {==}
    case Con{event, rest}:
      match event:
        case Management.Connect{}:
          match slot:
            case Management.Free{}:
              Laws.management_answers_are_accounted(rest, Management.Negotiating{})
            case Management.Negotiating{}:
              Laws.management_answers_are_accounted(rest, Management.Negotiating{})
            case Management.Leased{}:
              Laws.management_answers_are_accounted(rest, Management.Leased{})
            case Management.Closing{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.Acquire{valid}:
          match valid:
            case True{}:
              match slot:
                case Management.Free{}:
                  Laws.management_answers_are_accounted(rest, Management.Free{})
                case Management.Negotiating{}:
                  Laws.management_answers_are_accounted(rest, Management.Leased{})
                case Management.Leased{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
                case Management.Closing{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
            case False{}:
              match slot:
                case Management.Free{}:
                  Laws.management_answers_are_accounted(rest, Management.Free{})
                case Management.Negotiating{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
                case Management.Leased{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
                case Management.Closing{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.Status{}:
          match slot:
            case Management.Free{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Negotiating{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
            case Management.Leased{}:
              Laws.management_answers_are_accounted(rest, Management.Leased{})
            case Management.Closing{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.Stop{}:
          match slot:
            case Management.Free{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Negotiating{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
            case Management.Leased{}:
              Laws.management_answers_are_accounted(rest, Management.Leased{})
            case Management.Closing{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.ProgramStatus{live}:
          match live:
            case True{}:
              match slot:
                case Management.Free{}:
                  Laws.management_answers_are_accounted(rest, Management.Free{})
                case Management.Negotiating{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
                case Management.Leased{}:
                  Laws.management_answers_are_accounted(rest, Management.Leased{})
                case Management.Closing{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
            case False{}:
              match slot:
                case Management.Free{}:
                  Laws.management_answers_are_accounted(rest, Management.Free{})
                case Management.Negotiating{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
                case Management.Leased{}:
                  Laws.management_answers_are_accounted(rest, Management.Leased{})
                case Management.Closing{}:
                  Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.Bad{}:
          match slot:
            case Management.Free{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Negotiating{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
            case Management.Leased{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
            case Management.Closing{}:
              Laws.management_answers_are_accounted(rest, Management.Closing{})
        case Management.Gone{}:
          match slot:
            case Management.Free{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Negotiating{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Leased{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})
            case Management.Closing{}:
              Laws.management_answers_are_accounted(rest, Management.Free{})

def Laws.management_stop_needs_the_lease(slot):
  match slot:
    case Management.Free{}:
      {==}
    case Management.Negotiating{}:
      {==}
    case Management.Leased{}:
      {==}
    case Management.Closing{}:
      {==}

def Laws.management_program_status_needs_a_live_lease(slot, live):
  match slot:
    case Management.Free{}:
      match live:
        case True{}:
          {==}
        case False{}:
          {==}
    case Management.Negotiating{}:
      match live:
        case True{}:
          {==}
        case False{}:
          {==}
    case Management.Leased{}:
      match live:
        case True{}:
          {==}
        case False{}:
          {==}
    case Management.Closing{}:
      match live:
        case True{}:
          {==}
        case False{}:
          {==}

# The revision counter: an increment is ahead unless it wraps.

# Every bit of the word is set.
def word_full(n: Nat, w: Word(n)) -> Bool:
  match n:
    case 0n:
      True{}
    case 1n+(+p):
      match w:
        case WCon{False{}, t}:
          False{}
        case WCon{True{}, t}:
          word_full(p, t)

def word_ones(n: Nat) -> Word(n):
  match n:
    case 0n:
      WNil{}
    case 1n+(+p):
      WCon{True{}, word_ones(p)}

def fin_ft_gt(c: Cmp) -> {Cmp.is_gt(Word.cmp.fin(False{}, True{}, c)) == Cmp.is_gt(c) : Bool}:
  match c:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

def fin_ft_eq(c: Cmp) -> {Cmp.is_eq(Word.cmp.fin(False{}, True{}, c)) == False{} : Bool}:
  match c:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

def fin_tt(c: Cmp) -> {Word.cmp.fin(True{}, True{}, c) == c : Cmp}:
  match c:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

# An increment is greater unless every bit is set.
def inc_gt(n: Nat, w: Word(n)) ->
  {Cmp.is_gt(Word.cmp(n, Word.inc(n, w), w)) == Bool.not(word_full(n, w)) : Bool}:
  match n:
    case 0n:
      {==}
    case 1n+(+p):
      match w:
        case WCon{False{}, +t}:
          %Equal.sym(Cmp, Word.cmp(p, t, t), EQ{}, Words.cmp_refl(p, t)) :
            {Cmp.is_gt(Word.cmp.fin(True{}, False{}, _)) == True{} : Bool}
          {==}
        case WCon{True{}, +t}:
          Equal.trans(
            Bool,
            Cmp.is_gt(Word.cmp.fin(False{}, True{}, Word.cmp(p, Word.inc(p, t), t))),
            Cmp.is_gt(Word.cmp(p, Word.inc(p, t), t)),
            Bool.not(word_full(p, t)),
            fin_ft_gt(Word.cmp(p, Word.inc(p, t), t)),
            inc_gt(p, t)
          )

# A word equals the all-ones word exactly when every bit is set.
def eq_ones(n: Nat, w: Word(n)) ->
  {Cmp.is_eq(Word.cmp(n, w, word_ones(n))) == word_full(n, w) : Bool}:
  match n:
    case 0n:
      {==}
    case 1n+(+p):
      match w:
        case WCon{False{}, +t}:
          fin_ft_eq(Word.cmp(p, t, word_ones(p)))
        case WCon{True{}, +t}:
          %Equal.sym(
            Cmp, Word.cmp.fin(True{}, True{}, Word.cmp(p, t, word_ones(p))),
            Word.cmp(p, t, word_ones(p)), fin_tt(Word.cmp(p, t, word_ones(p)))
          ) : {Cmp.is_eq(_) == word_full(p, t) : Bool}
          eq_ones(p, t)

def ones_max() -> {U32{word_ones(32n)} == 4294967295 : U32}:
  {==}

def order_ft(x: Cmp, c: Cmp) ->
  {
    Cmp.is_gt(Revision.order(x, Word.cmp.fin(False{}, True{}, c))) ==
    Cmp.is_gt(Word.cmp.fin(False{}, True{}, c)) : Bool
  }:
  match c:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

# An incremented word never equals the word, so the low halves cannot decide.
def order_carried(x: Cmp, +p: Nat, w: Word(1n+p)) ->
  {
    Cmp.is_gt(Revision.order(x, Word.cmp(1n+p, Word.inc(1n+p, w), w))) ==
    Cmp.is_gt(Word.cmp(1n+p, Word.inc(1n+p, w), w)) : Bool
  }:
  match w:
    case WCon{False{}, +t}:
      %Equal.sym(Cmp, Word.cmp(p, t, t), EQ{}, Words.cmp_refl(p, t)) : {
        Cmp.is_gt(Revision.order(x, Word.cmp.fin(True{}, False{}, _))) ==
        Cmp.is_gt(Word.cmp.fin(True{}, False{}, _)) : Bool
      }
      {==}
    case WCon{True{}, +t}:
      order_ft(x, Word.cmp(p, Word.inc(p, t), t))

# The carry case: the high half increments.
def wrapped_ahead(+high: U32, +low: U32) ->
  {
    Revision.ahead(Revision.carry(True{}, high, low), Revision.Revision{high, low}) ==
    Bool.not(Bool.and(True{}, U32.is_eq(high, 4294967295))) : Bool
  }:
  match high:
    case U32{+hw}:
      Equal.trans(
        Bool,
        Cmp.is_gt(Revision.order(U32.cmp(0, low), Word.cmp(32n, Word.inc(32n, hw), hw))),
        Cmp.is_gt(Word.cmp(32n, Word.inc(32n, hw), hw)),
        Bool.not(U32.is_eq(U32{hw}, 4294967295)),
        order_carried(U32.cmp(0, low), 31n, hw),
        Equal.trans(
          Bool,
          Cmp.is_gt(Word.cmp(32n, Word.inc(32n, hw), hw)),
          Bool.not(word_full(32n, hw)),
          Bool.not(U32.is_eq(U32{hw}, 4294967295)),
          inc_gt(32n, hw),
          Equal.cong(
            Bool, Bool, v => Bool.not(v), word_full(32n, hw), U32.is_eq(U32{hw}, 4294967295),
            Equal.sym(Bool, U32.is_eq(U32{hw}, 4294967295), word_full(32n, hw), eq_ones(32n, hw))
          )
        )
      )

# Without a carry, the high halves are equal and the low half increments.
def stepped_ahead(+high: U32, +low: U32) ->
  ({U32.is_eq(low, 4294967295) == False{} : Bool} ->
    {
      Revision.ahead(Revision.carry(False{}, high, low), Revision.Revision{high, low}) ==
      True{} : Bool
    }):
  match high low:
    case U32{+hw} U32{+lw}:
      e =>
        %Equal.sym(Cmp, Word.cmp(32n, hw, hw), EQ{}, Words.cmp_refl(32n, hw)) :
          {Cmp.is_gt(Revision.order(Word.cmp(32n, Word.inc(32n, lw), lw), _)) == True{} : Bool}
        Equal.trans(
          Bool,
          Cmp.is_gt(Word.cmp(32n, Word.inc(32n, lw), lw)),
          Bool.not(word_full(32n, lw)),
          True{},
          inc_gt(32n, lw),
          Equal.cong(
            Bool, Bool, v => Bool.not(v), word_full(32n, lw), False{},
            Equal.trans(
              Bool, word_full(32n, lw), U32.is_eq(U32{lw}, 4294967295), False{},
              Equal.sym(Bool, U32.is_eq(U32{lw}, 4294967295), word_full(32n, lw), eq_ones(32n, lw)),
              e
            )
          )
        )

def carried_ahead(wrapped: Bool, +high: U32, +low: U32) ->
  ({U32.is_eq(low, 4294967295) == wrapped : Bool} ->
    {
      Revision.ahead(Revision.carry(wrapped, high, low), Revision.Revision{high, low}) ==
      Bool.not(Bool.and(wrapped, U32.is_eq(high, 4294967295))) : Bool
    }):
  match wrapped:
    case True{}:
      e => wrapped_ahead(high, low)
    case False{}:
      stepped_ahead(high, low)

def Laws.revision_next_is_ahead(r):
  match r:
    case Revision.Revision{+high, +low}:
      carried_ahead(U32.is_eq(low, 4294967295), high, low)({==})

def Laws.status_prompt_marks_drop(tracker, foreground, marks):
  match tracker:
    case Status.Tracker{marked, child}:
      {==}

def Laws.status_foreground_returns_drop(marked, child, foreground, marks):
  match foreground:
    case Boundary.ForegroundUnknown{}:
      {==}
    case Boundary.ForegroundChild{}:
      {==}
    case Boundary.ForegroundOther{}:
      {==}

def Laws.status_foreground_drops_once(tracker, first, marks):
  match tracker:
    case Status.Tracker{marked, child}:
      match marked first:
        case True{} True{}:
          {==}
        case True{} False{}:
          {==}
        case False{} True{}:
          {==}
        case False{} False{}:
          {==}

def Laws.status_unknown_samples_keep_the_last(marked, child, marks):
  {==}

# Once the latch is set, no turn drops at its sample and the latch stays set.
def latched_edges(turns: List<&2, Laws.Turn>, child: Bool) ->
  {Laws.edges(turns, Status.Tracker{True{}, child}) == False{} : Bool}:
  match turns:
    case Nil{}:
      {==}
    case Con{turn, rest}:
      match turn:
        case Laws.Turn{+foreground, +marks}:
          latched_edges(rest, Status.sampled(child, foreground))

def Laws.status_prompt_marks_end_foreground_drops(turns, tracker, foreground):
  match tracker:
    case Status.Tracker{marked, child}:
      match marked:
        case True{}:
          latched_edges(turns, Status.sampled(child, foreground))
        case False{}:
          latched_edges(turns, Status.sampled(child, foreground))

# The lifecycle automaton of ORB-C1, C3 and C12 over the whole step. Once the
# host leaves its fact order the automaton is Outside and stays there.
def outside_kept(lines: List<&2, Boundary.Line>, +state: Session.State) ->
  {Laws.kept(Laws.run(lines, state, Laws.Outside{})) == True{} : Bool}:
  match lines:
    case Nil{}:
      {==}
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          outside_kept(rest, Laws.after(Session.step(fact, flags, state)))

# A phase that is Outside, or that equals the phase of the state step reached,
# keeps the automaton clean as the induction hypothesis does.
def carry(
  lines: List<&2, Boundary.Line>, +state: Session.State, p: Laws.Phase,
  e: {p == Step.choose(p, Laws.phase(state)) : Laws.Phase},
  held: {Laws.kept(Laws.run(lines, state, Laws.phase(state))) == True{} : Bool}
) -> {Laws.kept(Laws.run(lines, state, p)) == True{} : Bool}:
  match p:
    case Laws.Outside{}:
      outside_kept(lines, state)
    case Laws.Live{client, observer}:
      %Equal.sym(Laws.Phase, Laws.Live{client, observer}, Laws.phase(state), e) :
        {Laws.kept(Laws.run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Stopping{}:
      %Equal.sym(Laws.Phase, Laws.Stopping{}, Laws.phase(state), e) :
        {Laws.kept(Laws.run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Stopped{}:
      %Equal.sym(Laws.Phase, Laws.Stopped{}, Laws.phase(state), e) :
        {Laws.kept(Laws.run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Broken{}:
      %Equal.sym(Laws.Phase, Laws.Broken{}, Laws.phase(state), e) :
        {Laws.kept(Laws.run(lines, state, _)) == True{} : Bool}
      held

# Induction over the facts: proof/step.bend shows that each step's decisions
# leave the automaton in the phase of the state step reaches, or Outside.
def Laws.lifecycle_follows_its_contracts(lines, state):
  match lines:
    case Nil{}:
      match state:
        case Session.State{session, shown, status}:
          match session:
            case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
              match life:
                case Orbit.Detached{pid}:
                  {==}
                case Orbit.Attached{pid}:
                  {==}
                case Orbit.Stopping{pid}:
                  {==}
                case Orbit.Stopped{pid}:
                  {==}
    case Con{line, +rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          carry(
            rest, Laws.after(Session.step(fact, flags, state)),
            Laws.check(
              Laws.phase(state), fact, Laws.moves(Laws.decided(Session.step(fact, flags, state)))
            ),
            Step.step_fits(state, flags, fact),
            Laws.lifecycle_follows_its_contracts(rest, Laws.after(Session.step(fact, flags, state)))
          )

# The view automaton of ORB-C8 and C9 over the whole step, by the same
# induction: once the host leaves its fact order the view is Unordered and
# stays there.
def unordered_kept(lines: List<&2, Boundary.Line>, +state: Session.State) ->
  {Laws.view_kept(Laws.view_run(lines, state, Laws.Unordered{})) == True{} : Bool}:
  match lines:
    case Nil{}:
      {==}
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          unordered_kept(rest, Laws.after(Session.step(fact, flags, state)))

# A view that is Unordered, or that equals the view of the state step reached,
# keeps the automaton clean as the induction hypothesis does.
def view_carry(
  lines: List<&2, Boundary.Line>, +state: Session.State, v: Laws.View,
  e: {v == Views.choose(v, Laws.view(state)) : Laws.View},
  held: {Laws.view_kept(Laws.view_run(lines, state, Laws.view(state))) == True{} : Bool}
) -> {Laws.view_kept(Laws.view_run(lines, state, v)) == True{} : Bool}:
  match v:
    case Laws.Unordered{}:
      unordered_kept(lines, state)
    case Laws.Viewing{client, waiting, route, copied, revision, shown}:
      %Equal.sym(Laws.View, Laws.Viewing{client, waiting, route, copied, revision, shown}, Laws.view(state), e) :
        {Laws.view_kept(Laws.view_run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Ending{}:
      %Equal.sym(Laws.View, Laws.Ending{}, Laws.view(state), e) :
        {Laws.view_kept(Laws.view_run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Violated{}:
      %Equal.sym(Laws.View, Laws.Violated{}, Laws.view(state), e) :
        {Laws.view_kept(Laws.view_run(lines, state, _)) == True{} : Bool}
      held

# Induction over the facts: proof/view.bend shows that each step's decisions
# leave the automaton in the view of the state step reaches, or Unordered.
def Laws.scrolling_and_selection_follow_their_contracts(lines, state):
  match lines:
    case Nil{}:
      match state:
        case Session.State{session, shown, status}:
          match session:
            case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
              match life:
                case Orbit.Detached{pid}:
                  match pointer:
                    case Session.Pointer{route, presented, copied}:
                      match shown:
                        case Session.Shown{revision, held}:
                          {==}
                case Orbit.Attached{pid}:
                  match pointer:
                    case Session.Pointer{route, presented, copied}:
                      match shown:
                        case Session.Shown{revision, held}:
                          {==}
                case Orbit.Stopping{pid}:
                  {==}
                case Orbit.Stopped{pid}:
                  {==}
    case Con{line, +rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          view_carry(
            rest, Laws.after(Session.step(fact, flags, state)),
            Laws.view_check(
              Laws.view(state), fact, flags,
              Laws.acts(Laws.decided(Session.step(fact, flags, state))),
              Laws.ahead_of(fact, Laws.view_revision(Laws.view(state)))
            ),
            Views.step_fits(state, flags, fact),
            Laws.scrolling_and_selection_follow_their_contracts(
              rest, Laws.after(Session.step(fact, flags, state))
            )
          )

# The management automaton of ORB-C13 over the whole step, by the same
# induction: once the host leaves its fact order the lease is Disordered and
# stays there.
def disordered_kept(lines: List<&2, Boundary.Line>, +state: Session.State) ->
  {Laws.lease_kept(Laws.lease_run(lines, state, Laws.Disordered{})) == True{} : Bool}:
  match lines:
    case Nil{}:
      {==}
    case Con{line, rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          disordered_kept(rest, Laws.after(Session.step(fact, flags, state)))

# A lease that is Disordered, or that equals the lease of the state step
# reached, keeps the automaton clean as the induction hypothesis does.
def lease_carry(
  lines: List<&2, Boundary.Line>, +state: Session.State, l: Laws.Lease,
  e: {l == Leases.choose(l, Laws.lease(state)) : Laws.Lease},
  held: {Laws.lease_kept(Laws.lease_run(lines, state, Laws.lease(state))) == True{} : Bool}
) -> {Laws.lease_kept(Laws.lease_run(lines, state, l)) == True{} : Bool}:
  match l:
    case Laws.Disordered{}:
      disordered_kept(lines, state)
    case Laws.Leasing{slot}:
      %Equal.sym(Laws.Lease, Laws.Leasing{slot}, Laws.lease(state), e) :
        {Laws.lease_kept(Laws.lease_run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Ceased{}:
      %Equal.sym(Laws.Lease, Laws.Ceased{}, Laws.lease(state), e) :
        {Laws.lease_kept(Laws.lease_run(lines, state, _)) == True{} : Bool}
      held
    case Laws.Breached{}:
      %Equal.sym(Laws.Lease, Laws.Breached{}, Laws.lease(state), e) :
        {Laws.lease_kept(Laws.lease_run(lines, state, _)) == True{} : Bool}
      held

# Induction over the facts: proof/lease.bend shows that each step's decisions
# leave the automaton in the lease of the state step reaches, or Disordered.
def Laws.management_follows_its_contract(lines, state):
  match lines:
    case Nil{}:
      match state:
        case Session.State{session, shown, status}:
          match session:
            case Session.Session{life, written, observer, explicit, waiting, pointer, manage}:
              match life:
                case Orbit.Detached{pid}:
                  {==}
                case Orbit.Attached{pid}:
                  {==}
                case Orbit.Stopping{pid}:
                  {==}
                case Orbit.Stopped{pid}:
                  {==}
    case Con{line, +rest}:
      match line:
        case Boundary.Line{+fact, +flags}:
          lease_carry(
            rest, Laws.after(Session.step(fact, flags, state)),
            Laws.lease_check(
              Laws.lease(state), fact, Laws.replies(Laws.decided(Session.step(fact, flags, state)))
            ),
            Leases.step_fits(state, flags, fact),
            Laws.management_follows_its_contract(rest, Laws.after(Session.step(fact, flags, state)))
          )
