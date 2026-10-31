//! Generates the per-fact lemmas behind the laws over the whole step in
//! LAWS.bend: proof/step.bend for lifecycle_follows_its_contracts,
//! proof/view.bend for scrolling_and_selection_follow_their_contracts, and
//! proof/lease.bend for management_follows_its_contract.
//!
//! Each lemma states, for one fact, that the automaton's next state after
//! step's decisions is the one it reads from step's next state, or its
//! unordered state (Outside, Unordered). Its proof splits cases until every
//! leaf holds by computation, since Bend reduces a call only once the values
//! it matches on are constructors. The tool starts from the splits already in
//! the law's file, or in proof/step.bend for a new law or a fact new to the
//! law's file, checks each lemma with Bend, and where a leaf fails, splits the
//! variable that the leaf's stuck call matches on. Rerun it for each law after
//! changing step (src/session.bend), and for a law after changing its
//! automaton.
//!
//! Usage, from the orbit directory:
//!   mkdir -p target
//!   rustc --edition 2024 -O tools/step-proof.rs -o target/step-proof
//!   target/step-proof LAW BEND2_MAIN_TS [FACT,...]
//! LAW is lifecycle, view or management. BEND2_MAIN_TS is bend2/main.ts of the
//! Bend that flake.nix pins. JOBS (default 8) checks that many facts at once.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};

const USAGE: &str = "usage: step-proof LAW BEND2_MAIN_TS [FACT,...]
LAW is lifecycle, view or management. BEND2_MAIN_TS is bend2/main.ts of the
Bend that flake.nix pins. JOBS (default 8) checks that many facts at once.";

const MODULES: [(&str, &str); 10] = [
    ("session", "Session"),
    ("status", "Status"),
    ("resource", "Resource"),
    ("boundary", "Boundary"),
    ("lifecycle", "Orbit"),
    ("admission", "Admission"),
    ("vertical", "Vertical"),
    ("selection", "Selection"),
    ("management", "Management"),
    ("revision", "Revision"),
];

// The facts of src/boundary.schema, with names for their fields.
const FACTS: [(&str, &[(&str, &str)]); 29] = [
    ("Started", &[("pid", "U32")]),
    (
        "Output",
        &[("foreground", "Boundary.Foreground"), ("marks", "Bool")],
    ),
    ("PtyClosed", &[]),
    ("Peer", &[("message", "Boundary.FirstMessage")]),
    ("Applied", &[("effect", "Boundary.Effect")]),
    ("Closed", &[("full", "Bool")]),
    ("Pressured", &[]),
    ("Reaped", &[("status", "U32")]),
    ("NotReaped", &[]),
    ("Finished", &[]),
    ("RoleRepeated", &[]),
    ("Refused", &[("version", "Bool")]),
    ("ObserverGone", &[("loss", "Boundary.ObserverLoss")]),
    ("Expired", &[]),
    (
        "PreviewVertical",
        &[
            ("frev", "Revision.Revision"),
            ("fdirection", "Boundary.Direction"),
        ],
    ),
    ("Fault", &[("pid", "U32")]),
    ("Wheel", &[("input", "Boundary.WheelInput")]),
    (
        "Selection",
        &[
            ("kind", "Boundary.SelectionKind"),
            ("frev", "Revision.Revision"),
        ],
    ),
    (
        "Selected",
        &[("outcome", "Boundary.Selected"), ("presented", "Bool")],
    ),
    (
        "Tick",
        &[
            ("tstate", "Boundary.TickState"),
            ("fx", "U32"),
            ("fy", "U32"),
        ],
    ),
    ("Management", &[("event", "Boundary.ManagementEvent")]),
    ("ChildExited", &[("drained", "Bool")]),
    ("Crowded", &[]),
    ("ObserverMessage", &[]),
    (
        "ScrollVertical",
        &[("frev", "Revision.Revision"), ("frows", "U32")],
    ),
    ("ReturnToLive", &[]),
    ("ExitedBeforeStop", &[("status", "U32")]),
    ("ProgramStatus", &[("foreground", "Boundary.Foreground")]),
    (
        "Resources",
        &[
            ("delivered", "Bool"),
            ("idle", "Bool"),
            ("capped", "Bool"),
            ("due", "Bool"),
        ],
    ),
];
const SESSION: [&str; 7] = [
    "life", "written", "observer", "explicit", "waiting", "pointer", "manage",
];
const STATE: [(&str, &str); 11] = [
    ("life", "Orbit.Lifecycle"),
    ("observer", "Bool"),
    ("waiting", "Session.Waiting"),
    ("pointer", "Session.Pointer"),
    ("written", "Bool"),
    ("explicit", "Bool"),
    ("manage", "Management.Slot"),
    ("tracker", "Status.Tracker"),
    ("revision", "Revision.Revision"),
    ("was", "Bool"),
    ("future", "Bool"),
];
// The flags in their word's order, and in the order the lemmas split them.
const FLAGS: [&str; 18] = [
    "held",
    "routed",
    "room",
    "alternate",
    "frame_room",
    "above_live",
    "clipboard",
    "clipboard_overflow",
    "tracking",
    "shift",
    "unanchored",
    "result_frame_room",
    "outside",
    "host_room",
    "terminal_room",
    "pty_full",
    "metadata_changed",
    "metadata_fresh",
];
const FLAG_ORDER: [&str; 18] = [
    "held",
    "unanchored",
    "clipboard_overflow",
    "clipboard",
    "routed",
    "room",
    "frame_room",
    "alternate",
    "above_live",
    "tracking",
    "shift",
    "result_frame_room",
    "outside",
    "host_room",
    "terminal_room",
    "pty_full",
    "metadata_changed",
    "metadata_fresh",
];

