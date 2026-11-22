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
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde_json::{Number, Value};
use std::collections::HashSet;
use std::fmt;

pub(crate) const MAX_EXACT: u64 = (1 << 53) - 1;

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

/// Deserializes `T` from JSON bytes after rejecting duplicate object member names and
/// refusing positional JSON arrays wherever a Rust struct (JSON object) is expected.
/// Preserves byte-stream line and column positions on errors.
pub(crate) fn from_slice_objects<'de, T: serde::Deserialize<'de>>(input: &'de [u8]) -> Result<T> {
    reject_duplicate_names(input)?;
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = T::deserialize(StrictObject(&mut deserializer))?;
    deserializer.end()?;
    Ok(value)
}

/// Deserializes `T` from an already-parsed `Value`, refusing positional JSON arrays wherever
/// a Rust struct (JSON object) is expected.
pub(crate) fn from_value_objects<T: DeserializeOwned>(value: Value) -> Result<T> {
    Ok(T::deserialize(StrictObject(value))?)
}

/// Strips a trailing ` at line N column M` suffix from a `serde_json` error message so a
/// parent deserializer can wrap it with field context without duplicating the location.
pub(crate) fn strip_json_location(msg: &str) -> &str {
    let Some((prefix, rest)) = msg.rsplit_once(" at line ") else {
        return msg;
    };
    let Some((line, col)) = rest.split_once(" column ") else {
        return msg;
    };
    if !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
        && !col.is_empty()
        && col.bytes().all(|b| b.is_ascii_digit())
    {
        prefix
    } else {
        msg
    }
}

/// Wraps any `Deserializer` so every `deserialize_struct` rejects JSON arrays instead of
/// accepting serde's default positional-sequence representation for structs.
struct StrictObject<D>(D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for StrictObject<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_any(StrictVisitor(visitor))
    }

    fn deserialize_bool<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_bool(visitor)
    }

    fn deserialize_i8<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_i8(visitor)
    }

    fn deserialize_i16<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_i16(visitor)
    }

    fn deserialize_i32<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_i32(visitor)
    }

    fn deserialize_i64<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_i64(visitor)
    }

    fn deserialize_i128<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_i128(visitor)
    }

    fn deserialize_u8<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_u8(visitor)
    }

    fn deserialize_u16<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_u16(visitor)
    }

    fn deserialize_u32<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_u32(visitor)
    }

    fn deserialize_u64<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_u64(visitor)
    }

    fn deserialize_u128<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_u128(visitor)
    }

    fn deserialize_f32<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_f32(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_f64(visitor)
    }

    fn deserialize_char<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_char(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_str(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_string(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_bytes(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_byte_buf(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_option(StrictVisitor(visitor))
    }

    fn deserialize_unit<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_unit(visitor)
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_unit_struct(name, visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0
            .deserialize_newtype_struct(name, StrictVisitor(visitor))
    }

    fn deserialize_seq<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_seq(StrictVisitor(visitor))
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_tuple(len, StrictVisitor(visitor))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0
            .deserialize_tuple_struct(name, len, StrictVisitor(visitor))
    }

    fn deserialize_map<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_map(StrictVisitor(visitor))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_struct(
            name,
            fields,
            StrictStructVisitor {
                name,
                inner: visitor,
            },
        )
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0
            .deserialize_enum(name, variants, StrictVisitor(visitor))
    }

    fn deserialize_identifier<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_identifier(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(
        self,
        visitor: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.0.deserialize_ignored_any(visitor)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

struct StrictStructVisitor<V> {
    name: &'static str,
    inner: V,
}

impl<'de, V: Visitor<'de>> Visitor<'de> for StrictStructVisitor<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a JSON object for {}", self.name)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<Self::Value, A::Error> {
        self.inner.visit_map(StrictMapAccess(map))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _seq: A) -> std::result::Result<Self::Value, A::Error> {
        Err(de::Error::custom(format!(
            "expected a JSON object for {}, not an array",
            self.name
        )))
    }
}

struct StrictVisitor<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for StrictVisitor<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
        self.0.visit_bool(v)
    }

    fn visit_i8<E: de::Error>(self, v: i8) -> std::result::Result<Self::Value, E> {
        self.0.visit_i8(v)
    }

    fn visit_i16<E: de::Error>(self, v: i16) -> std::result::Result<Self::Value, E> {
        self.0.visit_i16(v)
    }

    fn visit_i32<E: de::Error>(self, v: i32) -> std::result::Result<Self::Value, E> {
        self.0.visit_i32(v)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
        self.0.visit_i64(v)
    }

    fn visit_i128<E: de::Error>(self, v: i128) -> std::result::Result<Self::Value, E> {
        self.0.visit_i128(v)
    }

    fn visit_u8<E: de::Error>(self, v: u8) -> std::result::Result<Self::Value, E> {
        self.0.visit_u8(v)
    }

    fn visit_u16<E: de::Error>(self, v: u16) -> std::result::Result<Self::Value, E> {
        self.0.visit_u16(v)
    }

    fn visit_u32<E: de::Error>(self, v: u32) -> std::result::Result<Self::Value, E> {
        self.0.visit_u32(v)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
        self.0.visit_u64(v)
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> std::result::Result<Self::Value, E> {
        self.0.visit_u128(v)
    }

    fn visit_f32<E: de::Error>(self, v: f32) -> std::result::Result<Self::Value, E> {
        self.0.visit_f32(v)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
        self.0.visit_f64(v)
    }

    fn visit_char<E: de::Error>(self, v: char) -> std::result::Result<Self::Value, E> {
        self.0.visit_char(v)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
        self.0.visit_str(v)
    }

    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> std::result::Result<Self::Value, E> {
        self.0.visit_borrowed_str(v)
    }

    fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
        self.0.visit_string(v)
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> std::result::Result<Self::Value, E> {
        self.0.visit_bytes(v)
    }

    fn visit_borrowed_bytes<E: de::Error>(
        self,
        v: &'de [u8],
    ) -> std::result::Result<Self::Value, E> {
        self.0.visit_borrowed_bytes(v)
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> std::result::Result<Self::Value, E> {
        self.0.visit_byte_buf(v)
    }

    fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
        self.0.visit_none()
    }

    fn visit_some<D: Deserializer<'de>>(self, de: D) -> std::result::Result<Self::Value, D::Error> {
        self.0.visit_some(StrictObject(de))
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
        self.0.visit_unit()
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        de: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        self.0.visit_newtype_struct(StrictObject(de))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> std::result::Result<Self::Value, A::Error> {
        self.0.visit_seq(StrictSeqAccess(seq))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<Self::Value, A::Error> {
        self.0.visit_map(StrictMapAccess(map))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> std::result::Result<Self::Value, A::Error> {
        self.0.visit_enum(StrictEnumAccess(data))
    }
}

struct StrictSeed<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for StrictSeed<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(
        self,
        de: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        self.0.deserialize(StrictObject(de))
    }
}

struct StrictMapAccess<A>(A);

impl<'de, A: MapAccess<'de>> MapAccess<'de> for StrictMapAccess<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> std::result::Result<Option<K::Value>, A::Error> {
        self.0.next_key_seed(seed)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> std::result::Result<V::Value, A::Error> {
        self.0.next_value_seed(StrictSeed(seed))
    }
}

struct StrictSeqAccess<A>(A);

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for StrictSeqAccess<A> {
    type Error = A::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> std::result::Result<Option<T::Value>, A::Error> {
        self.0.next_element_seed(StrictSeed(seed))
    }
}

struct StrictEnumAccess<A>(A);

impl<'de, A: EnumAccess<'de>> EnumAccess<'de> for StrictEnumAccess<A> {
    type Error = A::Error;
    type Variant = StrictVariantAccess<A::Variant>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> std::result::Result<(V::Value, Self::Variant), A::Error> {
        let (val, variant) = self.0.variant_seed(StrictSeed(seed))?;
        Ok((val, StrictVariantAccess(variant)))
    }
}

