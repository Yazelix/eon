//! Generates the boundary between the Rust host and the Bend worker
//! (docs/HOST.md) from its schema: `boundary SCHEMA DIR` writes
//! `DIR/boundary.rs` and `DIR/boundary.bend`. The schema's header describes
//! its format.

use std::{collections::HashSet, env, fmt::Write as _, fs, path::Path, process::ExitCode};

struct Flag {
    doc: Vec<String>,
    bit: u32,
    name: String,
}

struct Variant {
    doc: Vec<String>,
    value: u32,
    name: String,
}

struct Enum {
    doc: Vec<String>,
    name: String,
    variants: Vec<Variant>,
}

#[derive(Clone, PartialEq)]
enum Type {
    U32,
    Bool,
    /// A presentation revision: two words, its high and low halves.
    Revision,
    Enum(String),
}

impl Type {
    /// The words a field of this type takes on the pipe.
    fn words(&self) -> usize {
        match self {
            Self::Revision => 2,
            _ => 1,
        }
    }
}

/// A fact or a decision.
struct Item {
    doc: Vec<String>,
    code: u32,
    name: String,
    fields: Vec<(String, Type)>,
}

#[derive(Default)]
struct Schema {
    flags: Vec<Flag>,
    enums: Vec<Enum>,
    facts: Vec<Item>,
    decisions: Vec<Item>,
}

impl Schema {
    fn uses(items: &[Item], kind: &Type) -> bool {
        items
            .iter()
            .any(|item| item.fields.iter().any(|(_, field)| field == kind))
    }
}

fn number(word: &str, line: usize) -> Result<u32, String> {
    word.parse()
        .map_err(|_| format!("line {line}: `{word}` is not a u32"))
}

fn parse(text: &str) -> Result<Schema, String> {
    let mut schema = Schema::default();
    let mut doc = Vec::new();
    // Whether indented lines still belong to the last enum.
    let mut in_enum = false;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            doc.clear();
            in_enum = false;
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            doc.push(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let doc = std::mem::take(&mut doc);
        if raw.starts_with(char::is_whitespace) {
            let (true, Some(owner), [value, name]) = (in_enum, schema.enums.last_mut(), &words[..])
            else {
                return Err(format!("line {line}: expected `VALUE Variant` in an enum"));
            };
            owner.variants.push(Variant {
                doc,
                value: number(value, line)?,
                name: (*name).to_owned(),
            });
            continue;
        }
        in_enum = false;
        match &words[..] {
            ["flag", bit, name] => schema.flags.push(Flag {
                doc,
                bit: number(bit, line)?,
                name: (*name).to_owned(),
            }),
            ["enum", name] => {
                schema.enums.push(Enum {
                    doc,
                    name: (*name).to_owned(),
                    variants: Vec::new(),
                });
                in_enum = true;
            }
            [keyword @ ("fact" | "decision"), code, name, rest @ ..] => {
                let fields = rest
                    .iter()
                    .map(|field| {
                        let (field, kind) = field.split_once(':').ok_or_else(|| {
                            format!("line {line}: expected `field:TYPE`, found `{field}`")
                        })?;
                        let kind = match kind {
                            "u32" => Type::U32,
                            "bool" => Type::Bool,
                            "revision" => Type::Revision,
                            name => Type::Enum(name.to_owned()),
                        };
                        Ok((field.to_owned(), kind))
                    })
                    .collect::<Result<_, String>>()?;
                let item = Item {
                    doc,
                    code: number(code, line)?,
                    name: (*name).to_owned(),
                    fields,
                };
                if *keyword == "fact" {
                    schema.facts.push(item);
                } else {
                    schema.decisions.push(item);
                }
            }
            _ => {
                return Err(format!(
                    "line {line}: expected `flag`, `enum`, `fact` or `decision`"
                ));
            }
        }
    }
    validate(&schema)?;
    Ok(schema)
}