// A finish compares the reaped status with itself, which needs a lemma.
const STATUS_LEAF: &str =
    "%Equal.sym(Bool, U32.is_eq(status, status), True{}, Words.is_eq_refl(status)) :
  {Laws.same_status(_, Nil{}) == choose(Laws.same_status(_, Nil{}), Laws.Stopped{}) : Laws.Phase}
{==}";

/// One constructor pattern, with the splittable variables it binds.
type Case = (String, Vec<(&'static str, &'static str)>);

/// Splittable types, by name.
fn enums() -> HashMap<String, Vec<Case>> {
    let plain = |cs: &[&str], f: &dyn Fn(&str) -> String| -> Vec<Case> {
        cs.iter().map(|c| (f(c), vec![])).collect()
    };
    let mut enums: HashMap<String, Vec<Case>> = HashMap::new();
    enums.insert("Bool".into(), plain(&["True{}", "False{}"], &|c| c.into()));
    enums.insert(
        "Orbit.Lifecycle".into(),
        plain(&["Detached", "Attached", "Stopping", "Stopped"], &|c| {
            format!("Orbit.{c}{{pid}}")
        }),
    );
    enums.insert(
        "Session.Waiting".into(),
        vec![
            ("Session.Clear{}".into(), vec![]),
            (
                "Session.Waiting{request}".into(),
                vec![("request", "Session.Request")],
            ),
        ],
    );
    enums.insert(
        "Session.Request".into(),
        plain(
            &[
                "Session.PreviewRequest{direction}",
                "Session.ScrollRequest{rows}",
                "Session.LiveRequest{}",
                "Session.TickRequest{tx, ty}",
            ],
            &|c| c.into(),
        ),
    );
    enums.insert(
        "Session.Pointer".into(),
        vec![(
            "Session.Pointer{route, lit, copied}".into(),
            vec![
                ("route", "Selection.Route"),
                ("lit", "Bool"),
                ("copied", "Bool"),
            ],
        )],
    );
    enums.insert(
        "Selection.Route".into(),
        plain(&["Idle", "Host", "Terminal"], &|c| {
            format!("Selection.{c}{{}}")
        }),
    );
    enums.insert(
        "Management.Slot".into(),
        plain(&["Free", "Negotiating", "Leased", "Closing"], &|c| {
            format!("Management.{c}{{}}")
        }),
    );
    let boundary: [(&str, &[&str]); 9] = [
        (
            "FirstMessage",
            &[
                "Hello",
                "ObserveMetadata",
                "Refused",
                "Unsupported",
                "Stranger",
            ],
        ),
        (
            "Effect",
            &["Quiet", "Write", "Resize", "Full", "Key", "Closed"],
        ),
        ("ObserverLoss", &["Closed", "Pressured", "Failed"]),
        ("Direction", &["Up", "Down"]),
        ("WheelInput", &["Fits", "Empty", "NoOutcomeRoom", "PtyFull"]),
        (
            "SelectionKind",
            &["Begin", "Update", "Finish", "Cancel", "Copy"],
        ),
        (
            "Selected",
            &["Applied", "Finished", "Failed", "Began", "Ticked"],
        ),
        ("TickState", &["Fits", "NotTopRow", "AnchorLost"]),
        (
            "ManagementEvent",
            &[
                "Connect",
                "AcquireValid",
                "AcquireInvalid",
                "Status",
                "Stop",
                "Bad",
                "Gone",
                "Read",
                "Input",
                "Allow",
                "Invalid",
            ],
        ),
    ];
    for (ty, cs) in boundary {
        enums.insert(
            format!("Boundary.{ty}"),
            plain(cs, &|c| format!("Boundary.{ty}{c}{{}}")),
        );
    }
    enums
}

/// The state type, unordered state, check and splits of one law.
struct Law {
    file: &'static str,
    ty: &'static str,
    outside: &'static str,
    start: &'static str,
    noun: &'static str,
    check: fn(&str, &str, &str, &str, &str) -> String,
    future: &'static str,
    splits: Vec<(&'static str, Vec<String>)>,
}

fn law(name: &str) -> Option<Law> {
    match name {
        "lifecycle" => Some(Law {
            file: "step.bend",
            ty: "Laws.Phase",
            outside: "Laws.Outside",
            start: "Laws.phase",
            noun: "phase",
            check: |state, fact, _flags, step, _future| {
                format!("Laws.check(Laws.phase({state}), {fact}, Laws.moves(Laws.decided({step})))")
            },
            future: "False{}",
            splits: vec![],
        }),
        "view" => Some(Law {
            file: "view.bend",
            ty: "Laws.View",
            outside: "Laws.Unordered",
            start: "Laws.view",
            noun: "view",
            check: |state, fact, flags, step, future| {
                format!(
                    "Laws.view_check(Laws.view({state}), {fact}, {flags}, \
                     Laws.acts(Laws.decided({step})), {future})"
                )
            },
            // The automaton reads a request's future from its own revision,
            // which computes once the dispatcher splits what Laws.view matches
            // on.
            future: "Laws.ahead_of(fact, Laws.view_revision(Laws.view(state)))",
            splits: vec![
                (
                    "life",
                    ["Detached", "Attached", "Stopping", "Stopped"]
                        .iter()
                        .map(|c| format!("Orbit.{c}{{lpid}}"))
                        .collect(),
                ),
                (
                    "pointer",
                    vec!["Session.Pointer{route, lit, copied}".into()],
                ),
            ],
        }),
        "management" => Some(Law {
            file: "lease.bend",
            ty: "Laws.Lease",
            outside: "Laws.Disordered",
            start: "Laws.lease",
            noun: "lease",
            check: |state, fact, flags, step, _future| {
                format!(
                    "Laws.lease_check(Laws.lease({state}), {fact}, {flags}, \
                     Laws.replies(Laws.decided({step})))"
                )
            },
            future: "False{}",
            splits: vec![],
        }),
        _ => None,
    }
}

struct Ctx {
    law: Law,
    root: PathBuf,
    target: PathBuf,
    enums: HashMap<String, Vec<Case>>,
    patterns: HashMap<String, Vec<(&'static str, &'static str)>>,
}

impl Ctx {
    fn choose_def(&self) -> String {
        let l = &self.law;
        format!(
            "# The automaton's {}, unless it is {}.\n\
             def choose(p: {ty}, q: {ty}) -> {ty}:\n  match p:\n    case {out}{{}}:\n      {out}{{}}\n\
             \x20   case _:\n      q\n",
            l.noun,
            &l.outside[5..],
            ty = l.ty,
            out = l.outside,
        )
    }

