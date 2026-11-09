//! A small CSS engine for real web pages: tag/class/id compounds joined by descendant (` `) and child (`>`) combinators,
//! specificity and source order, `@media` (screen, width-based), custom properties with `var()`, colours (hex with alpha,
//! `rgb()/rgba()/hsl()/hsla()`, names), font size/weight/style, display:none, visibility:hidden, margins/padding, borders,
//! background and text decoration. Rules with pseudo-classes, pseudo-elements, attribute or sibling selectors are ignored
//! (they style states and decorations the browser does not have). There is no layout model: this decides what is shown and
//! how text looks, not where boxes go.

use alloc::string::{String, ToString};
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

/// What a selector (or an element) is made of.
#[derive(Clone, Debug, Default)]
pub struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

/// An element on the open-element stack, as selectors see it.
#[derive(Clone, Debug, Default)]
pub struct Anc {
    /// Lower-case tag name.
    pub tag: String,
    /// `id` attribute.
    pub id: Option<String>,
    /// Class names.
    pub classes: Vec<String>,
}

impl Compound {
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

#[derive(Clone, Debug)]
struct Rule {
    subject: Compound,
    /// Ancestors nearest first; `true` = direct child combinator.
    ancestors: Vec<(Compound, bool)>,
    decl: Decl,
    spec: i32,
    order: usize,
}

impl Rule {
    fn matches(&self, tag: &str, id: Option<&str>, classes: &[String], stack: &[Anc]) -> bool {
        if !self.subject.matches(tag, id, classes) { return false; }
        let mut up = stack.len();                      /* stack[..up] are the candidate ancestors */
        for (comp, child) in &self.ancestors {
            if *child {
                if up == 0 { return false; }
                let a = &stack[up - 1];
                if !comp.matches(&a.tag, a.id.as_deref(), &a.classes) { return false; }
                up -= 1;
            } else {
                loop {
                    if up == 0 { return false; }
                    let a = &stack[up - 1];
                    up -= 1;
                    if comp.matches(&a.tag, a.id.as_deref(), &a.classes) { break; }
                }
            }
        }
        true
    }
}

/// A parsed stylesheet (rules in source order).
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
    order: usize,
    vars: Vec<(String, String)>,
}

/// Does a media query apply to a desktop screen of about 1000 logical pixels?
fn media_applies(q: &str) -> bool {
    let q = q.to_ascii_lowercase();
    if q.contains("print") || q.contains("dark") || q.contains("reduce") || q.contains("portrait") || q.contains("hover: none") || q.contains("pointer: coarse") {
        return false;
    }
    let width = |key: &str| -> Option<i32> {
        let i = q.find(key)?;
        parse_px(q[i + key.len()..].trim_start_matches([':', ' ']).split(')').next()?, 16)
    };
    if let Some(n) = width("max-width") { if n < 900 { return false; } }
    if let Some(n) = width("min-width") { if n > 1000 { return false; } }
    true
}

impl StyleSheet {
    /// Parse `<style>` text and append its rules.
    pub fn add(&mut self, css: &str) {
        let src = strip_comments(css);
        self.add_rules(&src, 0);
    }

    fn add_rules(&mut self, src: &str, depth: u32) {
        let b = src.as_bytes();
        let mut i = 0usize;
        while i < b.len() {
            while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
            if i >= b.len() { break; }
            if b[i] == b'@' {
                let at = i;
                while i < b.len() && b[i] != b';' && b[i] != b'{' { i += 1; }
                if i < b.len() && b[i] == b'{' {
                    let end = skip_block(src, i);
                    let head = src[at..i].trim();
                    if depth < 3 && (head.starts_with("@media") && media_applies(&head[6..]) || head.starts_with("@supports") || head.starts_with("@layer")) {
                        self.add_rules(&src[i + 1..end.saturating_sub(1).max(i + 1)], depth + 1);
                    }
                    i = end;
                } else if i < b.len() { i += 1; }
                continue;
            }
            let sel_start = i;
            while i < b.len() && b[i] != b'{' && b[i] != b'}' { i += 1; }
            if i >= b.len() || b[i] != b'{' { i += 1; continue; }
            let selector = src[sel_start..i].trim();
            let body_start = i + 1;
            i = skip_block(src, i);
            let body = &src[body_start..i.saturating_sub(1).max(body_start)];
            self.collect_vars(body);
            let decl = parse_decl(body, &self.vars);
            if selector.is_empty() || decl_is_empty(&decl) { continue; }
            for part in selector.split(',') {
                if let Some((subject, ancestors, spec)) = parse_selector(part) {
                    self.order += 1;
                    self.rules.push(Rule { subject, ancestors, decl: decl.clone(), spec, order: self.order });
                }
            }
        }
    }

