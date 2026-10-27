//! Control-file stanzas (`/var/lib/dpkg/status`, `Packages`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One stanza: ordered fields; continuation lines are joined with `\n`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stanza {
    /// `(name, value)` in file order.
    pub fields: Vec<(String, String)>,
}

impl Stanza {
    /// Case-insensitive field lookup.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Serialize back to control syntax (continuation lines indented by one space).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.fields {
            s.push_str(k);
            s.push(':');
            for (i, line) in v.split('\n').enumerate() {
                if i == 0 {
                    if !line.is_empty() {
                        s.push(' ');
                    }
                } else {
                    s.push_str("\n ");
                }
                s.push_str(if i > 0 && line.is_empty() { "." } else { line });
            }
            s.push('\n');
        }
        s
    }
}

/// Parse a file into stanzas. Comment lines (`#`) are skipped; malformed lines are an error.
pub fn parse(text: &str) -> Result<Vec<Stanza>, usize> {
    let mut out = Vec::new();
    let mut cur = Stanza::default();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            if !cur.fields.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
        } else if line.starts_with(' ') || line.starts_with('\t') {
            let (_, v) = cur.fields.last_mut().ok_or(n + 1)?;
            v.push('\n');
            let t = line.trim_start();
            v.push_str(if t == "." { "" } else { t });
        } else {
            let (k, v) = line.split_once(':').ok_or(n + 1)?;
            if k.is_empty() || k.contains(char::is_whitespace) {
                return Err(n + 1);
            }
            cur.fields.push((k.to_string(), v.trim().to_string()));
        }
    }
    if !cur.fields.is_empty() {
        out.push(cur);
    }
    Ok(out)
}