    fn statement(
        &self,
        fact_text: &str,
        state: &str,
        step: &str,
        flags: &str,
        future: &str,
    ) -> String {
        let check = (self.law.check)(state, fact_text, flags, step, future);
        let after = format!("{}(Laws.after({step}))", self.law.start);
        format!("{{{check} == choose({check}, {after}) : {}}}", self.law.ty)
    }

    fn goal(&self, fact: &str) -> String {
        let fact_text = fact_term(fact);
        let flags = flags_term();
        let session = session_term();
        let state =
            format!("Session.State{{{session}, Session.Shown{{revision, was}}, tracker}}");
        // A PTY turn's program status lifetime leads its decisions and moves
        // the tracker, and so does a ProgramStatus request's foreground
        // sample, without prompt marks; every other fact keeps it.
        let (lifetime, tracked) = match fact {
            "Output" => (
                "[Status.lifetime(tracker, foreground, marks)]",
                "Status.after(tracker, foreground, marks)",
            ),
            "ProgramStatus" => (
                "[Status.lifetime(tracker, foreground, False{})]",
                "Status.after(tracker, foreground, False{})",
            ),
            _ => ("Nil{}", "tracker"),
        };
        let step = format!(
            "Session.begun(Session.fact_advanced({fact_text}, was, revision), held, {lifetime}, \
             {tracked}, Session.anchored(unanchored, {fact_text}, {flags}, future, {session}))"
        );
        self.statement(&fact_text, &state, &step, &flags, "future")
    }

    fn split(&self, var: &str, fact: &str, child: &Node) -> Node {
        let ty = types(fact)[var];
        Node::Split {
            var: var.into(),
            cases: self.enums[ty]
                .iter()
                .map(|(pat, _)| (pat.clone(), child.clone()))
                .collect(),
        }
    }
}

fn fields(fact: &str) -> &'static [(&'static str, &'static str)] {
    FACTS
        .iter()
        .find(|(f, _)| *f == fact)
        .expect("known fact")
        .1
}

fn params(fact: &str) -> Vec<(&'static str, &'static str)> {
    let mut out = fields(fact).to_vec();
    out.extend(STATE);
    out.extend(FLAG_ORDER.iter().map(|f| (*f, "Bool")));
    out
}

fn types(fact: &str) -> HashMap<&'static str, &'static str> {
    let mut ty: HashMap<_, _> = params(fact).into_iter().collect();
    ty.extend([
        ("request", "Session.Request"),
        ("route", "Selection.Route"),
        ("lit", "Bool"),
        ("copied", "Bool"),
    ]);
    ty
}

fn priority(fact: &str) -> HashMap<&'static str, usize> {
    let mut order = vec![];
    for (name, _) in params(fact) {
        order.push(name);
        match name {
            "waiting" => order.push("request"),
            "pointer" => order.extend(["route", "lit", "copied"]),
            _ => {}
        }
    }
    order.into_iter().enumerate().map(|(k, n)| (n, k)).collect()
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn fact_term(fact: &str) -> String {
    let names: Vec<_> = fields(fact).iter().map(|(n, _)| *n).collect();
    format!("Boundary.{fact}{{{}}}", names.join(", "))
}

fn flags_term() -> String {
    format!("Boundary.Flags{{{}}}", FLAGS.join(", "))
}

fn session_term() -> String {
    format!("Session.Session{{{}}}", SESSION.join(", "))
}

