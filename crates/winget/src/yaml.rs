//! YAML subset used by winget manifests: block mappings and sequences, plain/quoted scalars,
//! `|`/`>` block scalars, simple flow sequences, comments, `---` document start.
//! Anchors, tags, multi-document streams and complex keys are rejected.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A YAML value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Scalar (always kept as text; `~`/`null` become [`Value::Null`]).
    Str(String),
    /// Null.
    Null,
    /// Sequence.
    Seq(Vec<Value>),
    /// Mapping in file order.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// Mapping lookup.
    pub fn get(&self, k: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(n, _)| n == k).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Scalar text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Sequence items.
    pub fn as_seq(&self) -> Option<&[Value]> {
        match self {
            Value::Seq(s) => Some(s),
            _ => None,
        }
    }
}

/// Parse error with 1-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Line number.
    pub line: usize,
    /// What went wrong.
    pub msg: &'static str,
}

struct Line<'a> {
    indent: usize,
    text: &'a str,
    no: usize,
}

fn strip_comment(s: &str) -> &str {
    let (mut sq, mut dq) = (false, false);
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b'#' if !sq && !dq && (i == 0 || b[i - 1] == b' ') => return s[..i].trim_end(),
            _ => {}
        }
    }
    s.trim_end()
}

fn scalar(s: &str, no: usize) -> Result<Value, Error> {
    let s = s.trim();
    if s.starts_with(['&', '*', '!']) {
        return Err(Error { line: no, msg: "anchors/tags unsupported" });
    }
    if let Some(r) = s.strip_prefix('"') {
        let r = r.strip_suffix('"').ok_or(Error { line: no, msg: "unterminated string" })?;
        let mut o = String::new();
        let mut it = r.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                o.push(c);
                continue;
            }
            match it.next() {
                Some('n') => o.push('\n'),
                Some('t') => o.push('\t'),
                Some('"') => o.push('"'),
                Some('\\') => o.push('\\'),
                Some('/') => o.push('/'),
                _ => return Err(Error { line: no, msg: "bad escape" }),
            }
        }
        return Ok(Value::Str(o));
    }
    if let Some(r) = s.strip_prefix('\'') {
        let r = r.strip_suffix('\'').ok_or(Error { line: no, msg: "unterminated string" })?;
        return Ok(Value::Str(r.replace("''", "'")));
    }
    if let Some(r) = s.strip_prefix('[') {
        let r = r.strip_suffix(']').ok_or(Error { line: no, msg: "unterminated flow sequence" })?;
        let mut v = Vec::new();
        if !r.trim().is_empty() {
            for p in r.split(',') {
                v.push(scalar(p, no)?);
            }
        }
        return Ok(Value::Seq(v));
    }
    if s.starts_with('{') {
        return Err(Error { line: no, msg: "flow mappings unsupported" });
    }
    Ok(if s.is_empty() || s == "~" || s == "null" { Value::Null } else { Value::Str(s.to_string()) })
}

