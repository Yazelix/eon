import Base

# Equality lemmas for Base's words, shared by PROOF.bend and proof/step.bend.

def cmp_refl(n: Nat, w: Word(n)) -> {Word.cmp(n, w, w) == EQ{} : Cmp}:
  match n:
    case 0n:
      {==}
    case 1n+(+p):
      match w:
        case WCon{False{}, +t}:
          %Equal.sym(Cmp, Word.cmp(p, t, t), EQ{}, cmp_refl(p, t)) :
            {Word.cmp.fin(False{}, False{}, _) == EQ{} : Cmp}
          {==}
        case WCon{True{}, +t}:
          %Equal.sym(Cmp, Word.cmp(p, t, t), EQ{}, cmp_refl(p, t)) :
            {Word.cmp.fin(True{}, True{}, _) == EQ{} : Cmp}
          {==}

def is_eq_refl(s: U32) -> {U32.is_eq(s, s) == True{} : Bool}:
  match s:
    case U32{w}:
      %Equal.sym(Cmp, Word.cmp(32n, w, w), EQ{}, cmp_refl(32n, w)) : {Cmp.is_eq(_) == True{} : Bool}
      {==}