fn header(prefix: &str) -> String {
    let imports: String = MODULES
        .iter()
        .map(|(m, a)| format!("import {prefix}src/{m}.bend as {a}\n"))
        .collect();
    format!(
        "import Base\nimport {prefix}LAWS.bend as Laws\n{imports}\
         import {prefix}proof/word.bend as Words\n"
    )
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

// ---------------------------------------------------------------- case trees

#[derive(Clone)]
enum Node {
    Leaf(String),
    Split {
        var: String,
        cases: Vec<(String, Node)>,
    },
}

fn leaf() -> Node {
    Node::Leaf("{==}".into())
}

/// The lines of a tree at an indent, and the path to the leaf at each leaf's
/// first line.
fn emit(tree: &Node, indent: usize) -> (Vec<String>, HashMap<usize, Vec<usize>>) {
    fn go(
        node: &Node,
        ind: usize,
        path: &mut Vec<usize>,
        lines: &mut Vec<String>,
        leaves: &mut HashMap<usize, Vec<usize>>,
    ) {
        match node {
            Node::Leaf(text) => {
                leaves.insert(lines.len(), path.clone());
                lines.extend(text.split('\n').map(|t| format!("{}{t}", " ".repeat(ind))));
            }
            Node::Split { var, cases } => {
                lines.push(format!("{}match {var}:", " ".repeat(ind)));
                for (k, (pat, child)) in cases.iter().enumerate() {
                    lines.push(format!("{}case {pat}:", " ".repeat(ind + 2)));
                    path.push(k);
                    go(child, ind + 4, path, lines, leaves);
                    path.pop();
                }
            }
        }
    }
    let (mut lines, mut leaves) = (vec![], HashMap::new());
    go(tree, indent, &mut vec![], &mut lines, &mut leaves);
    (lines, leaves)
}

/// The variable a `match VAR:` line splits.
fn match_var(text: &str) -> Option<&str> {
    let var = text.strip_prefix("match ")?.strip_suffix(':')?;
    (!var.is_empty() && var.chars().all(is_word)).then_some(var)
}

/// Reads back a body that emit wrote.
fn parse_tree(lines: &[&str]) -> Node {
    fn go(lines: &[&str], mut k: usize, ind: usize) -> (Node, usize) {
        let Some(var) = match_var(lines[k].trim()) else {
            let mut end = k;
            while end < lines.len()
                && indent_of(lines[end]) >= ind
                && !lines[end].trim_start().starts_with("case ")
            {
                end += 1;
            }
            let text: Vec<_> = lines[k..end]
                .iter()
                .map(|l| l.get(ind..).unwrap_or(""))
                .collect();
            return (Node::Leaf(text.join("\n")), end);
        };
        let mut cases = vec![];
        k += 1;
        let case = format!("{}case ", " ".repeat(ind + 2));
        while k < lines.len() && lines[k].starts_with(&case) {
            let text = lines[k].trim();
            let pat = text[5..text.len() - 1].to_string();
            let (child, next) = go(lines, k + 1, ind + 4);
            cases.push((pat, child));
            k = next;
        }
        (
            Node::Split {
                var: var.into(),
                cases,
            },
            k,
        )
    }
    if lines.is_empty() {
        return leaf();
    }
    go(lines, 0, indent_of(lines[0])).0
}

fn existing_trees(ctx: &Ctx) -> HashMap<&'static str, Node> {
    // A new law, or a fact new to the law's file, starts from the lifecycle
    // law's splits, which step needs anyway.
    let step = ctx.root.join("proof").join("step.bend");
    let own = fs::read_to_string(&ctx.target).unwrap_or_default();
    let seed = fs::read_to_string(&step).unwrap_or_default();
    let own: Vec<&str> = own.split('\n').collect();
    let seed: Vec<&str> = seed.split('\n').collect();
    let mut trees = HashMap::new();
    for (fact, _) in FACTS {
        let start = format!("def fits_{}(", snake(fact));
        let found = |text: &Vec<&str>| text.iter().position(|l| l.starts_with(&start));
        let (text, seeded, mut k) = match (found(&own), found(&seed)) {
            (Some(k), _) => (&own, false, k),
            (None, Some(k)) => (&seed, true, k),
            (None, None) => continue,
        };
        while !text[k].ends_with(':') || text[k].starts_with("def ") {
            k += 1;
        }
        let mut end = k + 1;
        while end < text.len() && (text[end].starts_with(' ') || text[end].is_empty()) {
            end += 1;
        }
        let body: Vec<&str> = text[k + 1..end]
            .iter()
            .copied()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let mut tree = complete(ctx, fact, parse_tree(&body));
        if seeded {
            tree = plain_leaves(&tree);
        }
        trees.insert(fact, tree);
    }
    trees
}

/// Adds a leaf for each constructor a split lacks, as after a variant joins
/// an enum the split matches on.
fn complete(ctx: &Ctx, fact: &str, tree: Node) -> Node {
    match tree {
        Node::Leaf(_) => tree,
        Node::Split { var, cases } => {
            let mut cases: Vec<(String, Node)> = cases
                .into_iter()
                .map(|(p, c)| (p, complete(ctx, fact, c)))
                .collect();
            if let Some(all) = types(fact).get(var.as_str()).and_then(|t| ctx.enums.get(*t)) {
                for (pat, _) in all {
                    if !cases.iter().any(|(p, _)| p == pat) {
                        cases.push((pat.clone(), leaf()));
                    }
                }
            }
            Node::Split { var, cases }
        }
    }
}

fn plain_leaves(tree: &Node) -> Node {
    match tree {
        Node::Leaf(_) => leaf(),
        Node::Split { var, cases } => Node::Split {
            var: var.clone(),
            cases: cases
                .iter()
                .map(|(p, c)| (p.clone(), plain_leaves(c)))
                .collect(),
        },
    }
}

/// The splittable variables in scope at the leaf at path, not yet split.
fn unsplit(ctx: &Ctx, fact: &str, tree: &Node, path: &[usize]) -> HashSet<String> {
    let mut avail: HashSet<String> = params(fact)
        .into_iter()
        .filter(|(_, t)| ctx.enums.contains_key(*t))
        .map(|(n, _)| n.to_string())
        .collect();
    let mut node = tree;
    for &k in path {
        let Node::Split { var, cases } = node else {
            break;
        };
        avail.remove(var);
        let (pat, child) = &cases[k];
        avail.extend(ctx.patterns[pat].iter().map(|(b, _)| b.to_string()));
        node = child;
    }
    avail
}

