import Base
import ./boundary.bend as Boundary

# The resource plane policy of ORB-C16 (docs/CONTRACTS.md). The host keeps the
# plane's records, its one transfer and its queue, and reports what Bend
# decides from; this decides when a resource starts and when retirements are
# queued.

# Whether the attached client's run is live, whether no transfer is in
# progress, whether 1,024 resources are announced and not retired, and whether
# a resource awaits retirement.
type Plane is Data:
  Plane{live: Bool, idle: Bool, capped: Bool, due: Bool}

# While the run is live, due retirements are queued first, then a resource
# starts when no transfer is in progress and the cap leaves room, so Orbit
# holds at most one copy outside storage. After the stop begins, nothing
# starts or retires. The answer is always one Transfer whose fields carry the
# choice, so the decisions' shape does not depend on the plane.
def answer(plane: Plane) -> List<&2, Boundary.Decision>:
  match plane:
    case Plane{+live, idle, capped, due}:
      [Boundary.Transfer{Bool.and(live, Bool.and(idle, Bool.not(capped))), Bool.and(live, due)}]

# Whether the decisions start a resource: the first Transfer's `start`.
def starts(decisions: List<&2, Boundary.Decision>) -> Bool:
  match decisions:
    case Nil{}:
      False{}
    case Con{decision, rest}:
      match decision:
        case Boundary.Transfer{start, retire}:
          start
        case _:
          starts(rest)
