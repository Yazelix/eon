# The presentation revision of ORB-C4, a 64-bit counter. Bend has no U64, so
# it is two U32 halves, and the boundary carries it as two words.
type Revision is Data:
  Revision{high: U32, low: U32}

# A low half at its maximum carries into the high half; the last revision
# wraps to the first, which no session reaches.
def carry(wrapped: Bool, high: U32, low: U32) -> Revision:
  match wrapped:
    case True{}:
      Revision{U32.inc(high), 0}
    case False{}:
      Revision{high, U32.inc(low)}

def next(revision: Revision) -> Revision:
  match revision:
    case Revision{high, +low}:
      carry(U32.is_eq(low, 4294967295), high, low)

# The high halves decide, unless they are equal.
def order(low: Cmp, high: Cmp) -> Cmp:
  match high:
    case LT{}:
      LT{}
    case EQ{}:
      low
    case GT{}:
      GT{}

# Whether `revision` is later than `authority`.
def ahead(revision: Revision, authority: Revision) -> Bool:
  match revision authority:
    case Revision{high, low} Revision{authority_high, authority_low}:
      Cmp.is_gt(order(U32.cmp(low, authority_low), U32.cmp(high, authority_high)))

def is_last(revision: Revision) -> Bool:
  match revision:
    case Revision{high, low}:
      Bool.and(U32.is_eq(low, 4294967295), U32.is_eq(high, 4294967295))