/// Splits var above the first node on path that splits a later variable, so
/// every path matches variables in parameter order, as Bend requires.
fn insert(ctx: &Ctx, fact: &str, tree: Node, path: &[usize], var: &str) -> Node {
    let order = priority(fact);
    fn go(
        ctx: &Ctx,
        fact: &str,
        order: &HashMap<&str, usize>,
        node: Node,
        path: &[usize],
        var: &str,
    ) -> Node {
        match (node, path.split_first()) {
            (
                Node::Split {
                    var: here,
                    mut cases,
                },
                Some((&k, rest)),
            ) if order[here.as_str()] <= order[var] => {
                let child = std::mem::replace(&mut cases[k].1, leaf());
                cases[k].1 = go(ctx, fact, order, child, rest, var);
                Node::Split { var: here, cases }
            }
            (node, _) => ctx.split(var, fact, &node),
        }
    }
    go(ctx, fact, &order, tree, path, var)
}

fn set_leaf(tree: &mut Node, path: &[usize], text: &str) {
    match path.split_first() {
        None => *tree = Node::Leaf(text.into()),
        Some((&k, rest)) => {
            if let Node::Split { cases, .. } = tree {
                set_leaf(&mut cases[k].1, rest, text);
            }
        }
    }
}

// ---------------------------------------------------------------- stuck calls

fn split_top(text: &str, sep: char) -> Vec<String> {
    let (mut out, mut depth, mut cur) = (vec![], 0i32, String::new());
    for ch in text.chars() {
        depth += i32::from("([{<".contains(ch)) - i32::from(")]}>".contains(ch));
        if ch == sep && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// A parameter and the field path within it that a body matches on.
type Matched = (usize, Vec<usize>);

/// Whether a joined def head reaches its `) -> TYPE:`.
fn head_complete(head: &str) -> bool {
    let trimmed = head.trim_end();
    trimmed.ends_with(':')
        && head.match_indices(')').any(|(i, _)| {
            let rest = head[i + 1..].trim_start();
            rest.starts_with("->") && head.len() - rest.len() + 2 < trimmed.len()
        })
}

/// The name and parameters of `def NAME(PARAMS) ->`, with PARAMS ending at the
/// first `)` that `->` follows.
fn def_head(head: &str) -> Option<(&str, &str)> {
    let rest = head.strip_prefix("def ")?;
    let name_end = rest
        .find(|c: char| !(is_word(c) || c == '.'))
        .unwrap_or(rest.len());
    let (name, rest) = rest.split_at(name_end);
    let rest = rest.strip_prefix('(')?;
    if name.is_empty() {
        return None;
    }
    rest.match_indices(')')
        .find(|(i, _)| rest[i + 1..].trim_start().starts_with("->"))
        .map(|(i, _)| (name, &rest[..i]))
}

/// The bindings of a `(a, b) = v` line.
fn tuple_let(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('(')?;
    let close = rest.find(") = ")?;
    let (names, value) = (&rest[..close], &rest[close + 4..]);
    let names_ok = !names.is_empty() && names.chars().all(|c| is_word(c) || "+, ".contains(c));
    let value_ok = !value.is_empty() && value.chars().all(is_word);
    (names_ok && value_ok).then_some((names, value))
}

/// The fields of a `Name{...}` pattern.
fn ctor_fields(pat: &str) -> Option<&str> {
    let open = pat.find('{')?;
    let name = &pat[..open];
    let ok = !name.is_empty() && name.chars().all(|c| is_word(c) || c == '.') && pat.ends_with('}');
    ok.then(|| &pat[open + 1..pat.len() - 1])
}

/// For each def, the (parameter, field path) pairs its body matches on.
fn matched_paths(path: &Path, prefix: Option<&str>) -> HashMap<String, HashSet<Matched>> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let lines: Vec<&str> = text.split('\n').collect();
    let mut table = HashMap::new();
    let mut k = 0;
    while k < lines.len() {
        if !lines[k].starts_with("def ") {
            k += 1;
            continue;
        }
        let mut head = lines[k].to_string();
        while !head_complete(&head) && k + 1 < lines.len() {
            k += 1;
            head.push(' ');
            head.push_str(lines[k].trim());
        }
        k += 1;
        let mut body = vec![];
        while k < lines.len() && (lines[k].starts_with(' ') || lines[k].is_empty()) {
            body.push(lines[k]);
            k += 1;
        }
        let Some((name, params)) = def_head(&head) else {
            continue;
        };
        let mut origin: HashMap<String, Matched> = split_top(params, ',')
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let p = p.trim().trim_start_matches(['+', '-']);
                (
                    p.split(':').next().unwrap_or("").trim().to_string(),
                    (i, vec![]),
                )
            })
            .collect();
        let mut matched = HashSet::new();
        let mut stack: Vec<(usize, Vec<String>)> = vec![];
        for line in body {
            let text = line.trim();
            let ind = indent_of(line);
            while stack.last().is_some_and(|(i, _)| *i >= ind) {
                stack.pop();
            }
            if let Some(scrutinees) = text
                .strip_prefix("match ")
                .and_then(|t| t.strip_suffix(':'))
            {
                let scrutinees: Vec<String> =
                    scrutinees.split_whitespace().map(String::from).collect();
                matched.extend(scrutinees.iter().filter_map(|v| origin.get(v).cloned()));
                stack.push((ind, scrutinees));
                continue;
            }
            if let Some(pats) = text.strip_prefix("case ").and_then(|t| t.strip_suffix(':')) {
                let Some((_, scrutinees)) = stack.last() else {
                    continue;
                };
                let pats = split_top(&pats.replace("} ", "}\0").replace("_ ", "_\0"), '\0');
                for (v, pat) in scrutinees.iter().zip(&pats) {
                    let (Some(inner), Some(from)) =
                        (ctor_fields(pat.trim()), origin.get(v).cloned())
                    else {
                        continue;
                    };
                    for (i, b) in split_top(inner, ',').iter().enumerate() {
                        let b = b.trim().trim_start_matches('+');
                        if !b.is_empty() && b.chars().all(is_word) {
                            let mut field = from.1.clone();
                            field.push(i);
                            origin.insert(b.to_string(), (from.0, field));
                        }
                    }
                }
                continue;
            }
            if let Some((names, v)) = tuple_let(text)
                && let Some(from) = origin.get(v).cloned()
            {
                matched.insert(from.clone());
                for (i, b) in names.split(',').enumerate() {
                    let mut field = from.1.clone();
                    field.push(i);
                    origin.insert(
                        b.trim().trim_start_matches('+').to_string(),
                        (from.0, field),
                    );
                }
            }
        }
        let key = match prefix {
            None => name.to_string(),
            Some(p) => format!("{p}.{name}"),
        };
        table.insert(key, matched);
    }
    table
}

