//! RFC 8785 JSON Canonicalization Scheme over `serde_json::Value`.
//!
//! - Object members are sorted by their names' UTF-16 code units (§3.2.3).
//! - Strings use serde_json's escaper, which is exactly §3.2.2.2: `\b \t \n \f \r`, other
//!   controls as lowercase `\u00xx`, `"` and `\` escaped, everything else (`/`, DEL, non-ASCII)
//!   written as UTF-8.
//! - Integers outside I-JSON's exact range, ±(2^53 − 1), are refused (RFC 7493 §2.2), as are
//!   non-finite numbers. Other numbers are written as ECMAScript's `Number.prototype.toString`
//!   writes them (§3.2.2.3).
//!
//! Parsing is not this module's: serde_json's default float parser can be one unit in the last
//! place off for long mantissas (`333333333.33333329`), so a stored canonical form would not
//! re-parse to the same double. The record types therefore accept only numbers it reads
//! exactly: integers, and the one decimal field limited to three decimal places.

use crate::{Result, fail};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};
use std::collections::HashSet;
use std::fmt;

const MAX_EXACT: u64 = (1 << 53) - 1;

/// The canonical form of `value`.
pub fn canonical(value: &Value) -> Result<String> {
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out)
}

fn write(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&write_number(number)?),
        Value::String(text) => out.push_str(&serde_json::to_string(text)?),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out)?;
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut members: Vec<_> = members.iter().collect();
            members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push('{');
            for (index, (name, item)) in members.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(name)?);
                out.push(':');
                write(item, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(number: &Number) -> Result<String> {
    if let Some(value) = number.as_u64() {
        if value > MAX_EXACT {
            fail!("integer {value} is outside the exact range ±(2^53 - 1)");
        }
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        if value.unsigned_abs() > MAX_EXACT {
            fail!("integer {value} is outside the exact range ±(2^53 - 1)");
        }
        return Ok(value.to_string());
    }
    match number.as_f64() {
        Some(value) => ecmascript(value),
        None => fail!("number {number} is not representable"),
    }
}

/// ECMAScript `Number.prototype.toString` for a finite double.
pub fn ecmascript(value: f64) -> Result<String> {
    if !value.is_finite() {
        fail!("{value} is not a finite number");
    }
    if value == 0.0 {
        return Ok("0".into());
    }
    // Rust's `{:e}` gives the shortest digits that round-trip and, among those, the closest,
    // as ECMAScript requires.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`{:e}` always writes an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}` writes an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    // ECMAScript's n: the value is 0.<digits> × 10^n.
    let n = exponent + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e >= 0 { "+" } else { "-" };
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        format!("{first}{fraction}e{sign}{}", e.abs())
    };
    let sign = if value < 0.0 { "-" } else { "" };
    Ok(format!("{sign}{body}"))
}

/// Refuses JSON text with a repeated member name in any object. RFC 8785 input is I-JSON,
/// which forbids them, and `serde_json::Value` would silently keep the last one.
pub fn reject_duplicate_names(input: &[u8]) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    Unique.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(())
}

/// Walks one JSON value, refusing an object that repeats a member name.
struct Unique;

impl<'de> DeserializeSeed<'de> for Unique {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Unique {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while seq.next_element_seed(Unique)?.is_some() {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        let mut seen = HashSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !seen.insert(name.clone()) {
                return Err(de::Error::custom(format!(
                    "member name {name:?} appears twice in one object"
                )));
            }
            map.next_value_seed(Unique)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jcs(text: &str) -> Result<String> {
        canonical(&serde_json::from_str(text).unwrap())
    }

    #[test]
    fn rfc8785_sample_object() {
        // RFC 8785 §3.2.2's input and output. The numbers come from Rust literals, which rustc
        // parses exactly (see the module note on serde_json's parser).
        let mut input: Value = serde_json::from_str(
            r#"{
              "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
              "literals": [null, true, false]
            }"#,
        )
        .unwrap();
        let first: f64 = "333333333.33333329".parse().unwrap();
        input["numbers"] = serde_json::json!([first, 1e30, 4.50, 2e-3, 1e-27]);
        assert_eq!(
            canonical(&input).unwrap(),
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"string":"EURO$\u000f\nA'B\"\\\\\"/"}"#
                .replace("EURO", "\u{20ac}")
        );
    }