struct StrictVariantAccess<A>(A);

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for StrictVariantAccess<A> {
    type Error = A::Error;

    fn unit_variant(self) -> std::result::Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> std::result::Result<T::Value, A::Error> {
        self.0.newtype_variant_seed(StrictSeed(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> std::result::Result<V::Value, A::Error> {
        self.0.tuple_variant(len, StrictVisitor(visitor))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> std::result::Result<V::Value, A::Error> {
        self.0.struct_variant(
            fields,
            StrictStructVisitor {
                name: "variant",
                inner: visitor,
            },
        )
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

    #[test]
    fn positional_arrays_are_refused_for_structs() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Inner {
            a: u64,
            b: u64,
        }
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Outer {
            inner: Inner,
            list: Vec<Inner>,
            map: std::collections::BTreeMap<String, Inner>,
            opt: Option<Inner>,
        }

        let ok = br#"{"inner":{"a":1,"b":2},"list":[{"a":3,"b":4}],"map":{"k":{"a":5,"b":6}},"opt":{"a":7,"b":8}}"#;
        let parsed: Outer = from_slice_objects(ok).unwrap();
        assert_eq!(parsed.inner, Inner { a: 1, b: 2 });

        for bad in [
            br#"[[{"a":1,"b":2}],[],{},null]"#.as_slice(),
            br#"{"inner":[1,2],"list":[],"map":{},"opt":null}"#.as_slice(),
            br#"{"inner":{"a":1,"b":2},"list":[[3,4]],"map":{},"opt":null}"#.as_slice(),
            br#"{"inner":{"a":1,"b":2},"list":[],"map":{"k":[5,6]},"opt":null}"#.as_slice(),
            br#"{"inner":{"a":1,"b":2},"list":[],"map":{},"opt":[7,8]}"#.as_slice(),
        ] {
            let err = from_slice_objects::<Outer>(bad).unwrap_err().0;
            assert!(err.contains("expected a JSON object"), "{err}");
            assert!(err.contains("line 1"), "{err}");
        }
    }
}