/// The tokens of a term, skipping what no token matches.
fn tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let token = |c: char| is_word(c) || "./+'".contains(c);
    let (mut out, mut k) = (vec![], 0);
    while k < chars.len() {
        let c = chars[k];
        let pair: String = chars[k..chars.len().min(k + 2)].iter().collect();
        if pair == "<>" || pair == "==" {
            out.push(pair);
            k += 2;
        } else if token(c) {
            let start = k;
            while k < chars.len() && token(chars[k]) {
                k += 1;
            }
            out.push(chars[start..k].iter().collect());
        } else {
            if "(){}[],:".contains(c) {
                out.push(c.to_string());
            }
            k += 1;
        }
    }
    out
}

#[derive(PartialEq)]
enum Kind {
    Var,
    Call,
    Ctor,
}

struct Term {
    kind: Kind,
    name: String,
    args: Vec<Term>,
}

/// A term, with `head <> tail` read as a node whose parts waits searches.
fn parse_term(tokens: &[String], k: usize) -> (Term, usize) {
    let (mut term, mut k) = parse_atom(tokens, k);
    while k < tokens.len() && tokens[k] == "<>" {
        let (tail, next) = parse_atom(tokens, k + 1);
        term = Term {
            kind: Kind::Ctor,
            name: "<>".into(),
            args: vec![term, tail],
        };
        k = next;
    }
    (term, k)
}

fn parse_atom(tokens: &[String], k: usize) -> (Term, usize) {
    let Some(t) = tokens.get(k) else {
        return (
            Term {
                kind: Kind::Var,
                name: String::new(),
                args: vec![],
            },
            k + 1,
        );
    };
    if t == "[" || t == "(" {
        let (args, k) = parse_args(tokens, k + 1, if t == "[" { "]" } else { ")" });
        return (
            Term {
                kind: Kind::Ctor,
                name: String::new(),
                args,
            },
            k,
        );
    }
    match tokens.get(k + 1).map(String::as_str) {
        Some(open @ ("(" | "{")) => {
            let (kind, close) = if open == "(" {
                (Kind::Call, ")")
            } else {
                (Kind::Ctor, "}")
            };
            let (args, k) = parse_args(tokens, k + 2, close);
            (
                Term {
                    kind,
                    name: t.clone(),
                    args,
                },
                k,
            )
        }
        _ => (
            Term {
                kind: Kind::Var,
                name: t.clone(),
                args: vec![],
            },
            k + 1,
        ),
    }
}

fn parse_args(tokens: &[String], mut k: usize, close: &str) -> (Vec<Term>, usize) {
    let mut args = vec![];
    while k < tokens.len() && tokens[k] != close {
        if tokens[k] == "," {
            k += 1;
            continue;
        }
        let (term, next) = parse_term(tokens, k);
        k = next;
        while k < tokens.len() && tokens[k] != "," && tokens[k] != close {
            k += 1;
        }
        args.push(term);
    }
    (args, k + 1)
}

/// The variables that stuck calls in term match on.
fn waits(term: &Term, funcs: &HashMap<String, HashSet<Matched>>) -> HashSet<String> {
    let mut found = HashSet::new();
    if term.kind == Kind::Call {
        for (index, path) in funcs.get(&term.name).into_iter().flatten() {
            let Some(mut arg) = term.args.get(*index) else {
                continue;
            };
            for &i in path {
                if arg.kind == Kind::Var || i >= arg.args.len() {
                    break;
                }
                arg = &arg.args[i];
            }
            if arg.kind == Kind::Var {
                found.insert(arg.name.clone());
            }
        }
    }
    for arg in &term.args {
        found.extend(waits(arg, funcs));
    }
    found
}

// ---------------------------------------------------------------- search

fn lemma(ctx: &Ctx, fact: &str, tree: &Node) -> (String, Vec<String>, HashMap<usize, Vec<usize>>) {
    let sig: Vec<String> = params(fact)
        .iter()
        .map(|(n, t)| format!("{}{n}: {t}", if *n == "status" { "+" } else { "" }))
        .collect();
    let (lines, leaves) = emit(tree, 2);
    let head = format!(
        "def fits_{}(\n  {}\n) ->\n  {}:\n",
        snake(fact),
        sig.join(", "),
        ctx.goal(fact)
    );
    (head, lines, leaves)
}

