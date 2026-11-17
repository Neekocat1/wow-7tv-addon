//! Read WoW SavedVariables files and write Lua data files.
//!
//! SavedVariables are plain Lua assignments that WoW writes on /reload and logout:
//!
//! ```text
//! SevenTVEmotesDB = {
//!     ["queue"] = {
//!         {
//!             ["kind"] = "add",
//!             ["ref"] = "https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52",
//!         }, -- [1]
//!     },
//! }
//! ```
//!
//! Only literal data appears there, so a small parser is enough. The input is bytes (Lua strings
//! are bytes); string values are decoded as UTF-8, falling back to Latin-1.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Int(i64),
    Str(String),
}

impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Key::Str(s.to_string())
    }
}
impl From<String> for Key {
    fn from(s: String) -> Self {
        Key::Str(s)
    }
}
impl From<i64> for Key {
    fn from(i: i64) -> Self {
        Key::Int(i)
    }
}

/// A Lua value. Tables whose keys are exactly 1..n come back as `List`, others as `Table`
/// (keys in file order), like the Python helper's list/dict split.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Table(Vec<(Key, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(kv) => kv
                .iter()
                .find(|(k, _)| matches!(k, Key::Str(s) if s == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_table(&self) -> bool {
        matches!(self, Value::Table(_) | Value::List(_))
    }
    /// Python truthiness, as the helper used it (`if db.get("x")`).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Table(t) => !t.is_empty(),
        }
    }
    /// Text the way Python's str() would show it (used for request fields like `ref`).
    pub fn to_plain_string(&self) -> String {
        match self {
            Value::Nil => String::new(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => py_float(*f),
            Value::Str(s) => s.clone(),
            other => lua_value(other, ""),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuaParseError(pub String);

impl fmt::Display for LuaParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for LuaParseError {}

type R<T> = Result<T, LuaParseError>;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'a> Parser<'a> {
    fn error<T>(&self, msg: &str) -> R<T> {
        let line = self.s[..self.i.min(self.s.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count()
            + 1;
        Err(LuaParseError(format!("{msg} at line {line}")))
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.s.get(self.i + off).copied()
    }

    fn starts_with(&self, at: usize, pat: &[u8]) -> bool {
        self.s.get(at..at + pat.len()) == Some(pat)
    }

    fn find(&self, from: usize, pat: &[u8]) -> Option<usize> {
        if pat.is_empty() || from > self.s.len() {
            return None;
        }
        self.s[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| p + from)
    }

    /// `[` followed by zero or more `=` and another `[` at `at`: returns the number of `=`.
    fn long_bracket(&self, at: usize) -> Option<usize> {
        if self.s.get(at) != Some(&b'[') {
            return None;
        }
        let mut j = at + 1;
        while self.s.get(j) == Some(&b'=') {
            j += 1;
        }
        (self.s.get(j) == Some(&b'[')).then_some(j - at - 1)
    }

    fn skip(&mut self) {
        let n = self.s.len();
        while self.i < n {
            let c = self.s[self.i];
            if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b',' | b';') {
                self.i += 1;
            } else if self.starts_with(self.i, b"--") {
                if let Some(eq) = self.long_bracket(self.i + 2) {
                    let mut close = vec![b']'];
                    close.extend(std::iter::repeat_n(b'=', eq));
                    close.push(b']');
                    self.i = match self.find(self.i, &close) {
                        Some(end) => end + eq + 2,
                        None => n,
                    };
                } else {
                    self.i = match self.find(self.i, b"\n") {
                        Some(nl) => nl + 1,
                        None => n,
                    };
                }
            } else {
                break;
            }
        }
    }

    fn ident_at(&self, at: usize) -> Option<usize> {
        if at < self.s.len() && is_ident_start(self.s[at]) {
            let mut j = at + 1;
            while j < self.s.len() && is_ident_char(self.s[j]) {
                j += 1;
            }
            Some(j)
        } else {
            None
        }
    }

    fn parse_file(&mut self) -> R<Vec<(String, Value)>> {
        let mut out: Vec<(String, Value)> = Vec::new();
        loop {
            self.skip();
            if self.i >= self.s.len() {
                return Ok(out);
            }
            let Some(end) = self.ident_at(self.i) else {
                return self.error("expected a variable name");
            };
            let name = String::from_utf8_lossy(&self.s[self.i..end]).into_owned();
            self.i = end;
            self.skip();
            if self.peek(0) != Some(b'=') {
                return self.error("expected '='");
            }
            self.i += 1;
            let v = self.value()?;
            if let Some(slot) = out.iter_mut().find(|(k, _)| *k == name) {
                slot.1 = v;
            } else {
                out.push((name, v));
            }
        }
    }

    fn value(&mut self) -> R<Value> {
        self.skip();
        let Some(c) = self.peek(0) else {
            return self.error("unexpected end of file");
        };
        if c == b'{' {
            return self.table();
        }
        if c == b'"' || c == b'\'' {
            return Ok(Value::Str(self.string()?));
        }
        if c == b'[' && self.long_bracket(self.i).is_some() {
            return Ok(Value::Str(self.long_string()?));
        }
        for (word, val) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("nil", Value::Nil),
        ] {
            let w = word.as_bytes();
            if self.starts_with(self.i, w)
                && !self
                    .s
                    .get(self.i + w.len())
                    .is_some_and(|&c| is_ident_char(c))
            {
                self.i += w.len();
                return Ok(val);
            }
        }
        if let Some(v) = self.number() {
            return Ok(v);
        }
        self.error(&format!("unexpected character {:?}", c as char))
    }

    /// `-?(0x[0-9a-f]+ | (\d+\.?\d* | \.\d+)([eE][+-]?\d+)? | inf | nan)`
    fn number(&mut self) -> Option<Value> {
        let s = self.s;
        let start = self.i;
        let mut j = start;
        let neg = s.get(j) == Some(&b'-');
        if neg {
            j += 1;
        }
        let digits = |mut k: usize| {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            k
        };
        if s.get(j) == Some(&b'0') && matches!(s.get(j + 1), Some(b'x' | b'X')) {
            let mut k = j + 2;
            while k < s.len() && s[k].is_ascii_hexdigit() {
                k += 1;
            }
            if k > j + 2 {
                let text = std::str::from_utf8(&s[j + 2..k]).ok()?;
                let v = i64::from_str_radix(text, 16).ok()?;
                self.i = k;
                return Some(Value::Int(if neg { -v } else { v }));
            }
        }
        for (word, val) in [(&b"inf"[..], f64::INFINITY), (&b"nan"[..], f64::NAN)] {
            if self.starts_with(j, word) {
                self.i = j + 3;
                return Some(Value::Float(if neg { -val } else { val }));
            }
        }
        let mut k;
        let mut is_float = false;
        if s.get(j).is_some_and(|c| c.is_ascii_digit()) {
            k = digits(j);
            if s.get(k) == Some(&b'.') {
                is_float = true;
                k = digits(k + 1);
            }
        } else if s.get(j) == Some(&b'.') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            k = digits(j + 1);
        } else {
            return None;
        }
        if matches!(s.get(k), Some(b'e' | b'E')) {
            let mut e = k + 1;
            if matches!(s.get(e), Some(b'+' | b'-')) {
                e += 1;
            }
            if s.get(e).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                k = digits(e);
            }
        }
        let text = std::str::from_utf8(&s[start..k]).ok()?;
        self.i = k;
        if is_float {
            text.parse::<f64>().ok().map(Value::Float)
        } else {
            match text.parse::<i64>() {
                Ok(v) => Some(Value::Int(v)),
                Err(_) => text.parse::<f64>().ok().map(Value::Float),
            }
        }
    }

    fn string(&mut self) -> R<String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(c) = self.peek(0) else {
                return self.error("unterminated string");
            };
            if c == q {
                self.i += 1;
                break;
            }
            if c == b'\\' {
                self.i += 1;
                let Some(e) = self.peek(0) else {
                    return self.error("unterminated string");
                };
                if e.is_ascii_digit() {
                    let mut n: u32 = 0;
                    let mut len = 0;
                    while len < 3 && self.peek(0).is_some_and(|d| d.is_ascii_digit()) {
                        n = n * 10 + (self.s[self.i] - b'0') as u32;
                        self.i += 1;
                        len += 1;
                    }
                    if n < 256 {
                        out.push(n as u8);
                    } else {
                        // Python would produce chr(n) here; keep it as UTF-8.
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(
                            char::from_u32(n)
                                .unwrap_or('?')
                                .encode_utf8(&mut buf)
                                .as_bytes(),
                        );
                    }
                    continue;
                }
                let r = match e {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'v' => 0x0b,
                    other => other, // \\ \" \' \<newline> and anything unknown
                };
                out.push(r);
                self.i += 1;
                continue;
            }
            out.push(c);
            self.i += 1;
        }
        Ok(decode(out))
    }

    fn long_string(&mut self) -> R<String> {
        let eq = self.long_bracket(self.i).unwrap_or(0);
        let mut close = vec![b']'];
        close.extend(std::iter::repeat_n(b'=', eq));
        close.push(b']');
        let start = self.i + eq + 2;
        let Some(end) = self.find(start, &close) else {
            return self.error("unterminated long string");
        };
        self.i = end + close.len();
        let mut text = &self.s[start..end];
        if text.first() == Some(&b'\n') {
            text = &text[1..];
        }
        Ok(decode(text.to_vec()))
    }

    fn table(&mut self) -> R<Value> {
        self.i += 1; // {
        let mut arr: Vec<Value> = Vec::new();
        let mut hsh: Vec<(Key, Value)> = Vec::new();
        fn put(h: &mut Vec<(Key, Value)>, k: Key, v: Value) {
            if let Some(slot) = h.iter_mut().find(|(x, _)| *x == k) {
                slot.1 = v;
            } else {
                h.push((k, v));
            }
        }
        loop {
            self.skip();
            let Some(c) = self.peek(0) else {
                return self.error("unterminated table");
            };
            if c == b'}' {
                self.i += 1;
                break;
            }
            if c == b'[' && self.long_bracket(self.i).is_none() {
                self.i += 1;
                let key = self.value()?;
                self.skip();
                if self.peek(0) != Some(b']') {
                    return self.error("expected ']'");
                }
                self.i += 1;
                self.skip();
                if self.peek(0) != Some(b'=') {
                    return self.error("expected '='");
                }
                self.i += 1;
                let v = self.value()?;
                let k = match key {
                    Value::Int(i) => Key::Int(i),
                    Value::Float(f) if f.fract() == 0.0 && f.abs() < 9e15 => Key::Int(f as i64),
                    Value::Str(s) => Key::Str(s),
                    other => Key::Str(other.to_plain_string()),
                };
                put(&mut hsh, k, v);
                continue;
            }
            if let Some(end) = self.ident_at(self.i) {
                let word = &self.s[self.i..end];
                if word != b"true" && word != b"false" && word != b"nil" {
                    let mut j = end;
                    while j < self.s.len() && (self.s[j] == b' ' || self.s[j] == b'\t') {
                        j += 1;
                    }
                    if self.s.get(j) == Some(&b'=') && self.s.get(j + 1) != Some(&b'=') {
                        let name = String::from_utf8_lossy(word).into_owned();
                        self.i = j + 1;
                        let v = self.value()?;
                        put(&mut hsh, Key::Str(name), v);
                        continue;
                    }
                }
            }
            arr.push(self.value()?);
        }
        if hsh.is_empty() {
            return Ok(Value::List(arr));
        }
        for (n, v) in arr.into_iter().enumerate() {
            put(&mut hsh, Key::Int(n as i64 + 1), v);
        }
        let mut ints: Vec<i64> = Vec::new();
        for (k, _) in &hsh {
            match k {
                Key::Int(i) => ints.push(*i),
                Key::Str(_) => return Ok(Value::Table(hsh)),
            }
        }
        ints.sort_unstable();
        if ints.iter().enumerate().all(|(n, &k)| k == n as i64 + 1) {
            let mut items: Vec<(i64, Value)> = hsh
                .into_iter()
                .map(|(k, v)| (if let Key::Int(i) = k { i } else { 0 }, v))
                .collect();
            items.sort_by_key(|(k, _)| *k);
            return Ok(Value::List(items.into_iter().map(|(_, v)| v).collect()));
        }
        Ok(Value::Table(hsh))
    }
}

