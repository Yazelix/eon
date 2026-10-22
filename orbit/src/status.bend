import Base
import ./boundary.bend as Boundary

# The program status lifetime policy of ORB-C15 (docs/CONTRACTS.md). The host
# keeps the records; this decides when a PTY turn drops the working and
# blocked ones. A prompt mark always drops them. On a terminal that has never
# sent one, a return of the PTY's foreground group to the child's own group
# from another group drops them too, sampled when the turn begins, before
# its output, so a report in the same turn survives it.

# Whether the terminal has ever sent a prompt mark, and whether the last
# sample found the child's own group in the foreground.
type Tracker is Data:
  Tracker{marked: Bool, child: Bool}

# Before the first turn: no prompt mark, and the child leads the foreground
# group of its new session.
def start() -> Tracker:
  Tracker{False{}, True{}}

def is_child(foreground: Boundary.Foreground) -> Bool:
  match foreground:
    case Boundary.ForegroundChild{}:
      True{}
    case _:
      False{}

# A sample that finds no group keeps the last one.
def sampled(child: Bool, foreground: Boundary.Foreground) -> Bool:
  match foreground:
    case Boundary.ForegroundUnknown{}:
      child
    case Boundary.ForegroundChild{}:
      True{}
    case Boundary.ForegroundOther{}:
      False{}

# Whether the turn's foreground sample drops: never after a prompt mark, and
# otherwise when it finds the child's group after a sample that found another.
def edge(tracker: Tracker, foreground: Boundary.Foreground) -> Bool:
  match tracker:
    case Tracker{marked, child}:
      Bool.and(Bool.not(marked), Bool.and(Bool.not(child), is_child(foreground)))

# The turn's answer: whether its foreground sample drops, and whether its
# prompt marks do. It is a constructor before the tracker is known, so a proof
# over step's decisions need not split the tracker.
def lifetime(tracker: Tracker, foreground: Boundary.Foreground, marks: Bool) -> Boundary.Decision:
  Boundary.StatusLifetime{edge(tracker, foreground), marks}

# The tracker after the turn. A prompt mark sets the latch for the rest of the
# run.
def after(tracker: Tracker, foreground: Boundary.Foreground, marks: Bool) -> Tracker:
  match tracker:
    case Tracker{marked, child}:
      Tracker{Bool.or(marked, marks), sampled(child, foreground)}