/// Runs bun on a lemma file, with stdout and stderr joined.
fn check(bend: &str, work: &Path, file: &str) -> String {
    let mut child = Command::new("bun")
        .args([bend, file])
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("bun: {e}"));
    let read = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = vec![];
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(Box::new(child.stdout.take().expect("piped")));
    let stderr = read(Box::new(child.stderr.take().expect("piped")));
    let deadline = Instant::now() + Duration::from_secs(900);
    while child.try_wait().expect("bun status").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return format!("{file}: bun timed out after 900 s");
        }
        thread::sleep(Duration::from_millis(20));
    }
    stdout.join().expect("reader") + &stderr.join().expect("reader")
}

/// The source line of Bend's first `N>|` excerpt line.
fn failing_line(out: &str) -> Option<usize> {
    out.split('\n').find_map(|line| {
        let rest = line.trim_start();
        let (number, _) = rest.split_once(">|")?;
        number.parse().ok()
    })
}

/// The rest of the line after `- KEY: `.
fn reported<'a>(out: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("- {key}");
    out.match_indices(&marker).find_map(|(i, _)| {
        let rest = out[i + marker.len()..].trim_start().strip_prefix(": ")?;
        Some(rest.split('\n').next().unwrap_or(""))
    })
}

fn search(
    ctx: &Ctx,
    fact: &str,
    mut tree: Node,
    work: &Path,
    funcs: &HashMap<String, HashSet<Matched>>,
    bend: &str,
) -> (Node, Option<String>) {
    const LIMIT: usize = 5000;
    for _ in 0..LIMIT {
        let (head, lines, leaves) = lemma(ctx, fact, &tree);
        let top = format!("{}\n{}\n{head}", header("./"), ctx.choose_def());
        let offset = top.matches('\n').count();
        let file = format!("fits_{}.bend", snake(fact));
        fs::write(work.join(&file), format!("{top}{}\n", lines.join("\n"))).expect("write lemma");
        let out = check(bend, work, &file);
        if out.contains("ALL PROOFS CHECK") {
            return (tree, None);
        }
        let Some((at, where_)) = failing_line(&out)
            .and_then(|n| n.checked_sub(1 + offset))
            .and_then(|at| Some((at, leaves.get(&at)?.clone())))
        else {
            return (tree, Some(out));
        };
        let mut candidates = HashSet::new();
        for key in ["expected", "observed"] {
            if let Some(text) = reported(&out, key) {
                candidates.extend(waits(&parse_term(&tokens(text), 0).0, funcs));
            }
        }
        let avail = unsplit(ctx, fact, &tree, &where_);
        let order = priority(fact);
        let first = candidates
            .iter()
            .filter(|c| avail.contains(*c))
            .min_by_key(|c| order[c.as_str()])
            .cloned();
        if let Some(var) = first {
            tree = insert(ctx, fact, tree, &where_, &var);
        } else if out.contains("Laws.same_status(") && lines[at].trim() == "{==}" {
            set_leaf(&mut tree, &where_, STATUS_LEAF);
        } else {
            return (tree, Some(out));
        }
    }
    (tree, Some(format!("no proof after {LIMIT} splits")))
}