fn decode(raw: Vec<u8>) -> String {
    match String::from_utf8(raw) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(), // Latin-1
    }
}

/// Parse a SavedVariables file into `[(global_name, value)]`.
pub fn parse(text: &[u8]) -> Result<Vec<(String, Value)>, LuaParseError> {
    Parser { s: text, i: 0 }.parse_file()
}

/// Convenience: the value of one global (Nil if absent).
pub fn global<'v>(file: &'v [(String, Value)], name: &str) -> Option<&'v Value> {
    file.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

// --------------------------------------------------------------------------- writing

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn is_ident(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && is_ident_start(b[0]) && b.iter().all(|&c| is_ident_char(c))
}

pub fn lua_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for &b in s.as_bytes() {
        if b == 0x22 || b == 0x5C {
            out.push('\\');
            out.push(b as char);
        } else if (32..127).contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{:03}", b));
        }
    }
    out.push('"');
    out
}

/// Python's repr() of a float (shortest round-trip, exponent outside 1e-4 .. 1e16).
pub fn py_float(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let e = format!("{:e}", v); // shortest digits: "1.5e3", "-1e-5"
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..16).contains(&exp) {
        let s = format!("{}", v);
        if s.contains('.') { s } else { s + ".0" }
    } else {
        format!(
            "{}e{}{:02}",
            mant,
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    }
}

