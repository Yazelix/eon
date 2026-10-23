import Base
import ./lifecycle.bend as Life
import ./selection.bend as Selection
import ./management.bend as Management
import ./boundary.bend as Boundary
import ./session.bend as Session
import ./status.bend as Status

# The Bend worker: serve reads the host's facts and writes step's decisions
# (src/session.bend) through Base File effects on the two pipes of docs/HOST.md.

# The line naming the worker's boundary schema, then Bend's bounds, sent once
# at start: the finish bound, the pending-peer bound and the
# synchronized-output bound in milliseconds, and the interactive client's
# output bound in bytes (one maximum frame, one maximum payload, two headers
# and 4 KiB).
def bounds() -> String:
  Boundary.schema() ++ Boundary.encode([Boundary.Bounds{1000, 1000, 1000, 12587045}])

@unsafe
def wrote_answer(
  pair: File & Result<&1, &1, U32 & String, Unit>, input: File, state: Session.State
) -> IO(Unit):
  (output, result) = pair
  do IO<Unit>:
    Unit <- IO.pass(Unit, result)
    serve(input, output, state)

def answer(pair: Session.Stepped(), input: File, output: File) -> IO(Unit):
  (state, decisions) = pair
  do IO<Unit>:
    written : File & Result<&1, &1, U32 & String, Unit> <-
      File.write(output, Boundary.encode(decisions))
    wrote_answer(written, input, state)

def finished(pair: File & Result<&1, &1, U32 & String, Unit>, input: File) -> IO(Unit):
  (output, result) = pair
  do IO<Unit>:
    Unit <- IO.pass(Unit, result)
    Unit <- File.close(input)
    File.close(output)

# The stop is reap-gated: the host reports finished only after Bend's exit
# decision, which follows the reaped fact, and exits with the child's code.
def finish(input: File, output: File) -> IO(Unit):
  do IO<Unit>:
    ended : File & Result<&1, &1, U32 & String, Unit> <-
      File.write(output, Boundary.encode(Nil{}))
    finished(ended, input)

def heard(
  +fact: Boundary.Fact, flags: Boundary.Flags, input: File, output: File, state: Session.State
) -> IO(Unit):
  match fact:
    # The containment fixture: Bend fails at once, holding nothing the host
    # must clean up for it.
    case Boundary.Fault{pid}:
      do IO<Unit>:
        Unit <- IO.print("FAULT pid=" ++ U32.show(pid))
        IO.die(Unit, 1, "guardian fault fixture")
    case Boundary.Finished{}:
      finish(input, output)
    case _:
      answer(Session.step(fact, flags, state), input, output)

def line(
  decoded: Maybe<&2, Boundary.Line>, input: File, output: File, state: Session.State
) -> IO(Unit):
  match decoded:
    case None{}:
      IO.die(Unit, 1, "the host sent a malformed fact")
    case Some{Boundary.Line{fact, flags}}:
      heard(fact, flags, input, output, state)

def got_text(
  empty: Bool, text: String, input: File, output: File, state: Session.State
) -> IO(Unit):
  match empty:
    case True{}:
      IO.die(Unit, 1, "the host closed the fact pipe")
    case False{}:
      line(Boundary.decode(text), input, output, state)

def got(
  pair: File & Result<&1, &1, U32 & String, String>, output: File, state: Session.State
) -> IO(Unit):
  (input, result) = pair
  do IO<Unit>:
    text : String <- IO.pass(String, result)
    +shared : String = text
    got_text(String.is_empty(shared), shared, input, output, state)

@unsafe
def serve(input: File, output: File, state: Session.State) -> IO(Unit):
  do IO<Unit>:
    pair : File & Result<&1, &1, U32 & String, String> <- File.read(input, 4096)
    got(pair, output, state)

def started_serving(pair: File & Result<&1, &1, U32 & String, Unit>, input: File) ->
  IO(Unit):
  (output, result) = pair
  do IO<Unit>:
    Unit <- IO.pass(Unit, result)
    serve(
      input, output,
      Session.State{
        Session.Session{
          Life.Detached{0}, False{}, False{}, False{}, Session.Clear{},
          Session.Pointer{Selection.Idle{}, False{}, False{}}, Management.Free{}
        },
        Session.Shown{Session.unstamped(), False{}}, Status.start()
      }
    )

def main() -> IO(Unit):
  do IO<Unit>:
    input : File <- IO.try(File, File.open("/dev/fd/3", "r"))
    output : File <- IO.try(File, File.open("/dev/fd/4", "a"))
    first : File & Result<&1, &1, U32 & String, Unit> <- File.write(output, bounds())
    started_serving(first, input)
