//! JSON with the semantics of Python's `json` module, which wrote and reads
//! every document the daemon serves and saves.
//!
//! * Parsing follows `json.loads` (strict mode): whitespace is ` \t\n\r`,
//!   integers are unbounded, a number with a fraction or an exponent is a
//!   float, `NaN`/`Infinity`/`-Infinity` are accepted, control characters
//!   inside strings are rejected and a repeated key keeps its first position
//!   with the last value.
//! * Encoding follows `json.dump` with its defaults: `ensure_ascii` (every
//!   character outside `' '..='~'` is a `\uXXXX` escape, astral characters a
//!   surrogate pair), `", "` and `": "` separators, floats as `repr`.

use std::fmt::Write as _;

use crate::bigint::BigInt;
use crate::pystr::float_repr;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Insertion-ordered, keys unique (see [`object_insert`]).
    Object(Vec<(String, Json)>),
}

/// Insert like a Python dict: an existing key keeps its position.
pub fn object_insert(map: &mut Vec<(String, Json)>, key: String, value: Json) {
    match map.iter_mut().find(|(existing, _)| *existing == key) {
        Some(slot) => slot.1 = value,
        None => map.push((key, value)),
    }
}

pub fn object_get<'a>(map: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

impl Json {
    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Int(i) => !i.is_zero(),
            Json::Float(f) => *f != 0.0,
            Json::Str(s) => !s.is_empty(),
            Json::Array(a) => !a.is_empty(),
            Json::Object(o) => !o.is_empty(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => object_get(map, key),
            _ => None,
        }
    }

    pub fn dump(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Json::Float(f) => dump_float(*f, out),
            Json::Str(s) => dump_str(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.dump(out);
                }
                out.push(']');
            }
            Json::Object(map) => {
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    dump_str(key, out);
                    out.push_str(": ");
                    value.dump(out);
                }
                out.push('}');
            }
        }
    }

    pub fn dumps(&self) -> String {
        let mut out = String::new();
        self.dump(&mut out);
        out
    }
}

/// A float as `json` encodes it (`repr`, with the JavaScript names for the
/// non-finite values). Dict keys that are floats use the same text.
pub fn float_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        float_repr(value)
    }
}

fn dump_float(value: f64, out: &mut String) {
    out.push_str(&float_text(value));
}

pub fn dump_str(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let code = c as u32;
                if code < 0x10000 {
                    let _ = write!(out, "\\u{code:04x}");
                } else {
                    let v = code - 0x10000;
                    let _ = write!(
                        out,
                        "\\u{:04x}\\u{:04x}",
                        0xd800 | (v >> 10),
                        0xdc00 | (v & 0x3ff)
                    );
                }
            }
        }
    }
    out.push('"');
}

#[derive(Debug)]
pub struct ParseError;

/// Nesting deeper than this is rejected (Python raises `RecursionError`).
const MAX_DEPTH: usize = 2000;