pub fn lua_value(v: &Value, indent: &str) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                py_float(*f)
            } else {
                "0".into()
            }
        }
        Value::Str(s) => lua_str(s),
        Value::List(items) => {
            if items.is_empty() {
                return "{}".into();
            }
            let inner = format!("{indent}  ");
            let parts: Vec<String> = items.iter().map(|x| lua_value(x, &inner)).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Value::Table(kv) => {
            if kv.is_empty() {
                return "{}".into();
            }
            let inner = format!("{indent}  ");
            let mut parts = Vec::with_capacity(kv.len());
            for (k, x) in kv {
                let key = match k {
                    Key::Int(i) => format!("[{i}]"),
                    Key::Str(s) if is_ident(s) && !KEYWORDS.contains(&s.as_str()) => s.clone(),
                    Key::Str(s) => format!("[{}]", lua_str(s)),
                };
                parts.push(format!("{inner}{key} = {},", lua_value(x, &inner)));
            }
            format!("{{\n{}\n{indent}}}", parts.join("\n"))
        }
    }
}

/// Small builder for ordered Lua tables.
#[derive(Default)]
pub struct TableBuilder(Vec<(Key, Value)>);

impl TableBuilder {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn set(mut self, k: impl Into<Key>, v: impl Into<Value>) -> Self {
        self.0.push((k.into(), v.into()));
        self
    }
    pub fn push(&mut self, k: impl Into<Key>, v: impl Into<Value>) {
        self.0.push((k.into(), v.into()));
    }
    pub fn build(self) -> Value {
        Value::Table(self.0)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<usize> for Value {
    fn from(i: usize) -> Self {
        Value::Int(i as i64)
    }
}
impl From<u32> for Value {
    fn from(i: u32) -> Self {
        Value::Int(i as i64)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<Option<String>> for Value {
    fn from(s: Option<String>) -> Self {
        s.map(Value::Str).unwrap_or(Value::Nil)
    }
}
impl From<Vec<String>> for Value {
    fn from(v: Vec<String>) -> Self {
        Value::List(v.into_iter().map(Value::Str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wow_format() {
        let text = concat!(
            "SevenTVEmotesDB = {\n",
            "\t[\"enabled\"] = true,\n",
            "\t[\"size\"] = 28,\n",
            "\t[\"ratio\"] = 1.5,\n",
            "\t[\"disabled\"] = {\n\t},\n",
            "\t[\"queue\"] = {\n",
            "\t\t{\n",
            "\t\t\t[\"id\"] = \"1763164800-0042\",\n",
            "\t\t\t[\"kind\"] = \"add\",\n",
            "\t\t\t[\"ref\"] = \"https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52\",\n",
            "\t\t\t[\"alias\"] = \"caf\\195\\169 \\\"q\\\"\",\n",
            "\t\t}, -- [1]\n",
            "\t},\n",
            "}\n",
            "OtherVar = nil\n",
        );
        let d = parse(text.as_bytes()).unwrap();
        let db = global(&d, "SevenTVEmotesDB").unwrap();
        assert_eq!(db.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(db.get("size"), Some(&Value::Int(28)));
        assert_eq!(db.get("ratio"), Some(&Value::Float(1.5)));
        assert_eq!(db.get("disabled"), Some(&Value::List(vec![])));
        let Value::List(q) = db.get("queue").unwrap() else {
            panic!()
        };
        assert_eq!(q[0].get("kind").unwrap().as_str(), Some("add"));
        assert_eq!(q[0].get("alias").unwrap().as_str(), Some("café \"q\""));
        assert_eq!(global(&d, "OtherVar"), Some(&Value::Nil));
        // round trip
        let again = parse(format!("X = {}", lua_value(db, "")).as_bytes()).unwrap();
        let Value::List(q2) = global(&again, "X").unwrap().get("queue").unwrap() else {
            panic!()
        };
        assert_eq!(q2[0].get("alias").unwrap().as_str(), Some("café \"q\""));
    }

    #[test]
    fn comments_long_strings_and_numbers() {
        let text = b"--[[ block\ncomment ]] A = { [[\nlong]], [==[x]]y]==], 0x1F, -3, .5, 1e3, x = 'it\\'s', [2.0] = 7 }";
        let d = parse(text).unwrap();
        let a = global(&d, "A").unwrap();
        let Value::Table(kv) = a else { panic!("{a:?}") };
        assert_eq!(kv[0], (Key::Str("x".into()), Value::Str("it's".into())));
        assert!(kv.contains(&(Key::Int(1), Value::Str("long".into()))));
        assert!(kv.contains(&(Key::Int(2), Value::Str("x]]y".into()))));
        assert!(kv.contains(&(Key::Int(3), Value::Int(31))));
        assert!(kv.contains(&(Key::Int(5), Value::Float(0.5))));
        assert!(kv.contains(&(Key::Int(6), Value::Float(1000.0))));
    }

    #[test]
    fn sparse_int_keys_stay_a_table_and_dense_become_list() {
        let d = parse(b"A = { [1] = 'a', [3] = 'c' } B = { [2] = 'b', [1] = 'a' }").unwrap();
        assert!(matches!(global(&d, "A"), Some(Value::Table(_))));
        assert_eq!(
            global(&d, "B"),
            Some(&Value::List(vec![
                Value::Str("a".into()),
                Value::Str("b".into())
            ]))
        );
    }

    #[test]
    fn writer_matches_python() {
        let v = TableBuilder::new()
            .set("version", "0.5.1")
            .set("channels", Value::List(vec![]))
            .set(
                "list",
                Value::List(vec![Value::Int(1), Value::Str("é".into())]),
            )
            .set("end", true)
            .set(
                "r1",
                TableBuilder::new()
                    .set("ok", true)
                    .set("name", Value::Nil)
                    .build(),
            )
            .build();
        assert_eq!(
            lua_value(&v, ""),
            "{\n  version = \"0.5.1\",\n  channels = {},\n  list = { 1, \"\\195\\169\" },\n  [\"end\"] = true,\n  r1 = {\n    ok = true,\n    name = nil,\n  },\n}"
        );
        assert_eq!(py_float(1.5), "1.5");
        assert_eq!(py_float(3.0), "3.0");
        assert_eq!(py_float(1e16), "1e+16");
        assert_eq!(py_float(1e-5), "1e-05");
        assert_eq!(py_float(1763164800.25), "1763164800.25");
    }
}