    #[test]
    fn members_sort_by_utf16_code_units() {
        // RFC 8785 §3.2.3's sorting example: U+1F600 (surrogates D83D DE00) sorts before
        // U+FB33, although it is the larger code point.
        let input = r#"{"\u20ac":"Euro Sign","\r":"Carriage Return","\ufb33":"Hebrew Letter Dalet With Dagesh","1":"One","\ud83d\ude00":"Emoji: Grinning Face","\u0080":"Control","\u00f6":"Latin Small Letter O With Diaeresis"}"#;
        let names: Vec<String> = {
            let out = jcs(input).unwrap();
            let value: serde_json::Map<String, Value> = serde_json::from_str(&out).unwrap();
            // serde_json's map is sorted by bytes; read the order from the text instead.
            let mut order: Vec<(usize, String)> = value
                .keys()
                .map(|k| {
                    (
                        out.find(&serde_json::to_string(k).unwrap()).unwrap(),
                        k.clone(),
                    )
                })
                .collect();
            order.sort();
            order.into_iter().map(|(_, k)| k).collect()
        };
        assert_eq!(
            names,
            [
                "\r",
                "1",
                "\u{80}",
                "\u{f6}",
                "\u{20ac}",
                "\u{1f600}",
                "\u{fb33}"
            ]
            .map(String::from)
        );
    }

    #[test]
    fn numbers_follow_ecmascript() {
        for (input, output) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (4.5, "4.5"),
            (-1.5, "-1.5"),
            (5e-324, "5e-324"),
            (-5e-324, "-5e-324"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (123.456, "123.456"),
            (f64::MAX, "1.7976931348623157e+308"),
            // RFC 8785 Appendix B: 0x44b52d02c7e14af5, 0x44b52d02c7e14af6, 0x4430000000000000.
            (
                f64::from_bits(0x44b5_2d02_c7e1_4af5),
                "9.999999999999997e+22",
            ),
            (f64::from_bits(0x44b5_2d02_c7e1_4af6), "1e+23"),
            (
                f64::from_bits(0x4430_0000_0000_0000),
                "295147905179352830000",
            ),
            (9007199254740991.0, "9007199254740991"),
        ] {
            assert_eq!(ecmascript(input).unwrap(), output, "{input:e}");
        }
        // Integers parsed from text take the integer path.
        assert_eq!(jcs("9007199254740991").unwrap(), "9007199254740991");
        assert_eq!(jcs("-9007199254740991").unwrap(), "-9007199254740991");
        assert_eq!(jcs("1.0").unwrap(), "1");
        assert_eq!(jcs("4.50").unwrap(), "4.5");
    }

    #[test]
    fn integers_outside_the_exact_range_are_refused() {
        for input in [
            "9007199254740992",
            "-9007199254740992",
            "18446744073709551615",
        ] {
            assert!(jcs(input).is_err(), "{input}");
        }
        assert!(ecmascript(f64::NAN).is_err());
        assert!(ecmascript(f64::INFINITY).is_err());
    }

    #[test]
    fn strings_escape_only_what_rfc8785_escapes() {
        assert_eq!(
            jcs(r#""\b\t\n\f\r\u001f\u0000/\u007f\"\\é""#).unwrap(),
            "\"\\b\\t\\n\\f\\r\\u001f\\u0000/\u{7f}\\\"\\\\é\""
        );
    }

    #[test]
    fn repeated_member_names_are_refused() {
        for text in [
            r#"{"a":1,"a":1}"#,
            r#"{"a":{"b":1,"b":2}}"#,
            r#"[{"a":[{"c":1,"c":1}]}]"#,
            r#"{"a":1,"\u0061":2}"#,
        ] {
            assert!(reject_duplicate_names(text.as_bytes()).is_err(), "{text}");
        }
        for text in [
            r#"{"a":1,"b":{"a":1}}"#,
            r#"[{"a":1},{"a":2}]"#,
            r#"{"a":[1,"x",null,true,-2.5]}"#,
        ] {
            reject_duplicate_names(text.as_bytes()).unwrap();
        }
        assert!(reject_duplicate_names(b"{} {}").is_err());
    }
}
