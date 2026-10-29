//! A deliberately small CSS subset for real web pages: class/id/tag selectors (last compound only), colours,
//! font size/weight, display:none, visibility:hidden, margins/padding, borders, background and text decoration.
//! No cascade shorthand beyond that, no variables, no media queries, no layout model. It is enough to hide the
//! menus a page expects to be hidden and to keep text readable.

use alloc::string::String;
use alloc::vec::Vec;

/// Horizontal text alignment requested by CSS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// Left (default).
    #[default]
    Left,
    /// Centre.
    Center,
    /// Right.
    Right,
}

/// Declarations extracted from one rule or an inline `style` attribute.
#[derive(Clone, Debug, Default)]
pub struct Decl {
    pub display: Option<bool>,   /* Some(true) = display:none */
    pub hidden: bool,
    pub color: Option<u32>,
    pub bg: Option<u32>,
    pub size: Option<i32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub align: Option<Align>,
    pub margin_before: Option<i32>,
    pub margin_after: Option<i32>,
    pub indent: Option<i32>,
    pub border: Option<u32>,
    pub pre: bool,
}

impl Decl {
    /// Merge `other` on top of `self` (later declarations win).
    pub fn merge(&mut self, other: &Decl) {
        if other.display.is_some() { self.display = other.display; }
        if other.hidden { self.hidden = true; }
        if other.color.is_some() { self.color = other.color; }
        if other.bg.is_some() { self.bg = other.bg; }
        if other.size.is_some() { self.size = other.size; }
        if other.bold.is_some() { self.bold = other.bold; }
        if other.italic.is_some() { self.italic = other.italic; }
        if other.underline.is_some() { self.underline = other.underline; }
        if other.align.is_some() { self.align = other.align; }
        if other.margin_before.is_some() { self.margin_before = other.margin_before; }
        if other.margin_after.is_some() { self.margin_after = other.margin_after; }
        if other.indent.is_some() { self.indent = other.indent; }
        if other.border.is_some() { self.border = other.border; }
        if other.pre { self.pre = true; }
    }
}

#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    decl: Decl,
    spec: i32,
    order: usize,
}

impl Rule {
    fn matches(&self, tag: &str, id: Option<&str>, classes: &[String]) -> bool {
        if let Some(t) = &self.tag {
            if t != "*" && !t.eq_ignore_ascii_case(tag) { return false; }
        }
        if let Some(want) = &self.id {
            if id != Some(want.as_str()) { return false; }
        }
        self.classes.iter().all(|c| classes.iter().any(|have| have == c))
    }
}

/// A parsed stylesheet (rules in source order).
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
    order: usize,
}

impl StyleSheet {
    /// Parse `<style>` text and append its rules.
    pub fn add(&mut self, css: &str) {
        let src = strip_comments(css);
        let b = src.as_bytes();
        let mut i = 0usize;
        while i < b.len() {
            while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
            if i >= b.len() { break; }
            if b[i] == b'@' {                              /* skip at-rules, including their blocks */
                while i < b.len() && b[i] != b';' && b[i] != b'{' { i += 1; }
                if i < b.len() && b[i] == b'{' { i = skip_block(&src, i); }
                else if i < b.len() { i += 1; }
                continue;
            }
            let sel_start = i;
            while i < b.len() && b[i] != b'{' && b[i] != b'}' { i += 1; }
            if i >= b.len() || b[i] != b'{' { i += 1; continue; }
            let selector = src[sel_start..i].trim();
            let body_start = i + 1;
            i = body_start;
            while i < b.len() && b[i] != b'}' { i += 1; }
            let body = &src[body_start..i.min(b.len())];
            if i < b.len() { i += 1; }
            let decl = parse_decl(body);
            if selector.is_empty() || decl_is_empty(&decl) { continue; }
            for part in selector.split(',') {
                if let Some((tag, id, classes, spec)) = parse_selector(part) {
                    self.order += 1;
                    self.rules.push(Rule { tag, id, classes, decl: decl.clone(), spec, order: self.order });
                }
            }
        }
    }

