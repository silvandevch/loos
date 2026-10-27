//! Small XML subset: elements, attributes, text, comments, CDATA, PIs, the five entities and
//! numeric references. No DTDs (rejected: entity-expansion attacks), no namespaces resolution
//! (prefixes are kept in names; use [`Node::local`]).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Parse error with byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Offset.
    pub at: usize,
    /// Message.
    pub msg: &'static str,
}

/// An element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Qualified name.
    pub name: String,
    /// Attributes in order.
    pub attrs: Vec<(String, String)>,
    /// Child elements.
    pub children: Vec<Node>,
    /// Concatenated text content.
    pub text: String,
}

impl Node {
    /// Name without prefix.
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
    /// Attribute by local name.
    pub fn attr(&self, n: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == n || k.rsplit(':').next() == Some(n) && !k.starts_with("xmlns")).map(|(_, v)| v.as_str())
    }
    /// Child elements with local name `n`.
    pub fn kids<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.local() == n)
    }
    /// First child with local name `n`.
    pub fn kid<'a>(&'a self, n: &'a str) -> Option<&'a Node> {
        self.kids(n).next()
    }
}

const MAX_DEPTH: usize = 64;

struct P<'a> {
    s: &'a str,
    i: usize,
}

fn unescape(s: &str, at: usize) -> Result<String, Error> {
    let mut o = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        o.push_str(&rest[..p]);
        let e = rest[p..].find(';').ok_or(Error { at, msg: "bad entity" })? + p;
        let ent = &rest[p + 1..e];
        match ent {
            "lt" => o.push('<'),
            "gt" => o.push('>'),
            "amp" => o.push('&'),
            "quot" => o.push('"'),
            "apos" => o.push('\''),
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x") { u32::from_str_radix(h, 16).ok() } else { ent.strip_prefix('#').and_then(|d| d.parse().ok()) };
                o.push(n.and_then(char::from_u32).ok_or(Error { at, msg: "unknown entity" })?);
            }
        }
        rest = &rest[e + 1..];
    }
    o.push_str(rest);
    Ok(o)
}

impl<'a> P<'a> {
    fn err<T>(&self, msg: &'static str) -> Result<T, Error> {
        Err(Error { at: self.i, msg })
    }
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn ws(&mut self) {
        while self.rest().starts_with(|c: char| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn skip_misc(&mut self) -> Result<(), Error> {
        loop {
            self.ws();
            if self.rest().starts_with("<!--") {
                let e = self.rest().find("-->").ok_or(Error { at: self.i, msg: "unterminated comment" })?;
                self.i += e + 3;
            } else if self.rest().starts_with("<?") {
                let e = self.rest().find("?>").ok_or(Error { at: self.i, msg: "unterminated PI" })?;
                self.i += e + 2;
            } else if self.rest().starts_with("<!DOCTYPE") || self.rest().starts_with("<!ENTITY") {
                return self.err("DTD not allowed");
            } else {
                return Ok(());
            }
        }
    }
    fn name(&mut self) -> Result<String, Error> {
        let n = self.rest().find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))).unwrap_or(self.rest().len());
        if n == 0 {
            return self.err("expected name");
        }
        let r = self.rest()[..n].to_string();
        self.i += n;
        Ok(r)
    }
    fn element(&mut self, depth: usize) -> Result<Node, Error> {
        if depth > MAX_DEPTH {
            return self.err("too deep");
        }
        if !self.rest().starts_with('<') {
            return self.err("expected element");
        }
        self.i += 1;
        let mut n = Node { name: self.name()?, ..Default::default() };
        loop {
            self.ws();
            if self.rest().starts_with("/>") {
                self.i += 2;
                return Ok(n);
            }
            if self.rest().starts_with('>') {
                self.i += 1;
                break;
            }
            let k = self.name()?;
            self.ws();
            if !self.rest().starts_with('=') {
                return self.err("expected =");
            }
            self.i += 1;
            self.ws();
            let q = self.rest().chars().next().filter(|c| *c == '"' || *c == '\'').ok_or(Error { at: self.i, msg: "expected quote" })?;
            self.i += 1;
            let e = self.rest().find(q).ok_or(Error { at: self.i, msg: "unterminated attribute" })?;
            let v = unescape(&self.rest()[..e], self.i)?;
            self.i += e + 1;
            if n.attrs.iter().any(|(a, _)| *a == k) {
                return self.err("duplicate attribute");
            }
            n.attrs.push((k, v));
        }
        loop {
            let lt = self.rest().find('<').ok_or(Error { at: self.i, msg: "unclosed element" })?;
            n.text.push_str(&unescape(&self.rest()[..lt], self.i)?);
            self.i += lt;
            if self.rest().starts_with("</") {
                self.i += 2;
                let c = self.name()?;
                self.ws();
                if c != n.name || !self.rest().starts_with('>') {
                    return self.err("mismatched close tag");
                }
                self.i += 1;
                return Ok(n);
            } else if self.rest().starts_with("<!--") {
                let e = self.rest().find("-->").ok_or(Error { at: self.i, msg: "unterminated comment" })?;
                self.i += e + 3;
            } else if self.rest().starts_with("<![CDATA[") {
                let e = self.rest().find("]]>").ok_or(Error { at: self.i, msg: "unterminated CDATA" })?;
                n.text.push_str(&self.rest()[9..e]);
                self.i += e + 3;
            } else if self.rest().starts_with("<?") {
                let e = self.rest().find("?>").ok_or(Error { at: self.i, msg: "unterminated PI" })?;
                self.i += e + 2;
            } else {
                let c = self.element(depth + 1)?;
                n.children.push(c);
            }
        }
    }
}

/// Parse a document, returning the root element.
pub fn parse(s: &str) -> Result<Node, Error> {
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut p = P { s, i: 0 };
    p.skip_misc()?;
    let root = p.element(0)?;
    p.skip_misc()?;
    if p.i != s.len() {
        return p.err("trailing content");
    }
    Ok(root)
}