    fn collect_vars(&mut self, body: &str) {
        for part in body.split(';') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let k = k.trim();
            if !k.starts_with("--") { continue; }
            let v = v.trim().to_string();
            match self.vars.iter_mut().find(|(n, _)| n == k) {
                Some(e) => e.1 = v,
                None => self.vars.push((k.to_string(), v)),
            }
        }
    }

    /// Resolve the declarations that apply to one element (`stack` = its open ancestors, root first), then overlay its inline `style`.
    pub fn decl_for(&self, tag: &str, id: Option<&str>, classes: &[String], stack: &[Anc], inline: Option<&str>) -> Decl {
        let mut hits: Vec<&Rule> = self.rules.iter().filter(|r| r.matches(tag, id, classes, stack)).collect();
        hits.sort_by_key(|r| (r.spec, r.order));
        let mut out = Decl::default();
        for r in hits { out.merge(&r.decl); }
        if let Some(s) = inline { out.merge(&parse_decl(s, &self.vars)); }
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

fn parse_compound(c: &str) -> Option<(Compound, i32)> {
    let mut out = Compound::default();
    let mut spec = 0;
    let mut buf = String::new();
    let mut mode = 0u8;                                        /* 0 tag, 2 class, 3 id */
    for ch in c.chars().chain(core::iter::once(' ')) {
        if ch == '.' || ch == '#' || ch == ' ' {
            if !buf.is_empty() {
                match mode {
                    2 => { out.classes.push(core::mem::take(&mut buf)); spec += 10; }
                    3 => { out.id = Some(core::mem::take(&mut buf)); spec += 100; }
                    _ => { out.tag = Some(core::mem::take(&mut buf).to_ascii_lowercase()); spec += 1; }
                }
            } else if mode != 0 && ch != ' ' || mode != 0 { return None; }
            mode = if ch == '.' { 2 } else if ch == '#' { 3 } else { 0 };
        } else if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '*' {
            buf.push(ch);
        } else {
            return None;                                       /* attribute, pseudo, escape, sibling...: not supported */
        }
    }
    Some((out, spec))
}

/// Parses `a > b .c#d`: the subject (last compound), its ancestors (nearest first, flagged child/descendant) and specificity.
/// Selectors with pseudo-classes/elements, attributes or sibling combinators return `None`. `:root` means `html`.
fn parse_selector(part: &str) -> Option<(Compound, Vec<(Compound, bool)>, i32)> {
    let raw = part.trim().replace(":root", "html").replace('>', " > ");
    if raw.is_empty() || raw.contains(['[', ':', '+', '~', '(', '\\']) { return None; }
    let mut parts: Vec<(Compound, bool)> = Vec::new();         /* each compound with "is a direct child of the previous" */
    let mut spec = 0;
    let mut child = false;
    for tok in raw.split_whitespace() {
        if tok == ">" { child = true; continue; }
        let (c, sp) = parse_compound(tok)?;
        spec += sp;
        parts.push((c, child));
        child = false;
    }
    if child || parts.is_empty() { return None; }
    let (subject, mut rel) = parts.pop()?;
    let mut ancestors = Vec::new();
    while let Some((c, next_rel)) = parts.pop() {
        ancestors.push((c, rel));
        rel = next_rel;
    }
    Some((subject, ancestors, spec))
}

/// Replaces `var(--x)` / `var(--x, fallback)` using the collected custom properties.
fn substitute_vars(v: &str, vars: &[(String, String)]) -> String {
    let mut out = String::from(v);
    for _ in 0..8 {
        let Some(i) = out.find("var(") else { break };
        let b = out.as_bytes();
        let (mut depth, mut j) = (0, i + 3);
        while j < b.len() {
            if b[j] == b'(' { depth += 1; }
            if b[j] == b')' { depth -= 1; if depth == 0 { break; } }
            j += 1;
        }
        let inner = out[i + 4..j.min(out.len())].to_string();
        let (name, fb) = inner.split_once(',').map_or((inner.as_str(), ""), |(n, f)| (n, f));
        let val = vars.iter().find(|(n, _)| n == name.trim()).map_or(fb.trim().to_string(), |(_, v)| v.clone());
        out.replace_range(i..(j + 1).min(out.len()), &val);
    }
    out
}

/// Splits on whitespace outside parentheses.
fn tokens(v: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, None);
    for (i, c) in v.char_indices() {
        match c {
            '(' => { depth += 1; start.get_or_insert(i); }
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => { if let Some(s) = start.take() { out.push(&v[s..i]); } }
            _ => { start.get_or_insert(i); }
        }
    }
    if let Some(s) = start { out.push(&v[s..]); }
    out
}