/// Splits `key: rest` (key may be quoted); `None` if the line is not a mapping entry.
fn split_key(t: &str) -> Option<(&str, &str)> {
    let b = t.as_bytes();
    let (mut sq, mut dq) = (false, false);
    for i in 0..b.len() {
        match b[i] {
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b':' if !sq && !dq && (i + 1 == b.len() || b[i + 1] == b' ') => {
                return Some((t[..i].trim().trim_matches(|c| c == '"' || c == '\''), t[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

struct P<'a> {
    lines: Vec<Line<'a>>,
    raw: Vec<&'a str>,
    pos: usize,
}

impl<'a> P<'a> {
    fn block(&mut self, indent: usize) -> Result<Value, Error> {
        let l = &self.lines[self.pos];
        if l.text.starts_with("- ") || l.text == "-" {
            self.seq(indent)
        } else if split_key(l.text).is_some() {
            self.map(indent)
        } else {
            let v = scalar(l.text, l.no)?;
            self.pos += 1;
            Ok(v)
        }
    }

    fn seq(&mut self, indent: usize) -> Result<Value, Error> {
        let mut out = Vec::new();
        while self.pos < self.lines.len() && self.lines[self.pos].indent == indent && (self.lines[self.pos].text.starts_with("- ") || self.lines[self.pos].text == "-") {
            let (no, text) = (self.lines[self.pos].no, self.lines[self.pos].text);
            let rest = text[1..].trim_start();
            if rest.is_empty() {
                self.pos += 1;
                if self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
                    let i = self.lines[self.pos].indent;
                    out.push(self.block(i)?);
                } else {
                    out.push(Value::Null);
                }
            } else if split_key(rest).is_some() {
                // "- key: v" opens a mapping whose indent is the column of `key`.
                let col = indent + (text.len() - rest.len());
                self.lines[self.pos] = Line { indent: col, text: rest, no };
                out.push(self.map(col)?);
            } else {
                out.push(scalar(rest, no)?);
                self.pos += 1;
            }
        }
        Ok(Value::Seq(out))
    }

    fn map(&mut self, indent: usize) -> Result<Value, Error> {
        let mut out: Vec<(String, Value)> = Vec::new();
        while self.pos < self.lines.len() && self.lines[self.pos].indent == indent {
            let (no, text) = (self.lines[self.pos].no, self.lines[self.pos].text);
            let Some((k, rest)) = split_key(text) else { return Err(Error { line: no, msg: "expected key: value" }) };
            if out.iter().any(|(n, _)| n == k) {
                return Err(Error { line: no, msg: "duplicate key" });
            }
            self.pos += 1;
            let v = if rest == "|" || rest == ">" || rest == "|-" || rest == ">-" {
                self.block_scalar(indent, rest, no)
            } else if rest.is_empty() {
                match self.lines.get(self.pos) {
                    Some(n) if n.indent > indent => {
                        let i = n.indent;
                        self.block(i)?
                    }
                    // a sequence may sit at the same indent as its key
                    Some(n) if n.indent == indent && (n.text.starts_with("- ") || n.text == "-") => self.seq(indent)?,
                    _ => Value::Null,
                }
            } else {
                scalar(rest, no)?
            };
            out.push((k.to_string(), v));
        }
        if self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
            return Err(Error { line: self.lines[self.pos].no, msg: "bad indentation" });
        }
        Ok(Value::Map(out))
    }

    fn block_scalar(&mut self, parent: usize, style: &str, _no: usize) -> Value {
        // Raw lines after the header whose indent > parent (or blank).
        let mut body: Vec<&str> = Vec::new();
        let hdr_raw = if self.pos > 0 { self.lines[self.pos - 1].no } else { 0 };
        let mut j = hdr_raw; // header's 1-based line number == raw index of the following line
        while j < self.raw.len() {
            let r = self.raw[j];
            let ind = r.len() - r.trim_start().len();
            if r.trim().is_empty() || ind > parent {
                body.push(r);
                j += 1;
            } else {
                break;
            }
        }
        while self.pos < self.lines.len() && self.lines[self.pos].no <= j {
            self.pos += 1;
        }
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        let min = body.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
        let lines: Vec<&str> = body.iter().map(|l| if l.len() >= min { &l[min..] } else { "" }).collect();
        let mut s = if style.starts_with('|') { lines.join("\n") } else { lines.join(" ") };
        if !style.ends_with('-') {
            s.push('\n');
        }
        Value::Str(s)
    }
}

/// Parse a document.
pub fn parse(text: &str) -> Result<Value, Error> {
    let raw: Vec<&str> = text.lines().collect();
    let mut lines = Vec::new();
    for (i, r) in raw.iter().enumerate() {
        if r.contains('\t') && r.trim_start_matches(' ').starts_with('\t') {
            return Err(Error { line: i + 1, msg: "tab indentation" });
        }
        let t = strip_comment(r);
        let tt = t.trim_start();
        if tt.is_empty() || (tt == "---" && lines.is_empty()) {
            continue;
        }
        if tt == "---" || tt == "..." {
            return Err(Error { line: i + 1, msg: "multi-document unsupported" });
        }
        lines.push(Line { indent: t.len() - tt.len(), text: tt, no: i + 1 });
    }
    if lines.is_empty() {
        return Ok(Value::Null);
    }
    let indent = lines[0].indent;
    let mut p = P { lines, raw, pos: 0 };
    let v = p.block(indent)?;
    if p.pos < p.lines.len() {
        return Err(Error { line: p.lines[p.pos].no, msg: "unexpected content" });
    }
    Ok(v)
}
