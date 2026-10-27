//! Dependency expressions.

use crate::version::Version;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Version relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    /// `<<`
    Lt,
    /// `<=`
    Le,
    /// `=`
    Eq,
    /// `>=`
    Ge,
    /// `>>`
    Gt,
}

/// One alternative: `name [(rel version)] [arch]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    /// Package name.
    pub name: String,
    /// Optional version constraint.
    pub ver: Option<(Rel, Version)>,
    /// Architecture qualifier (`:any`, `:amd64`) if present.
    pub arch: Option<String>,
}

impl Atom {
    /// Does version `v` satisfy the constraint?
    pub fn accepts(&self, v: &Version) -> bool {
        match &self.ver {
            None => true,
            Some((r, w)) => {
                let o = v.cmp(w);
                match r {
                    Rel::Lt => o == Ordering::Less,
                    Rel::Le => o != Ordering::Greater,
                    Rel::Eq => o == Ordering::Equal,
                    Rel::Ge => o != Ordering::Less,
                    Rel::Gt => o == Ordering::Greater,
                }
            }
        }
    }
}

/// `a | b` alternatives.
pub type Alt = Vec<Atom>;

fn atom(s: &str) -> Option<Atom> {
    let s = s.trim();
    let (head, ver) = match s.find('(') {
        Some(i) => {
            let end = s.find(')')?;
            let inner = s[i + 1..end].trim();
            let (rel, rest) = if let Some(r) = inner.strip_prefix(">=") {
                (Rel::Ge, r)
            } else if let Some(r) = inner.strip_prefix("<=") {
                (Rel::Le, r)
            } else if let Some(r) = inner.strip_prefix(">>").or_else(|| inner.strip_prefix('>')) {
                (Rel::Gt, r)
            } else if let Some(r) = inner.strip_prefix("<<").or_else(|| inner.strip_prefix('<')) {
                (Rel::Lt, r)
            } else if let Some(r) = inner.strip_prefix('=') {
                (Rel::Eq, r)
            } else {
                return None;
            };
            (s[..i].trim(), Some((rel, Version::parse(rest)?)))
        }
        None => (s.split_whitespace().next()?, None),
    };
    let (name, arch) = match head.split_once(':') {
        Some((n, a)) => (n, Some(a.to_string())),
        None => (head, None),
    };
    if name.is_empty() {
        return None;
    }
    Some(Atom { name: name.to_string(), ver, arch })
}

/// Remove `[arch]` and `<profile>` restriction lists that sit outside `( )`.
fn strip_restrictions(a: &str) -> String {
    let mut out = String::new();
    let (mut paren, mut skip) = (false, 0u32);
    for c in a.chars() {
        match c {
            '(' => paren = true,
            ')' => paren = false,
            '[' | '<' if !paren => {
                skip += 1;
                continue;
            }
            ']' | '>' if !paren && skip > 0 => {
                skip -= 1;
                continue;
            }
            _ => {}
        }
        if skip == 0 {
            out.push(c);
        }
    }
    out
}

/// Parse `Depends`/`Conflicts`/`Provides`-style lists. Architecture/profile restrictions are
/// ignored. `None` on malformed input.
pub fn parse(s: &str) -> Option<Vec<Alt>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        if part.trim().is_empty() {
            continue;
        }
        let mut alt = Vec::new();
        for a in part.split('|') {
            alt.push(atom(&strip_restrictions(a))?);
        }
        out.push(alt);
    }
    Some(out)
}
