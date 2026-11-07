//! A small JSON value with a writer and a parser, enough for eon-bench's own
//! files. Numbers are f64, written in Rust's shortest form that parses back to
//! the same value; objects keep their key order.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::String(text.to_owned())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::String(text)
    }
}

impl From<f64> for Value {
    fn from(number: f64) -> Self {
        Value::Number(number)
    }
}

impl From<u64> for Value {
    fn from(number: u64) -> Self {
        Value::Number(number as f64)
    }
}

impl From<bool> for Value {
    fn from(flag: bool) -> Self {
        Value::Bool(flag)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Value::Null, Into::into)
    }
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// A number that is a whole value a u64 holds exactly.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64()
            .filter(|n| n.fract() == 0.0 && (0.0..=MAX_EXACT).contains(n))
            .map(|n| n as u64)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// Two-space indented text with a final newline. A number that is not
    /// finite has no JSON form and is written as null.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(flag) => write!(out, "{flag}").unwrap(),
            Value::Number(number) if number.is_finite() => write!(out, "{number}").unwrap(),
            Value::Number(_) => out.push_str("null"),
            Value::String(text) => write_string(out, text),
            Value::Array(items) if items.is_empty() => out.push_str("[]"),
            Value::Object(entries) if entries.is_empty() => out.push_str("{}"),
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Value::Object(entries) => {
                out.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }

    pub fn parse(text: &str) -> Result<Value, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            at: 0,
        };
        let value = parser.value(0)?;
        parser.space();
        if parser.at != parser.bytes.len() {
            return Err(parser.error("trailing text"));
        }
        Ok(value)
    }
}

/// The largest integer an f64 holds exactly, 2^53.
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;
const MAX_DEPTH: usize = 64;

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("JSON: {what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        let found = self.bytes.get(self.at) == Some(&byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.eat(byte) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", byte as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        self.space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut entries = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let key = self.string()?;
                        self.expect(b':')?;
                        entries.push((key, self.value(depth + 1)?));
                        if self.eat(b'}') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                Ok(Value::Object(entries))
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value(depth + 1)?);
                        if self.eat(b']') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                Ok(Value::Array(items))
            }
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap();
        // Rust also accepts forms JSON does not, such as `1.` and `.5`; eon-bench
        // only reads files it wrote, so it does not reject them.
        text.parse()
            .map(Value::Number)
            .map_err(|_| self.error("bad number"))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return Err(self.error("expected a string"));
        }
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while self
                .bytes
                .get(self.at)
                .is_some_and(|b| !matches!(b, b'"' | b'\\') && *b >= b' ')
            {
                self.at += 1;
            }
            // The input is a &str and the run stops only at ASCII bytes, so
            // it is whole UTF-8.
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).unwrap());
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = *self
                        .bytes
                        .get(self.at)
                        .ok_or_else(|| self.error("unterminated string"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("bad escape")),
                    }
                }
                _ => return Err(self.error("unterminated string")),
            }
        }
    }

    /// The code point after `\u`, joining a UTF-16 surrogate pair.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&high) {
            if !self.bytes[self.at..].starts_with(b"\\u") {
                return Err(self.error("unpaired surrogate"));
            }
            self.at += 2;
            let low = self.hex4()?;
            if !(0xdc00..0xe000).contains(&low) {
                return Err(self.error("unpaired surrogate"));
            }
            0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
        } else {
            high
        };
        char::from_u32(code).ok_or_else(|| self.error("bad \\u escape"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.at += 4;
        Ok(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    /// Values survive writing and parsing, including escapes, non-ASCII text,
    /// exact large integers and fractions that need every digit; parsing reads
    /// other writers' escapes and spacing and refuses malformed text.
    #[test]
    fn values_round_trip_and_malformed_text_is_refused() {
        let value = Value::Object(vec![
            ("text".into(), "tab\t \"q\" \\ \u{1} 漢 🙂".into()),
            ("big".into(), Value::from(67_108_884u64)),
            ("exact".into(), Value::from(9_007_199_254_740_992u64)),
            ("third".into(), Value::from(1.0 / 3.0)),
            ("tiny".into(), Value::from(1e-9)),
            ("negative".into(), Value::from(-2.5)),
            ("empty".into(), Value::Array(vec![])),
            (
                "nested".into(),
                Value::Array(vec![Value::Null, true.into(), Value::Object(vec![])]),
            ),
        ]);
        let text = value.to_pretty();
        assert_eq!(Value::parse(&text).unwrap(), value);
        assert!(text.contains("\"big\": 67108884,"), "{text}");
        assert_eq!(
            Value::parse(&text).unwrap().get("exact").unwrap().as_u64(),
            Some(1 << 53)
        );
        assert_eq!(
            Value::parse(r#" {"a":[1e3, "\u00e9\ud83d\ude42\/"] } "#).unwrap(),
            Value::Object(vec![(
                "a".into(),
                Value::Array(vec![1000.0.into(), "é🙂/".into()])
            )])
        );
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "\"open",
            "\"\\ud800\"",
            "nul",
            "1 2",
            "\"bad\\q\"",
            &"[".repeat(100),
        ] {
            assert!(Value::parse(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(Value::Number(f64::NAN).to_pretty(), "null\n");
    }
}