fn parse_decl(body: &str, vars: &[(String, String)]) -> Decl {
    let mut d = Decl::default();
    for part in body.split(';') {
        let Some((k, v)) = part.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        if k.starts_with("--") { continue; }
        let v = substitute_vars(v.trim(), vars);
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
                if let Some(c) = tokens(v).into_iter().find_map(parse_color) { d.bg = Some(c); }
            }
            "font-size" => {
                let kw = match v.to_ascii_lowercase().as_str() {
                    "xx-small" => Some(9), "x-small" => Some(10), "small" => Some(12), "medium" => Some(14), "large" => Some(17),
                    "x-large" => Some(20), "xx-large" => Some(26), "smaller" => Some(12), "larger" => Some(17), _ => None,
                };
                if let Some(s) = kw.or_else(|| parse_px(v, 14)) { d.size = Some(s); }
            }
            "font-weight" => {
                let val = v.to_ascii_lowercase();
                if val == "bold" || val == "bolder" || val.parse::<i32>().map(|n| n >= 600).unwrap_or(false) { d.bold = Some(true); }
                if val == "normal" || val == "lighter" || val.parse::<i32>().map(|n| n < 600).unwrap_or(false) { d.bold = Some(false); }
            }
            "font-style" => { d.italic = Some(v.eq_ignore_ascii_case("italic") || v.eq_ignore_ascii_case("oblique")); }
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
                if let Some(n) = tokens(v).first().and_then(|x| parse_px(x, 14)) {
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
                d.border = Some(tokens(v).into_iter().find_map(parse_color).unwrap_or(0xffdadce0));
            }
            "white-space" => if v.eq_ignore_ascii_case("pre") || v.eq_ignore_ascii_case("pre-wrap") { d.pre = true; },
            "border-color" => if let Some(c) = parse_color(v) { d.border = Some(c); },
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

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u32, u32, u32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = ((h % 360.0 + 360.0) % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    let m = l - c / 2.0;
    let f = |v: f64| (((v + m) * 255.0 + 0.5) as i32).clamp(0, 255) as u32;
    (f(r), f(g), f(b))
}

/// Parse `#rgb[a]`, `#rrggbb[aa]`, `rgb()/rgba()/hsl()/hsla()` (comma or space syntax) and named colours.
/// Mostly transparent colours (alpha under 50%) return `None`, like `transparent`.
pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) { return None; }
        let digit = |i: usize| u32::from_str_radix(&h[i..i + 1], 16).ok().map(|x| x * 17);
        let pair = |i: usize| u32::from_str_radix(&h[i..i + 2], 16).ok();
        let (r, g, b, a) = match h.len() {
            3 => (digit(0)?, digit(1)?, digit(2)?, 255),
            4 => (digit(0)?, digit(1)?, digit(2)?, digit(3)?),
            6 => (pair(0)?, pair(2)?, pair(4)?, 255),
            8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
            _ => return None,
        };
        return (a >= 128).then_some(0xff00_0000 | r << 16 | g << 8 | b);
    }
    let lower = v.to_ascii_lowercase();
    if let Some(open) = lower.find('(') {
        let func = lower[..open].trim();
        let inner = lower[open + 1..].trim_end().strip_suffix(')')?;
        let nums: Vec<f64> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|x| !x.is_empty())
            .map(|x| { let pct = x.ends_with('%'); let n = x.trim_end_matches('%').trim_end_matches("deg").parse::<f64>().unwrap_or(0.0); if pct { n / 100.0 * if func.starts_with("rgb") { 255.0 } else { 1.0 } } else { n } })
            .collect();
        if nums.len() < 3 { return None; }
        let alpha = nums.get(3).map_or(1.0, |a| if inner.contains('%') && *a > 1.0 { *a / 100.0 } else { *a });
        if alpha < 0.5 { return None; }
        let (r, g, b) = match func {
            "rgb" | "rgba" => (nums[0].clamp(0.0, 255.0) as u32, nums[1].clamp(0.0, 255.0) as u32, nums[2].clamp(0.0, 255.0) as u32),
            "hsl" | "hsla" => hsl_to_rgb(nums[0], nums[1].clamp(0.0, 1.0), nums[2].clamp(0.0, 1.0)),
            _ => return None,
        };
        return Some(0xff00_0000 | r << 16 | g << 8 | b);
    }
    let c = match lower.as_str() {
        "black" => 0xff000000, "white" => 0xffffffff, "red" => 0xffd32f2f, "green" => 0xff2e7d32,
        "blue" => 0xff1565c0, "gray" | "grey" => 0xff757575, "silver" => 0xffbdbdbd,
        "navy" => 0xff0d47a1, "maroon" => 0xff7b1fa2, "olive" => 0xff827717, "teal" => 0xff00695c,
        "yellow" => 0xfff9a825, "orange" => 0xffef6c00, "purple" => 0xff6a1b9a,
        "lime" => 0xff00e676, "aqua" | "cyan" => 0xff00bcd4, "fuchsia" | "magenta" => 0xffd500f9, "pink" => 0xfff48fb1,
        "brown" => 0xff6d4c41, "gold" => 0xffffc107, "indigo" => 0xff3f51b5, "violet" => 0xff9c27b0, "crimson" => 0xffc62828,
        "darkgray" | "darkgrey" => 0xff616161, "lightgray" | "lightgrey" => 0xffd3d3d3, "whitesmoke" => 0xfff5f5f5,
        "darkblue" => 0xff0d2a6b, "lightblue" => 0xff81d4fa, "darkgreen" => 0xff1b5e20, "lightgreen" => 0xffa5d6a7,
        "darkred" => 0xff8b0000, "dimgray" | "dimgrey" => 0xff696969, "gainsboro" => 0xffdcdcdc, "ghostwhite" => 0xfff8f8ff,
        "steelblue" => 0xff4682b4, "royalblue" => 0xff4169e1, "dodgerblue" => 0xff1e90ff, "tomato" => 0xffff6347,
        "coral" => 0xffff7f50, "salmon" => 0xfffa8072, "khaki" => 0xfff0e68c, "beige" => 0xfff5f5dc, "ivory" => 0xfffffff0,
        "transparent" | "currentcolor" | "inherit" | "initial" => return None,
        _ => return None,
    };
    Some(c | 0xff00_0000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn anc(tag: &str, class: &str) -> Anc {
        Anc { tag: tag.to_string(), id: None, classes: class.split_whitespace().map(|c| c.to_string()).collect() }
    }
    fn color_for(sheet: &StyleSheet, tag: &str, class: &str, stack: &[Anc]) -> Option<u32> {
        let classes: Vec<String> = class.split_whitespace().map(|c| c.to_string()).collect();
        sheet.decl_for(tag, None, &classes, stack, None).color
    }

    #[test]
    fn descendant_and_child_combinators() {
        let mut s = StyleSheet::default();
        s.add(".nav a { color: #ff0000 } .box > p { color: #00ff00 } p { color: #0000ff }");
        assert_eq!(color_for(&s, "a", "", &[anc("div", "nav"), anc("ul", "")]), Some(0xffff0000));
        assert_eq!(color_for(&s, "a", "", &[anc("div", "other")]), None);
        assert_eq!(color_for(&s, "p", "", &[anc("div", "box")]), Some(0xff00ff00));
        assert_eq!(color_for(&s, "p", "", &[anc("div", "box"), anc("section", "")]), Some(0xff0000ff), "not a direct child");
    }

    #[test]
    fn specificity_order_and_ignored_selectors() {
        let mut s = StyleSheet::default();
        s.add("a { color: #111111 } .x { color: #222222 } a:hover { color: #333333 } a[href] { color: #444444 } a::before { color: #555555 }");
        assert_eq!(color_for(&s, "a", "x", &[]), Some(0xff222222));
        assert_eq!(color_for(&s, "a", "", &[]), Some(0xff111111), "pseudo and attribute rules never apply");
    }

    #[test]
    fn media_queries() {
        let mut s = StyleSheet::default();
        s.add("@media (max-width: 600px) { p { color: #111111 } } @media screen and (min-width: 768px) { p { color: #222222 } } @media print { p { color: #333333 } } @media (prefers-color-scheme: dark) { p { color: #444444 } }");
        assert_eq!(color_for(&s, "p", "", &[]), Some(0xff222222));
    }

    #[test]
    fn variables_and_colors() {
        let mut s = StyleSheet::default();
        s.add(":root { --bg: #102030; --fg: rgb(10 20 30) } body { background: var(--bg); color: var(--fg) } p { color: var(--nope, hsl(120, 100%, 25%)) }");
        let d = s.decl_for("body", None, &[], &[anc("html", "")], None);
        assert_eq!((d.bg, d.color), (Some(0xff102030), Some(0xff0a141e)));
        assert_eq!(color_for(&s, "p", "", &[]), Some(0xff008000));
        assert_eq!(parse_color("#ff000080"), Some(0xffff0000));
        assert_eq!(parse_color("#ff00007f"), None);
        assert_eq!(parse_color("rgba(0,0,0,0.2)"), None);
        assert_eq!(parse_color("rgba(0, 0, 255, 0.9)"), Some(0xff0000ff));
        let _ = vec![0];
    }
}