fn unique<T: Eq + std::hash::Hash>(
    items: impl IntoIterator<Item = T>,
    what: &str,
    show: impl Fn(&T) -> String,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for item in items {
        let label = show(&item);
        if !seen.insert(item) {
            return Err(format!("duplicate {what} {label}"));
        }
    }
    Ok(())
}

fn validate(schema: &Schema) -> Result<(), String> {
    for flag in &schema.flags {
        if flag.bit > 31 {
            return Err(format!("flag {} has bit {} past 31", flag.name, flag.bit));
        }
    }
    unique(
        schema.flags.iter().map(|flag| flag.bit),
        "flag bit",
        u32::to_string,
    )?;
    unique(schema.flags.iter().map(|flag| &flag.name), "flag", |name| {
        name.to_string()
    })?;
    unique(
        schema.facts.iter().map(|fact| fact.code),
        "fact code",
        u32::to_string,
    )?;
    unique(
        schema.decisions.iter().map(|decision| decision.code),
        "decision code",
        u32::to_string,
    )?;
    for item in &schema.enums {
        if item.variants.is_empty() {
            return Err(format!("enum {} has no variants", item.name));
        }
        unique(
            item.variants.iter().map(|variant| variant.value),
            "value",
            |value| format!("{value} in enum {}", item.name),
        )?;
    }
    for item in schema.facts.iter().chain(&schema.decisions) {
        unique(
            item.fields.iter().map(|(field, _)| field),
            "field",
            |field| format!("{field} in {}", item.name),
        )?;
        for (_, kind) in &item.fields {
            if let Type::Enum(name) = kind
                && !schema.enums.iter().any(|known| &known.name == name)
            {
                return Err(format!("{} names unknown type {name}", item.name));
            }
        }
    }
    // Bend constructors and types share one namespace per module.
    let names = schema
        .enums
        .iter()
        .flat_map(|item| {
            item.variants
                .iter()
                .map(|variant| format!("{}{}", item.name, variant.name))
        })
        .chain(
            schema
                .enums
                .iter()
                .map(|item| format!("type {}", item.name)),
        )
        .chain(
            schema
                .facts
                .iter()
                .chain(&schema.decisions)
                .map(|item| item.name.clone()),
        );
    unique(names, "name", Clone::clone)
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn doc(out: &mut String, indent: &str, marker: &str, lines: &[String]) {
    for line in lines {
        if line.is_empty() {
            writeln!(out, "{indent}{marker}").unwrap();
        } else {
            writeln!(out, "{indent}{marker} {line}").unwrap();
        }
    }
}

const HEADER: &str = "Generated by tools/boundary.rs from src/boundary.schema, which defines the \
boundary (docs/HOST.md); edit the schema and regenerate.";

fn wrap(prefix: &str, text: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && prefix.len() + line.len() + 1 + word.len() > 79 {
            writeln!(out, "{prefix}{line}").unwrap();
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    writeln!(out, "{prefix}{line}").unwrap();
    out
}

fn rust_type(kind: &Type) -> &str {
    match kind {
        Type::U32 => "u32",
        Type::Bool => "bool",
        Type::Revision => "u64",
        Type::Enum(name) => name,
    }
}

/// A variant: bare without fields, a tuple around one enum, else a struct.
fn rust_variant(out: &mut String, item: &Item) {
    doc(out, "    ", "///", &item.doc);
    match &item.fields[..] {
        [] => writeln!(out, "    {},", item.name),
        [(_, Type::Enum(name))] => writeln!(out, "    {}({name}),", item.name),
        fields => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(field, kind)| format!("{field}: {}", rust_type(kind)))
                .collect();
            // rustfmt's struct_variant_width.
            if fields.join(", ").len() <= 35 {
                writeln!(out, "    {} {{ {} }},", item.name, fields.join(", "))
            } else {
                writeln!(
                    out,
                    "    {} {{\n        {},\n    }},",
                    item.name,
                    fields.join(",\n        ")
                )
            }
        }
    }
    .unwrap();
}