    /// Resolve the declarations that apply to one element, then overlay its inline `style`.
    pub fn decl_for(&self, tag: &str, id: Option<&str>, classes: &[String], inline: Option<&str>) -> Decl {
        let mut hits: Vec<&Rule> = self.rules.iter().filter(|r| r.matches(tag, id, classes)).collect();
        hits.sort_by_key(|r| (r.spec, r.order));
        let mut out = Decl::default();
        for r in hits { out.merge(&r.decl); }
        if let Some(s) = inline { out.merge(&parse_decl(s)); }
        out
    }
}

fn decl_is_empty(d: &Decl) -> bool {
    d.color.is_none() && d.bg.is_none() && d.size.is_none() && d.bold.is_none() && d.italic.is_none()
        && d.underline.is_none() && d.align.is_none() && d.margin_before.is_none() && d.margin_after.is_none()
        && d.indent.is_none() && d.border.is_none() && d.display.is_none() && !d.hidden && !d.pre
}

fn skip_block(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        if b[i] == b'{' { depth += 1; }
        else if b[i] == b'}' { depth -= 1; if depth == 0 { return i + 1; } }
        i += 1;
    }
    b.len()
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1; }
            i = (i + 2).min(b.len());
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Parse `.cls#id`, `tag.cls`, `*`, and ignore pseudo/attribute tails. Only the last descendant compound counts.
fn parse_selector(part: &str) -> Option<(Option<String>, Option<String>, Vec<String>, i32)> {
    let mut raw = part.trim();
    if let Some(pos) = raw.find(|c: char| c == ':') { raw = &raw[..pos]; }
    if let Some(pos) = raw.find('[') { raw = &raw[..pos]; }
    let compound = raw.split_whitespace().last().unwrap_or("");
    if compound.is_empty() || compound.contains('>') || compound.contains('+') || compound.contains('~') { return None; }
    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();
    let mut buf = String::new();
    let mut mode = 0u8;                                        /* 1 tag, 2 class, 3 id */
    let mut spec = 0;
    for c in compound.chars().chain(core::iter::once(' ')) {
        if c == '.' || c == '#' || c == ' ' {
            if !buf.is_empty() {
                match mode {
                    2 => { classes.push(core::mem::take(&mut buf)); spec += 10; }
                    3 => { id = Some(core::mem::take(&mut buf)); spec += 100; }
                    _ => { tag = Some(core::mem::take(&mut buf).to_ascii_lowercase()); spec += 1; }
                }
            }
            mode = if c == '.' { 2 } else if c == '#' { 3 } else { 0 };
        } else {
            buf.push(c);
        }
    }
    if mode != 0 && !buf.is_empty() { return None; }
    Some((tag, id, classes, spec))
}