fn dispatcher(ctx: &Ctx) -> String {
    let session = session_term();
    let flags = flags_term();
    let mut out = vec![
        "# Every fact: the per-fact lemma, with the future flag step computes.".to_string(),
        "def step_fits(state: Session.State, flags: Boundary.Flags, fact: Boundary.Fact) ->".into(),
        format!(
            "  {}:",
            ctx.statement(
                "fact",
                "state",
                "Session.step(fact, flags, state)",
                "flags",
                ctx.law.future
            )
        ),
        "  match state:".into(),
        "    case Session.State{session, shown, tracker}:".into(),
        "      match session:".into(),
        format!("        case {session}:"),
    ];

    fn facts(out: &mut Vec<String>, ind: usize, terms: &BTreeMap<&str, String>, flags: &str) {
        let pad = " ".repeat(ind);
        out.extend([
            format!("{pad}match shown:"),
            format!("{pad}  case Session.Shown{{+revision, was}}:"),
            format!("{pad}    match flags:"),
            format!("{pad}      case {flags}:"),
            format!("{pad}        match fact:"),
        ]);
        for (fact, fs) in FACTS {
            // Each field is an argument and part of the fact future_of reads.
            let binders: Vec<String> = fs.iter().map(|(n, _)| format!("+{n}")).collect();
            let mut args: Vec<String> = fs.iter().map(|(n, _)| n.to_string()).collect();
            args.extend(
                STATE
                    .iter()
                    .filter(|(n, _)| *n != "future")
                    .map(|(n, _)| terms.get(n).cloned().unwrap_or_else(|| n.to_string())),
            );
            args.insert(
                fs.len() + STATE.len() - 1,
                format!("Session.future_of({}, revision)", fact_term(fact)),
            );
            args.extend(FLAG_ORDER.iter().map(|f| f.to_string()));
            out.push(format!(
                "{pad}          case Boundary.{fact}{{{}}}:",
                binders.join(", ")
            ));
            out.push(format!("{pad}            fits_{}(", snake(fact)));
            out.push(format!("{pad}              {}", args.join(", ")));
            out.push(format!("{pad}            )"));
        }
    }

    fn splits(
        out: &mut Vec<String>,
        ind: usize,
        rest: &[(&'static str, Vec<String>)],
        terms: &BTreeMap<&str, String>,
        flags: &str,
    ) {
        let Some(((var, patterns), rest)) = rest.split_first() else {
            return facts(out, ind, terms, flags);
        };
        out.push(format!("{}match {var}:", " ".repeat(ind)));
        for pattern in patterns {
            out.push(format!("{}  case {pattern}:", " ".repeat(ind)));
            let mut terms = terms.clone();
            terms.insert(var, pattern.clone());
            splits(out, ind + 4, rest, &terms, flags);
        }
    }

    splits(&mut out, 10, &ctx.law.splits, &BTreeMap::new(), &flags);
    out.join("\n") + "\n"
}

fn write(ctx: &Ctx, trees: &HashMap<&str, Node>) {
    let mut parts = vec![
        header("../"),
        "\n# Generated by tools/step-proof.rs, which searches the case splits;\n\
         # rerun it after changing step (src/session.bend) or the automaton in\n# LAWS.bend.\n\n"
            .to_string(),
        ctx.choose_def(),
    ];
    for (fact, _) in FACTS {
        let (head, lines, _) = lemma(ctx, fact, &trees[fact]);
        parts.push(format!("\n{head}{}\n", lines.join("\n")));
    }
    parts.push(format!("\n{}", dispatcher(ctx)));
    fs::write(&ctx.target, parts.concat()).expect("write proof");
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create directory");
    for entry in fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

/// LAWS.bend without its `law` blocks: open laws would hide whether the lemma
/// checks.
fn without_laws(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("\nlaw ") {
        let line_end = rest[i + 1..].find('\n').map(|j| i + 1 + j);
        match line_end {
            Some(end) if rest[i + 1..end].ends_with(':') => {
                let mut stop = end + 1;
                while rest[stop..].starts_with("  ") {
                    match rest[stop..].find('\n') {
                        Some(j) => stop += j + 1,
                        None => break,
                    }
                }
                out.push_str(&rest[..i]);
                out.push('\n');
                rest = &rest[stop..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = &rest[i + 1..];
            }
        }
    }
    out + rest
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(law) = args
        .get(1)
        .map(String::as_str)
        .and_then(law)
        .filter(|_| args.len() >= 3)
    else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let root = env::current_dir().expect("working directory");
    let target = root.join("proof").join(law.file);
    let enums = enums();
    let patterns = enums.values().flatten().cloned().collect();
    let ctx = Ctx {
        law,
        root,
        target,
        enums,
        patterns,
    };
    let bend = args[2].as_str();
    let mut chosen: Vec<&'static str> = vec![];
    for name in args.get(3).map_or(vec![], |list| list.split(',').collect()) {
        let Some((fact, _)) = FACTS.iter().find(|(f, _)| *f == name) else {
            eprintln!("unknown fact {name}\n{USAGE}");
            return ExitCode::FAILURE;
        };
        chosen.push(fact);
    }
    if chosen.is_empty() {
        chosen = FACTS.iter().map(|(f, _)| *f).collect();
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let work = env::temp_dir().join(format!("step-proof-{}-{stamp}", std::process::id()));
    copy_tree(&ctx.root.join("src"), &work.join("src"));
    fs::create_dir(work.join("proof")).expect("create proof directory");
    fs::copy(
        ctx.root.join("proof/word.bend"),
        work.join("proof/word.bend"),
    )
    .expect("copy word.bend");
    let laws = fs::read_to_string(ctx.root.join("LAWS.bend")).expect("read LAWS.bend");
    fs::write(work.join("LAWS.bend"), without_laws(&laws)).expect("write LAWS.bend");
    let base = Path::new(bend)
        .parent()
        .unwrap_or(Path::new("."))
        .join("base.bend");
    let mut funcs = matched_paths(&base, None);
    for (module, alias) in MODULES {
        funcs.extend(matched_paths(
            &work.join("src").join(format!("{module}.bend")),
            Some(alias),
        ));
    }
    funcs.extend(matched_paths(&work.join("LAWS.bend"), Some("Laws")));
    let mut trees = existing_trees(&ctx);
    let jobs: usize = env::var("JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(8)
        .max(1);

    // Checks the chosen facts on JOBS threads and reports them in order.
    let taken = AtomicUsize::new(0);
    let (sender, results) = mpsc::channel();
    let mut failed = false;
    let found = thread::scope(|scope| {
        for _ in 0..jobs {
            let (taken, chosen, sender, ctx, trees, funcs, work) =
                (&taken, &chosen, sender.clone(), &ctx, &trees, &funcs, &work);
            scope.spawn(move || {
                loop {
                    let index = taken.fetch_add(1, Ordering::Relaxed);
                    let Some(&fact) = chosen.get(index) else {
                        break;
                    };
                    let tree = trees.get(fact).cloned().unwrap_or_else(leaf);
                    let _ = sender.send((index, fact, search(ctx, fact, tree, work, funcs, bend)));
                }
            });
        }
        drop(sender);
        let mut pending = BTreeMap::new();
        let mut next = 0;
        let mut found = vec![];
        for (index, fact, result) in results {
            pending.insert(index, (fact, result));
            while let Some((fact, (tree, error))) = pending.remove(&next) {
                println!("{fact}: {}", if error.is_none() { "ok" } else { "FAILED" });
                if let Some(error) = error {
                    failed = true;
                    println!("{}", error.chars().take(4000).collect::<String>());
                }
                let _ = std::io::stdout().flush();
                found.push((fact, tree));
                next += 1;
            }
        }
        found
    });
    trees.extend(found);
    if FACTS.iter().all(|(f, _)| trees.contains_key(f)) {
        write(&ctx, &trees);
    }
    let _ = fs::remove_dir_all(&work);
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
