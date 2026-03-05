///! Homemade JSON parser — recursive descent, zero dependencies.
///! Handles objects, arrays, strings, numbers, booleans, null.
///! Built from scratch to parse Docker Registry API responses.

use std::collections::HashMap;
use std::fmt;

// ─── JSON Value Type ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(HashMap<String, JsonValue>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        match self {
            JsonValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HashMap<String, JsonValue>> {
        match self {
            JsonValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Get a value from an object by key.
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Get an array element by index.
    pub fn index(&self, i: usize) -> Option<&JsonValue> {
        match self {
            JsonValue::Array(arr) => arr.get(i),
            _ => None,
        }
    }
}

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonValue::Null => write!(f, "null"),
            JsonValue::Bool(b) => write!(f, "{}", b),
            JsonValue::Number(n) => {
                if *n == (*n as i64) as f64 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            JsonValue::Str(s) => write!(f, "\"{}\"", escape_json_string(s)),
            JsonValue::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 { write!(f, ",")?; }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            JsonValue::Object(map) => {
                write!(f, "{{")?;
                let mut first = true;
                for (k, v) in map {
                    if !first { write!(f, ",")?; }
                    write!(f, "\"{}\":{}", escape_json_string(k), v)?;
                    first = false;
                }
                write!(f, "}}")
            }
        }
    }
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

// ─── Parser ─────────────────────────────────────────────────────────────────

pub fn parse(input: &str) -> Result<JsonValue, String> {
    let mut parser = Parser::new(input);
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.pos < parser.input.len() {
        return Err(format!("trailing data at position {}", parser.pos));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<u8> {
        if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        match self.advance() {
            Some(b) if b == expected => Ok(()),
            Some(b) => Err(format!(
                "expected '{}' but got '{}' at position {}",
                expected as char, b as char, self.pos - 1
            )),
            None => Err(format!("unexpected end of input, expected '{}'", expected as char)),
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'"') => self.parse_string().map(JsonValue::Str),
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!("unexpected character '{}' at position {}", b as char, self.pos)),
            None => Err("unexpected end of input".into()),
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s = String::new();
        loop {
            match self.advance() {
                Some(b'"') => return Ok(s),
                Some(b'\\') => {
                    match self.advance() {
                        Some(b'"') => s.push('"'),
                        Some(b'\\') => s.push('\\'),
                        Some(b'/') => s.push('/'),
                        Some(b'n') => s.push('\n'),
                        Some(b'r') => s.push('\r'),
                        Some(b't') => s.push('\t'),
                        Some(b'b') => s.push('\x08'),
                        Some(b'f') => s.push('\x0c'),
                        Some(b'u') => {
                            let hex = self.parse_hex4()?;
                            if let Some(c) = char::from_u32(hex) {
                                s.push(c);
                            } else if hex >= 0xD800 && hex <= 0xDBFF {
                                // High surrogate — expect \uXXXX low surrogate
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let low = self.parse_hex4()?;
                                if low >= 0xDC00 && low <= 0xDFFF {
                                    let cp = 0x10000 + ((hex - 0xD800) << 10) + (low - 0xDC00);
                                    if let Some(c) = char::from_u32(cp) {
                                        s.push(c);
                                    }
                                }
                            }
                        }
                        Some(b) => return Err(format!("invalid escape '\\{}'", b as char)),
                        None => return Err("unexpected end in string escape".into()),
                    }
                }
                Some(b) => s.push(b as char),
                None => return Err("unterminated string".into()),
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut val: u32 = 0;
        for _ in 0..4 {
            let b = self.advance().ok_or("unexpected end in unicode escape")?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return Err(format!("invalid hex digit '{}'", b as char)),
            };
            val = (val << 4) | digit;
        }
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;

        // Optional minus
        if self.peek() == Some(b'-') {
            self.advance();
        }

        // Integer part
        match self.peek() {
            Some(b'0') => { self.advance(); }
            Some(b'1'..=b'9') => {
                self.advance();
                while let Some(b'0'..=b'9') = self.peek() {
                    self.advance();
                }
            }
            _ => return Err(format!("invalid number at position {}", self.pos)),
        }

        // Fractional part
        if self.peek() == Some(b'.') {
            self.advance();
            let frac_start = self.pos;
            while let Some(b'0'..=b'9') = self.peek() {
                self.advance();
            }
            if self.pos == frac_start {
                return Err("expected digit after decimal point".into());
            }
        }

        // Exponent
        if self.peek() == Some(b'e') || self.peek() == Some(b'E') {
            self.advance();
            if self.peek() == Some(b'+') || self.peek() == Some(b'-') {
                self.advance();
            }
            let exp_start = self.pos;
            while let Some(b'0'..=b'9') = self.peek() {
                self.advance();
            }
            if self.pos == exp_start {
                return Err("expected digit in exponent".into());
            }
        }

        let num_str = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| "invalid utf8 in number")?;

        let n: f64 = num_str.parse()
            .map_err(|e| format!("invalid number '{}': {}", num_str, e))?;

        Ok(JsonValue::Number(n))
    }

    fn parse_bool(&mut self) -> Result<JsonValue, String> {
        if self.input[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(JsonValue::Bool(true))
        } else if self.input[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(JsonValue::Bool(false))
        } else {
            Err(format!("unexpected token at position {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<JsonValue, String> {
        if self.input[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(JsonValue::Null)
        } else {
            Err(format!("unexpected token at position {}", self.pos))
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[')?;
        self.skip_whitespace();

        let mut arr = Vec::new();
        if self.peek() == Some(b']') {
            self.advance();
            return Ok(JsonValue::Array(arr));
        }

        loop {
            let val = self.parse_value()?;
            arr.push(val);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => { self.advance(); }
                Some(b']') => { self.advance(); return Ok(JsonValue::Array(arr)); }
                _ => return Err(format!("expected ',' or ']' at position {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        self.skip_whitespace();

        let mut map = HashMap::new();
        if self.peek() == Some(b'}') {
            self.advance();
            return Ok(JsonValue::Object(map));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => { self.advance(); }
                Some(b'}') => { self.advance(); return Ok(JsonValue::Object(map)); }
                _ => return Err(format!("expected ',' or '}}' at position {}", self.pos)),
            }
        }
    }
}

// ─── Serialization helpers ──────────────────────────────────────────────────

/// Build a JSON object from key-value pairs.
pub fn object(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    let mut map = HashMap::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    JsonValue::Object(map)
}

/// Build a JSON array.
pub fn array(items: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(items)
}

/// Build a JSON string.
pub fn string(s: &str) -> JsonValue {
    JsonValue::Str(s.to_string())
}

/// Build a JSON number.
pub fn number(n: f64) -> JsonValue {
    JsonValue::Number(n)
}