fn parse_decl(body: &str) -> Decl {
    let mut d = Decl::default();
    for part in body.split(';') {
        let Some((k, v)) = part.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().trim_end_matches("!important").trim();
        if v.is_empty() { continue; }
        match k.as_str() {
            "display" => {
                let val = v.to_ascii_lowercase();
                d.display = Some(val == "none");
            }
            "visibility" => if v.eq_ignore_ascii_case("hidden") { d.hidden = true; },
            "color" => if let Some(c) = parse_color(v) { d.color = Some(c); },
            "background-color" => if let Some(c) = parse_color(v) { d.bg = Some(c); },
            "background" => {
                if let Some(c) = parse_color(v) { d.bg = Some(c); }
                else if let Some(c) = v.split_whitespace().find_map(parse_color) { d.bg = Some(c); }
            }
            "font-size" => { if let Some(s) = parse_px(v, 14) { d.size = Some(s); } }
            "font-weight" => {
                let val = v.to_ascii_lowercase();
                if val == "bold" || val == "bolder" || val.parse::<i32>().map(|n| n >= 600).unwrap_or(false) { d.bold = Some(true); }
                if val == "normal" || val == "lighter" || val.parse::<i32>().map(|n| n < 600).unwrap_or(false) { d.bold = Some(false); }
            }
            "font-style" => if v.eq_ignore_ascii_case("italic") { d.italic = Some(true); },
            "text-decoration" | "text-decoration-line" => {
                let val = v.to_ascii_lowercase();
                if val.contains("underline") { d.underline = Some(true); }
                if val == "none" { d.underline = Some(false); }
            }
            "text-align" => {
                let val = v.to_ascii_lowercase();
                if val == "center" { d.align = Some(Align::Center); }
                else if val == "right" || val == "end" { d.align = Some(Align::Right); }
                else if val == "left" || val == "start" { d.align = Some(Align::Left); }
            }
            "margin-top" => if let Some(n) = parse_px(v, 14) { d.margin_before = Some(n); },
            "margin-bottom" => if let Some(n) = parse_px(v, 14) { d.margin_after = Some(n); },
            "margin" | "padding" => {
                if let Some(n) = v.split_whitespace().next().and_then(|x| parse_px(x, 14)) {
                    if k == "margin" {
                        d.margin_before = Some(n);
                        d.margin_after = Some(n);
                    } else {
                        d.indent = Some(n);
                    }
                }
            }
            "margin-left" | "padding-left" => if let Some(n) = parse_px(v, 14) { d.indent = Some(n); },
            "border" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                d.border = Some(v.split_whitespace().find_map(parse_color).unwrap_or(0xffdadce0));
            }
            "white-space" => if v.eq_ignore_ascii_case("pre") || v.eq_ignore_ascii_case("pre-wrap") { d.pre = true; },
            _ => {}
        }
    }
    d
}

fn parse_px(v: &str, base: i32) -> Option<i32> {
    let v = v.trim();
    let b = v.as_bytes();
    let mut end = 0usize;
    while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'.' || b[end] == b'-') { end += 1; }
    if end == 0 { return None; }
    let num = v[..end].parse::<f64>().ok()?;
    let unit = v[end..].trim().to_ascii_lowercase();
    let n = if unit.is_empty() || unit == "px" { num }
        else if unit == "pt" { num * 4.0 / 3.0 }
        else if unit == "em" || unit == "rem" { num * base as f64 }
        else if unit == "%" { num * base as f64 / 100.0 }
        else { return None; };
    Some((n as i32).clamp(0, 200))
}

/// Parse `#rgb`, `#rrggbb`, `rgb(...)`, and a handful of named colours.
pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        if h.len() == 3 {
            let r = u8::from_str_radix(&h[0..1], 16).ok()?;
            let g = u8::from_str_radix(&h[1..2], 16).ok()?;
            let b = u8::from_str_radix(&h[2..3], 16).ok()?;
            return Some(0xff00_0000 | ((r * 17) as u32) << 16 | ((g * 17) as u32) << 8 | (b * 17) as u32);
        }
        if h.len() == 6 {
            let n = u32::from_str_radix(h, 16).ok()?;
            return Some(0xff00_0000 | n);
        }
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut it = inner.split(',').map(|x| x.trim().parse::<u32>().ok());
        let (r, g, b) = (it.next()??, it.next()??, it.next()??);
        return Some(0xff00_0000 | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255));
    }
    let c = match lower.as_str() {
        "black" => 0xff000000, "white" => 0xffffffff, "red" => 0xffd32f2f, "green" => 0xff2e7d32,
        "blue" => 0xff1565c0, "gray" | "grey" => 0xff757575, "silver" => 0xffbdbdbd,
        "navy" => 0xff0d47a1, "maroon" => 0xff7b1fa2, "olive" => 0xff827717, "teal" => 0xff00695c,
        "yellow" => 0xfff9a825, "orange" => 0xffef6c00, "purple" => 0xff6a1b9a,
        "transparent" => return None,
        _ => return None,
    };
    Some(c | 0xff00_0000)
}