fn rust(schema: &Schema, identity: &str) -> String {
    let mut out = wrap("//! ", HEADER);
    for item in &schema.enums {
        out.push('\n');
        doc(&mut out, "", "///", &item.doc);
        writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
        writeln!(out, "pub(crate) enum {} {{", item.name).unwrap();
        for variant in &item.variants {
            doc(&mut out, "    ", "///", &variant.doc);
            writeln!(out, "    {} = {},", variant.name, variant.value).unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
    out.push_str("\n/// The flags word every fact carries.\n");
    out.push_str("#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]\n");
    out.push_str("pub(crate) struct Flags {\n");
    for flag in &schema.flags {
        doc(&mut out, "    ", "///", &flag.doc);
        writeln!(out, "    pub(crate) {}: bool,", flag.name).unwrap();
    }
    out.push_str("}\n\nimpl Flags {\n    fn word(self) -> u32 {\n        ");
    for (index, flag) in schema.flags.iter().enumerate() {
        if index > 0 {
            out.push_str("\n            | ");
        }
        write!(out, "u32::from(self.{})", flag.name).unwrap();
        if flag.bit > 0 {
            write!(out, " << {}", flag.bit).unwrap();
        }
    }
    if schema.flags.is_empty() {
        out.push('0');
    }
    out.push_str("\n    }\n}\n\n/// One fact for Bend.\n");
    out.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub(crate) enum Fact {\n");
    for fact in &schema.facts {
        rust_variant(&mut out, fact);
    }
    out.push_str(
        "}\n\nimpl Fact {\n    /// The fact's line on the pipe: `CODE FLAGS ARG...`.\n    \
         pub(crate) fn line(self, flags: Flags) -> String {\n        \
         let word = flags.word();\n        match self {\n",
    );
    for fact in &schema.facts {
        let names: Vec<&str> = fact
            .fields
            .iter()
            .map(|(field, _)| field.as_str())
            .collect();
        // rustfmt's struct_lit_width breaks a wider pattern, one field a line.
        let wide = names.join(", ").len() > 18;
        let pattern = match &fact.fields[..] {
            [] => String::new(),
            [(field, Type::Enum(_))] => format!("({field})"),
            _ if wide => format!(
                " {{\n                {},\n            }}",
                names.join(",\n                ")
            ),
            _ => format!(" {{ {} }}", names.join(", ")),
        };
        let mut text = format!("{} {{word}}", fact.code);
        let mut args = String::new();
        for (field, kind) in &fact.fields {
            match kind {
                Type::U32 => write!(text, " {{{field}}}").unwrap(),
                Type::Bool => {
                    text.push_str(" {}");
                    write!(args, ", u32::from({field})").unwrap();
                }
                Type::Revision => {
                    text.push_str(" {}");
                    write!(args, ", halves({field})").unwrap();
                }
                Type::Enum(_) => {
                    text.push_str(" {}");
                    write!(args, ", {field} as u32").unwrap();
                }
            }
        }
        let arm = format!("Self::{}{pattern} =>", fact.name);
        let mut body = format!("format!(\"{text}\\n\"{args})");
        // A body past max_width in the block takes one argument a line.
        if format!("                {body}").len() > 100 {
            body = format!(
                "format!(\n                    \"{text}\\n\"{}\n                )",
                args.replace(", ", ",\n                    ")
            );
        }
        // The arm on one line, against rustfmt's max_width; a broken pattern
        // takes a block.
        let width = format!("            {arm} {body},").len();
        if width <= 100 && !wide {
            writeln!(out, "            {arm} {body},").unwrap();
        } else {
            writeln!(
                out,
                "            {arm} {{\n                {body}\n            }}"
            )
            .unwrap();
        }
    }
    out.push_str("        }\n    }\n}\n");
    if Schema::uses(&schema.facts, &Type::Revision) {
        out.push_str(
            "\n/// A revision's two words: its high and low 32 bits.\n\
             fn halves(revision: u64) -> String {\n    \
             format!(\"{} {}\", revision >> 32, revision & 0xffff_ffff)\n}\n",
        );
    }
    writeln!(
        out,
        "\n/// The line the worker writes before its first answer, naming the schema\n\
         /// it was generated from; the host refuses a worker that names another.\n\
         pub(crate) const SCHEMA: &str = \"{identity}\";"
    )
    .unwrap();
    rust_decisions(&mut out, schema);
    out
}

fn rust_decisions(out: &mut String, schema: &Schema) {
    out.push_str(
        "\n/// The line that ends each of the worker's answers.\n\
         pub(crate) const END: &str = \"end\";\n\n/// One decision from Bend.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub(crate) enum Decision {\n",
    );
    for decision in &schema.decisions {
        rust_variant(out, decision);
    }
    out.push_str(
        "}\n\ntype Words<'a> = std::str::Split<'a, char>;\n\n\
         fn number(words: &mut Words<'_>) -> Option<u32> {\n    words.next()?.parse().ok()\n}\n",
    );
    if Schema::uses(&schema.decisions, &Type::Bool) {
        out.push_str(
            "\nfn flag(words: &mut Words<'_>) -> Option<bool> {\n    match number(words)? {\n        \
             0 => Some(false),\n        1 => Some(true),\n        _ => None,\n    }\n}\n",
        );
    }
    if Schema::uses(&schema.decisions, &Type::Revision) {
        out.push_str(
            "\nfn revision(words: &mut Words<'_>) -> Option<u64> {\n    \
             let high = u64::from(number(words)?);\n    \
             Some(high << 32 | u64::from(number(words)?))\n}\n",
        );
    }
    for item in &schema.enums {
        if !Schema::uses(&schema.decisions, &Type::Enum(item.name.clone())) {
            continue;
        }
        writeln!(
            out,
            "\nimpl {} {{\n    fn decode(words: &mut Words<'_>) -> Option<Self> {{\n        \
             Some(match number(words)? {{",
            item.name
        )
        .unwrap();
        for variant in &item.variants {
            writeln!(
                out,
                "            {} => Self::{},",
                variant.value, variant.name
            )
            .unwrap();
        }
        out.push_str("            _ => return None,\n        })\n    }\n}\n");
    }
    out.push_str(
        "\nimpl Decision {\n    /// Decodes one decision line, `CODE ARG...`; a malformed line is None.\n    \
         pub(crate) fn decode(line: &str) -> Option<Self> {\n        \
         let mut words = line.split(' ');\n        \
         let decision = match number(&mut words)? {\n",
    );
    let read = |kind: &Type| match kind {
        Type::U32 => "number(&mut words)?".to_owned(),
        Type::Bool => "flag(&mut words)?".to_owned(),
        Type::Revision => "revision(&mut words)?".to_owned(),
        Type::Enum(name) => format!("{name}::decode(&mut words)?"),
    };
    for decision in &schema.decisions {
        let value = match &decision.fields[..] {
            [] => decision.name.clone(),
            [(_, kind @ Type::Enum(_))] => format!("{}({})", decision.name, read(kind)),
            fields => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(field, kind)| format!("                {field}: {},\n", read(kind)))
                    .collect();
                format!("{} {{\n{}            }}", decision.name, fields.concat())
            }
        };
        writeln!(out, "            {} => Self::{value},", decision.code).unwrap();
    }
    out.push_str(
        "            _ => return None,\n        };\n        \
         words.next().is_none().then_some(decision)\n    }\n}\n",
    );
}

fn bend_type(kind: &Type) -> &str {
    match kind {
        Type::U32 => "U32",
        Type::Bool => "Bool",
        Type::Revision => "Revision.Revision",
        Type::Enum(name) => name,
    }
}

fn bend_constructor(out: &mut String, item: &Item) {
    doc(out, "  ", "#", &item.doc);
    let fields: Vec<String> = item
        .fields
        .iter()
        .map(|(field, kind)| format!("{field}: {}", bend_type(kind)))
        .collect();
    writeln!(out, "  {}{{{}}}", item.name, fields.join(", ")).unwrap();
}

fn bend(schema: &Schema, identity: &str) -> String {
    let mut out = String::from("import Base\n");
    let items = || schema.facts.iter().chain(&schema.decisions);
    if items().any(|item| item.fields.iter().any(|(_, kind)| kind == &Type::Revision)) {
        out.push_str("import ./revision.bend as Revision\n");
    }
    out.push('\n');
    out.push_str(&wrap("# ", HEADER));
    for item in &schema.enums {
        out.push('\n');
        doc(&mut out, "", "#", &item.doc);
        writeln!(out, "type {} is Data:", item.name).unwrap();
        for variant in &item.variants {
            doc(&mut out, "  ", "#", &variant.doc);
            writeln!(out, "  {}{}{{}}", item.name, variant.name).unwrap();
        }
    }
    let names: Vec<&str> = schema.flags.iter().map(|flag| flag.name.as_str()).collect();
    let fields: Vec<String> = names.iter().map(|name| format!("{name}: Bool")).collect();
    out.push_str("\n# The flags word every fact carries; each flag's def below reads it.\n");
    out.push_str("type Flags is Data:\n");
    writeln!(out, "  Flags{{{}}}", fields.join(", ")).unwrap();
    out.push_str("\n# One fact from the host.\ntype Fact is Data:\n");
    for fact in &schema.facts {
        bend_constructor(&mut out, fact);
    }
    out.push_str("\n# One line from the host: a fact and the flags it carries.\n");
    out.push_str("type Line is Data:\n  Line{fact: Fact, flags: Flags}\n");
    let pattern = format!("Flags{{{}}}", names.join(", "));
    for flag in &schema.flags {
        out.push('\n');
        doc(&mut out, "", "#", &flag.doc);
        writeln!(
            out,
            "def {0}(flags: Flags) -> Bool:\n  match flags:\n    case {pattern}:\n      {0}",
            flag.name
        )
        .unwrap();
    }
    out.push_str(
        "\ndef decode_bool(arg: U32) -> Maybe<&2, Bool>:\n  match arg:\n    case 0:\n      \
         Some{False{}}\n    case 1:\n      Some{True{}}\n    case _:\n      None{}\n",
    );
    for item in &schema.enums {
        if !Schema::uses(&schema.facts, &Type::Enum(item.name.clone())) {
            continue;
        }
        writeln!(
            out,
            "\ndef decode_{}(arg: U32) -> Maybe<&2, {}>:\n  match arg:",
            snake(&item.name),
            item.name
        )
        .unwrap();
        for variant in &item.variants {
            writeln!(
                out,
                "    case {}:\n      Some{{{}{}{{}}}}",
                variant.value, item.name, variant.name
            )
            .unwrap();
        }
        out.push_str("    case _:\n      None{}\n");
    }
    // A fact's fields decode from its argument words, in order; any other
    // number of words is malformed.
    for fact in &schema.facts {
        let count = fact.fields.iter().map(|(_, kind)| kind.words()).sum();
        let names: Vec<String> = (0..count).map(|i| format!("a{i}")).collect();
        let mut pattern = String::from("Nil{}");
        for name in names.iter().rev() {
            pattern = format!("Con{{{name}, {pattern}}}");
        }
        writeln!(
            out,
            "\ndef fact_{}(args: List<&2, String>) -> Maybe<&2, Fact>:\n  match args:\n    \
             case {pattern}:",
            snake(&fact.name)
        )
        .unwrap();
        let fields: Vec<String> = fact
            .fields
            .iter()
            .map(|(field, kind)| match kind {
                Type::Revision => format!("Revision.Revision{{{field}_high, {field}_low}}"),
                _ => field.clone(),
            })
            .collect();
        let value = format!("{}{{{}}}", fact.name, fields.join(", "));
        if fact.fields.is_empty() {
            writeln!(out, "      Some{{{value}}}").unwrap();
        } else {
            out.push_str("      do Maybe<&2, Fact>:\n");
            let mut words = names.iter();
            for (field, kind) in &fact.fields {
                let word = words.next().unwrap();
                let (name, decoder) = match kind {
                    Type::U32 => {
                        writeln!(out, "        {field} : U32 <- U32.read({word})").unwrap();
                        continue;
                    }
                    Type::Revision => {
                        let low = words.next().unwrap();
                        writeln!(
                            out,
                            "        {field}_high : U32 <- U32.read({word})\n        \
                             {field}_low : U32 <- U32.read({low})"
                        )
                        .unwrap();
                        continue;
                    }
                    Type::Bool => ("Bool", "decode_bool".to_owned()),
                    Type::Enum(name) => (name.as_str(), format!("decode_{}", snake(name))),
                };
                writeln!(
                    out,
                    "        {field}_word : U32 <- U32.read({word})\n        \
                     {field} : {name} <- {decoder}({field}_word)"
                )
                .unwrap();
            }
            writeln!(out, "        return {value}").unwrap();
        }
        out.push_str("    case _:\n      None{}\n");
    }
    out.push_str(
        "\ndef decode_fact(code: U32, args: List<&2, String>) -> Maybe<&2, Fact>:\n  \
         match code:\n",
    );
    for fact in &schema.facts {
        writeln!(
            out,
            "    case {}:\n      fact_{}(args)",
            fact.code,
            snake(&fact.name)
        )
        .unwrap();
    }
    out.push_str("    case _:\n      None{}\n");
    let known = schema
        .flags
        .iter()
        .fold(0u32, |mask, flag| mask | 1 << flag.bit);
    let reads: Vec<String> = schema
        .flags
        .iter()
        .map(|flag| format!("decode_bit(word, {})", 1u32 << flag.bit))
        .collect();
    out.push_str(
        "\ndef decode_bit(+word: U32, mask: U32) -> Bool:\n  \
         Bool.not(U32.is_zero(U32.and(word, mask)))\n",
    );
    writeln!(
        out,
        "\ndef decode_known_flags(known: Bool, +word: U32) -> Maybe<&2, Flags>:\n  match known:\n    \
         case True{{}}:\n      Some{{Flags{{{}}}}}\n    case False{{}}:\n      None{{}}",
        reads.join(", ")
    )
    .unwrap();
    writeln!(
        out,
        "\n# A word with a bit no flag names is malformed.\n\
         def decode_flags(+word: U32) -> Maybe<&2, Flags>:\n  \
         decode_known_flags(U32.is_zero(U32.and(word, {})), word)",
        !known
    )
    .unwrap();
    out.push_str(
        "\ndef decode_words(words: List<&2, String>) -> Maybe<&2, Line>:\n  match words:\n    \
         case Con{code, Con{flags, args}}:\n      do Maybe<&2, Line>:\n        \
         number : U32 <- U32.read(code)\n        word : U32 <- U32.read(flags)\n        \
         fact : Fact <- decode_fact(number, args)\n        \
         decoded : Flags <- decode_flags(word)\n        return Line{fact, decoded}\n    \
         case _:\n      None{}\n",
    );
    out.push_str(
        "\n# Decodes one fact line, `CODE FLAGS ARG...`; a malformed line is None.\n\
         def decode(text: String) -> Maybe<&2, Line>:\n  \
         decode_words(String.split(String.trim(text), ' '))\n",
    );
    bend_decisions(&mut out, schema);
    writeln!(
        out,
        "\n# The line the worker writes before its first answer, naming the schema it\n\
         # was generated from; the host refuses a worker that names another.\n\
         def schema() -> String:\n  \"{identity}\\n\""
    )
    .unwrap();
    out
}

fn bend_decisions(out: &mut String, schema: &Schema) {
    out.push_str("\n# One decision for the host.\ntype Decision is Data:\n");
    for decision in &schema.decisions {
        bend_constructor(out, decision);
    }
    if Schema::uses(&schema.decisions, &Type::Bool) {
        out.push_str(
            "\ndef encode_bool(value: Bool) -> String:\n  match value:\n    case False{}:\n      \
             \"0\"\n    case True{}:\n      \"1\"\n",
        );
    }
    if Schema::uses(&schema.decisions, &Type::Revision) {
        out.push_str(
            "\ndef encode_revision(value: Revision.Revision) -> String:\n  match value:\n    \
             case Revision.Revision{high, low}:\n      U32.show(high) ++ \" \" ++ U32.show(low)\n",
        );
    }
    for item in &schema.enums {
        if !Schema::uses(&schema.decisions, &Type::Enum(item.name.clone())) {
            continue;
        }
        writeln!(
            out,
            "\ndef encode_{}(value: {}) -> String:\n  match value:",
            snake(&item.name),
            item.name
        )
        .unwrap();
        for variant in &item.variants {
            writeln!(
                out,
                "    case {}{}{{}}:\n      \"{}\"",
                item.name, variant.name, variant.value
            )
            .unwrap();
        }
    }
    out.push_str("\ndef encode_decision(decision: Decision) -> String:\n  match decision:\n");
    for decision in &schema.decisions {
        let names: Vec<&str> = decision
            .fields
            .iter()
            .map(|(field, _)| field.as_str())
            .collect();
        let words: String = decision
            .fields
            .iter()
            .map(|(field, kind)| match kind {
                Type::U32 => format!(" ++ \" \" ++ U32.show({field})"),
                Type::Bool => format!(" ++ \" \" ++ encode_bool({field})"),
                Type::Revision => format!(" ++ \" \" ++ encode_revision({field})"),
                Type::Enum(name) => format!(" ++ \" \" ++ encode_{}({field})", snake(name)),
            })
            .collect();
        writeln!(
            out,
            "    case {}{{{}}}:\n      \"{}\"{words} ++ \"\\n\"",
            decision.name,
            names.join(", "),
            decision.code
        )
        .unwrap();
    }
    out.push_str(
        "\n# One answer: decisions, carried out in order.\n\
         def Decisions() -> Type:\n  List<&2, Decision>\n\
         \n# Encodes one answer: a line per decision and the end line.\n\
         def encode(decisions: Decisions()) -> String:\n  match decisions:\n    \
         case Nil{}:\n      \"end\\n\"\n    case Con{decision, rest}:\n      \
         encode_decision(decision) ++ encode(rest)\n",
    );
}

/// The schema's identity line: the 64-bit FNV-1a digest of its text, in hex.
/// It tells builds apart; it is no defense against an attacker.
fn identity(text: &str) -> String {
    let digest = text
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |digest, byte| {
            (digest ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("schema {digest:016x}")
}

fn run(schema: &Path, dir: &Path) -> Result<(), String> {
    let text =
        fs::read_to_string(schema).map_err(|error| format!("{}: {error}", schema.display()))?;
    let identity = identity(&text);
    let schema = parse(&text)?;
    for (name, text) in [
        ("boundary.rs", rust(&schema, &identity)),
        ("boundary.bend", bend(&schema, &identity)),
    ] {
        let path = dir.join(name);
        fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let [_, schema, dir] = &args[..] else {
        eprintln!("usage: boundary SCHEMA DIR");
        return ExitCode::from(2);
    };
    match run(Path::new(schema), Path::new(dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("boundary: {error}");
            ExitCode::FAILURE
        }
    }
}