/// `json.loads(text)`.
pub fn parse(text: &str) -> Result<Json, ParseError> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(ParseError);
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn literal(&mut self, word: &str) -> bool {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError);
        }
        match self.peek().ok_or(ParseError)? {
            b'"' => Ok(Json::Str(self.string()?)),
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'n' if self.literal("null") => Ok(Json::Null),
            b't' if self.literal("true") => Ok(Json::Bool(true)),
            b'f' if self.literal("false") => Ok(Json::Bool(false)),
            b'N' if self.literal("NaN") => Ok(Json::Float(f64::NAN)),
            b'I' if self.literal("Infinity") => Ok(Json::Float(f64::INFINITY)),
            b'-' if self.literal("-Infinity") => Ok(Json::Float(f64::NEG_INFINITY)),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(ParseError),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'1'..=b'9') => {
                self.digits();
            }
            Some(b'0') => self.pos += 1,
            _ => return Err(ParseError),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.bytes.get(self.pos + 1), Some(b'0'..=b'9')) {
            self.pos += 1;
            self.digits();
            is_float = true;
        }
        if let Some(b'e' | b'E') = self.peek() {
            let mark = self.pos;
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                // Not an exponent after all: the number ends before the `e`.
                self.pos = mark;
            } else {
                is_float = true;
            }
        }
        let text = &self.text[start..self.pos];
        if is_float {
            text.parse::<f64>().map(Json::Float).map_err(|_| ParseError)
        } else {
            let (negative, digits) = match text.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, text),
            };
            if digits.len() > 4300 {
                return Err(ParseError);
            }
            Ok(Json::Int(BigInt::from_digits(negative, digits.as_bytes())))
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let chunk = self.bytes.get(at..at + 4)?;
        let text = std::str::from_utf8(chunk).ok()?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(text, 16).ok()
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[start..self.pos]);
            match self.peek().ok_or(ParseError)? {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let escape = *self.bytes.get(self.pos + 1).ok_or(ParseError)?;
                    self.pos += 2;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4(self.pos).ok_or(ParseError)?;
                            self.pos += 4;
                            if (0xd800..0xdc00).contains(&unit)
                                && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
                            {
                                if let Some(low @ 0xdc00..=0xdfff) = self.hex4(self.pos + 2) {
                                    self.pos += 6;
                                    let code = 0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00);
                                    out.push(char::from_u32(code).ok_or(ParseError)?);
                                    continue;
                                }
                            }
                            // A lone surrogate cannot live in a Rust string.
                            out.push(char::from_u32(unit).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(ParseError),
                    }
                }
                _ => return Err(ParseError), // control character
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek().ok_or(ParseError)? {
                b']' => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                b',' => {
                    self.pos += 1;
                    self.skip_ws();
                }
                _ => return Err(ParseError),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.pos += 1;
        let mut map = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(map));
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(ParseError);
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(ParseError);
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            object_insert(&mut map, key, value);
            self.skip_ws();
            match self.peek().ok_or(ParseError)? {
                b'}' => {
                    self.pos += 1;
                    return Ok(Json::Object(map));
                }
                b',' => {
                    self.pos += 1;
                    self.skip_ws();
                }
                _ => return Err(ParseError),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(text: &str) -> String {
        parse(text).unwrap().dumps()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            roundtrip(r#"[1e3, 5e-324, -2.5E-2, -0.0, 1.578364e9, -0, 1180591620717411303424]"#),
            "[1000.0, 5e-324, -0.025, -0.0, 1578364000.0, 0, 1180591620717411303424]"
        );
    }

    #[test]
    fn strings_escape_like_ensure_ascii() {
        assert_eq!(
            roundtrip("[\"caf\u{e9} \u{1f389} \\\"q\\\" \\\\ / \\u007f\\b\\f\\n\\r\\t\\u0001\"]"),
            "[\"caf\\u00e9 \\ud83c\\udf89 \\\"q\\\" \\\\ / \\u007f\\b\\f\\n\\r\\t\\u0001\"]"
        );
        assert_eq!(roundtrip(r#""😀""#), r#""\ud83d\ude00""#);
    }

    #[test]
    fn objects_keep_first_position() {
        assert_eq!(
            roundtrip(r#"{"a": 1, "b": 2, "a": 3}"#),
            r#"{"a": 3, "b": 2}"#
        );
        assert_eq!(roundtrip(" { } "), "{}");
    }

    #[test]
    fn rejects_like_python() {
        for text in [
            "[{not valid json,,,",
            "[1,]",
            "01",
            "1.",
            "\u{feff}[]",
            "[\"a\u{1}\"]",
            "{\"a\" 1}",
            "",
            "[] x",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        assert!(matches!(parse("1e"), Err(_)));
        assert!(matches!(parse("NaN"), Ok(Json::Float(f)) if f.is_nan()));
    }
}
