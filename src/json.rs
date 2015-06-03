//! A small, dependency-free JSON implementation.
//!
//! `recallweave` deliberately avoids third-party crates (see `docs/MEMORY.md`),
//! so we ship a compact hand-written JSON parser and serializer. It supports the
//! subset of JSON needed by the memory format: objects, arrays, strings, numbers
//! (integer and float), booleans, and null. It is not a general-purpose library
//! but it is correct for the data this engine produces and consumes, and it round
//! trips every value we emit.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// All numbers are stored as f64. Integer-valued numbers are serialized
    /// without a fractional part so IDs and counters stay stable.
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Objects use a BTreeMap so serialization is deterministic (sorted keys).
    /// Determinism matters: integrity hashes are computed over serialized bytes.
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as u64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Obj(o) => Some(o),
            _ => None,
        }
    }

    /// Convenience accessor for a field of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// Serialize to a compact single-line string (no whitespace).
    /// Used for the append-only log so each record is exactly one line.
    pub fn to_compact(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    /// Serialize to a human-readable, indented string. Used for exported packs.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }

    fn write_compact(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Num(n) => out.push_str(&format_number(*n)),
            Json::Str(s) => write_json_string(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_compact(out);
                }
                out.push(']');
            }
            Json::Obj(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(k, out);
                    out.push(':');
                    v.write_compact(out);
                }
                out.push('}');
            }
        }
    }

    fn write_pretty(&self, out: &mut String, depth: usize) {
        let pad = "  ".repeat(depth);
        let pad_inner = "  ".repeat(depth + 1);
        match self {
            Json::Arr(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad_inner);
                    item.write_pretty(out, depth + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push(']');
            }
            Json::Obj(map) if !map.is_empty() => {
                out.push_str("{\n");
                let len = map.len();
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(&pad_inner);
                    write_json_string(k, out);
                    out.push_str(": ");
                    v.write_pretty(out, depth + 1);
                    if i + 1 < len {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push('}');
            }
            // Empty containers and scalars: fall back to compact form.
            _ => self.write_compact(out),
        }
    }

    /// Parse a JSON string into a `Json` value.
    pub fn parse(input: &str) -> Result<Json, String> {
        let mut parser = Parser {
            chars: input.chars().collect(),
            pos: 0,
        };
        parser.skip_ws();
        let value = parser.parse_value()?;
        parser.skip_ws();
        if parser.pos != parser.chars.len() {
            return Err(format!("trailing characters at position {}", parser.pos));
        }
        Ok(value)
    }
}

/// Format an f64 the way we want it in JSON: integers without a decimal point,
/// finite floats with the shortest round-tripping representation Rust gives us.
fn format_number(n: f64) -> String {
    if n.is_finite() {
        if n.fract() == 0.0 && n.abs() < 1e15 {
            format!("{}", n as i64)
        } else {
            let mut s = String::new();
            let _ = write!(s, "{}", n);
            s
        }
    } else {
        // JSON has no representation for NaN/Infinity; emit null defensively.
        "null".to_string()
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Json::Str(self.parse_string()?)),
            Some('t') | Some('f') => self.parse_bool(),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(format!("unexpected character '{}' at {}", c, self.pos)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.next(); // consume '{'
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.next();
            return Ok(Json::Obj(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(format!("expected string key at {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.next() != Some(':') {
                return Err(format!("expected ':' at {}", self.pos));
            }
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.next() {
                Some(',') => continue,
                Some('}') => break,
                other => return Err(format!("expected ',' or '}}', got {:?}", other)),
            }
        }
        Ok(Json::Obj(map))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.next(); // consume '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.next();
            return Ok(Json::Arr(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.next() {
                Some(',') => continue,
                Some(']') => break,
                other => return Err(format!("expected ',' or ']', got {:?}", other)),
            }
        }
        Ok(Json::Arr(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.next(); // consume opening quote
        let mut s = String::new();
        loop {
            match self.next() {
                Some('"') => break,
                Some('\\') => match self.next() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('b') => s.push('\u{08}'),
                    Some('f') => s.push('\u{0C}'),
                    Some('u') => {
                        let cp = self.parse_hex4()?;
                        // Handle UTF-16 surrogate pairs.
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if self.next() != Some('\\') || self.next() != Some('u') {
                                return Err("invalid surrogate pair".to_string());
                            }
                            let low = self.parse_hex4()?;
                            let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                            match char::from_u32(combined) {
                                Some(c) => s.push(c),
                                None => return Err("invalid code point".to_string()),
                            }
                        } else {
                            match char::from_u32(cp) {
                                Some(c) => s.push(c),
                                None => return Err("invalid code point".to_string()),
                            }
                        }
                    }
                    other => return Err(format!("invalid escape {:?}", other)),
                },
                Some(c) => s.push(c),
                None => return Err("unterminated string".to_string()),
            }
        }
        Ok(s)
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.next().ok_or("unexpected end in \\u escape")?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("invalid hex digit '{}'", c))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.match_literal("true") {
            Ok(Json::Bool(true))
        } else if self.match_literal("false") {
            Ok(Json::Bool(false))
        } else {
            Err(format!("invalid literal at {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.match_literal("null") {
            Ok(Json::Null)
        } else {
            Err(format!("invalid literal at {}", self.pos))
        }
    }

    fn match_literal(&mut self, lit: &str) -> bool {
        let end = self.pos + lit.len();
        if end <= self.chars.len() {
            let slice: String = self.chars[self.pos..end].iter().collect();
            if slice == lit {
                self.pos = end;
                return true;
            }
        }
        false
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.next();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.next();
            } else {
                break;
            }
        }
        if self.peek() == Some('.') {
            self.next();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.next();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.next();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.next();
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.next();
                } else {
                    break;
                }
            }
        }
        let slice: String = self.chars[start..self.pos].iter().collect();
        slice
            .parse::<f64>()
            .map(Json::Num)
            .map_err(|e| format!("invalid number '{}': {}", slice, e))
    }
}

/// Ergonomic builder helpers used throughout the crate.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    let mut map = BTreeMap::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    Json::Obj(map)
}

pub fn arr(items: Vec<Json>) -> Json {
    Json::Arr(items)
}

pub fn s(value: &str) -> Json {
    Json::Str(value.to_string())
}

pub fn num(value: f64) -> Json {
    Json::Num(value)
}
