//! LoOS Browser: a local-first browser for the LoOS desktop.
//!
//! Shipped in this milestone ("basics"):
//! - tabs, back/forward history, reload, bookmarks and local history that survive a restart;
//! - an HTML subset renderer (headings, paragraphs, links, lists, bold/italic/code,
//!   quotes, rules, preformatted text, `alt` text) with word wrapping and scrolling;
//! - `loos://` pages (start, bookmarks, history, about), local files (`file://`, `/path`,
//!   Windows paths) and local search over bookmarks and `/home/user/Web`;
//! - real `http`/`https` pages **under `./loos qemu`** through the host-assisted serial web
//!   bridge (`boot/web-bridge.py`), with a CSS subset, meta-refresh and a light page theme;
//!   without the bridge the browser shows a local explanation page and sends nothing.
//!
//! The full Firefox/Gecko port is planned in `docs/BROWSER.md`. This module is the native
//! browser *shell* that the port will reuse (tabs, session/history, address bar, bookmarks,
//! permissions and the shell integration), not a stand-in for Gecko.
#![allow(clippy::missing_safety_doc, missing_docs)]

use super::term::Host;
mod css;

use self::css::{Align, Anc, Decl, StyleSheet};
use super::{cs, BeHost, Ui, MOD_CTRL};
use crate::draw::Rect;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

const MOD_ALT: u32 = 4;
const TAB_H: i32 = 32;
const BAR_H: i32 = 40;
const STATUS_H: i32 = 22;
const PAGE_TOP: i32 = 16;
const MARGIN: i32 = 26;
const MAX_CONTENT: i32 = 900;
const PLACEHOLDER: &str = "Search or enter address";
const MAX_TABS: usize = 12;
const MAX_HISTORY: usize = 80;
const BOOKMARK_FILE: &str = "/home/user/.config/loos-browser/bookmarks.tsv";
const HISTORY_FILE: &str = "/home/user/.config/loos-browser/history.tsv";
const HOME: &str = "loos://home";
const WEB_COMPRESSED_MAX: usize = 3 * 1024 * 1024;
const WEB_RAW_MAX: usize = 2 * 1024 * 1024;

extern "C" {
    /// `src/shell/web.c`: compressed fetch through the QEMU COM2 web bridge (false when absent).
    fn loos_web_fetch(
        url: *const core::ffi::c_char,
        out: *mut core::ffi::c_char,
        cap: usize,
        out_len: *mut usize,
        raw_len: *mut usize,
        final_url: *mut core::ffi::c_char,
        final_cap: usize,
    ) -> bool;
    /// `web.rs`: why the last fetch failed (NUL-terminated text; length 0 when unknown).
    fn loos_web_last_error(out: *mut core::ffi::c_char, cap: usize) -> usize;
}

fn web_error() -> String {
    let mut b = [0u8; 256];
    // SAFETY: b is writable for its length.
    let n = unsafe { loos_web_last_error(b.as_mut_ptr().cast(), b.len()) };
    if n == 0 {
        return "the request failed".to_string();
    }
    String::from_utf8_lossy(&b[..n.min(b.len())]).into_owned()
}

// ------------------------------------------------------------------ document model

/// Block-level element kinds the renderer understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Normal paragraph.
    P,
    /// Top heading.
    H1,
    /// Section heading.
    H2,
    /// Minor heading.
    H3,
    /// List item.
    Li,
    /// Preformatted text.
    Pre,
    /// Block quote.
    Quote,
    /// Horizontal rule.
    Rule,
}

impl Default for BlockKind {
    fn default() -> Self {
        BlockKind::P
    }
}

/// One run of text with a single style.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Span {
    /// Text (already entity-decoded).
    pub text: String,
    /// Link target for this run, if any.
    pub link: Option<String>,
    /// Bold/strong.
    pub bold: bool,
    /// Italic/emphasis (drawn with the regular face; Inter has no italic face yet).
    pub italic: bool,
    /// Inline code.
    pub code: bool,
    /// CSS colour override.
    pub color: Option<u32>,
    /// CSS font size override (logical px).
    pub size: Option<i32>,
    /// CSS underline.
    pub underline: bool,
}

/// CSS-derived block presentation.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BlockStyle {
    /// Font size in logical px.
    pub size: Option<i32>,
    /// Bold.
    pub bold: Option<bool>,
    /// Text colour.
    pub color: Option<u32>,
    /// Background colour.
    pub bg: Option<u32>,
    /// Alignment.
    pub align: Option<Align>,
    /// Space before the block.
    pub margin_before: Option<i32>,
    /// Space after the block.
    pub margin_after: Option<i32>,
    /// Left indent/padding.
    pub indent: i32,
    /// Border colour (1 px).
    pub border: Option<u32>,
    /// Underline.
    pub underline: bool,
}

/// A rendered block.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Block {
    /// Element kind.
    pub kind: BlockKind,
    /// Inline runs.
    pub spans: Vec<Span>,
    /// List marker (`"- "` or `"1. "`), for [`BlockKind::Li`].
    pub marker: Option<String>,
    /// CSS presentation.
    pub style: BlockStyle,
}

/// A parsed page.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    /// Document title (`<title>` or the file name).
    pub title: String,
    /// Blocks in document order.
    pub blocks: Vec<Block>,
    /// True for pages fetched from the web (rendered on a light background).
    pub web: bool,
    /// CSS page background.
    pub bg: Option<u32>,
    /// CSS page text colour.
    pub fg: Option<u32>,
    /// CSS page border colour.
    pub border: Option<u32>,
    /// `<meta http-equiv="refresh">` target, if any.
    pub redirect: Option<String>,
}

impl Doc {
    #[cfg(test)]
    fn plain_text(&self) -> String {
        let mut s = String::new();
        for b in &self.blocks {
            for x in &b.spans {
                s.push_str(&x.text);
            }
            s.push('\n');
        }
        s
    }
}

// ------------------------------------------------------------------ HTML subset parser

struct Builder<'a> {
    base: &'a str,
    title: String,
    blocks: Vec<Block>,
    spans: Vec<Span>,
    kind: BlockKind,
    marker: Option<String>,
    lists: Vec<(bool, usize)>,
    bold: u32,
    italic: u32,
    code: u32,
    link: Option<String>,
    in_head: bool,
    in_title: bool,
    skip: u32,
    skip_tag: Option<String>,
    in_style: bool,
    css_buf: String,
    css: StyleSheet,
    pre: u32,
    web: bool,
    hidden: u32,
    elements: Vec<(String, bool, Anc)>,
    block_style: BlockStyle,
    in_color: Option<u32>,
    in_size: Option<i32>,
    in_underline: bool,
    in_bold: bool,
    inline_restore: Vec<(String, Option<u32>, Option<i32>, bool, bool)>,
    page_bg: Option<u32>,
    page_fg: Option<u32>,
    page_border: Option<u32>,
    redirect: Option<String>,
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr"
    )
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let b = html.as_bytes();
    let mut quote = 0u8;
    let mut i = start + 1;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i] != b'&' {
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let rest = &s[i..];
        let end = rest.find(';').filter(|&k| k <= 12);
        let Some(k) = end else {
            out.push('&');
            i += 1;
            continue;
        };
        let name = &rest[1..k];
        let decoded = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = name.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "mdash" => Some('\u{2014}'),
                "ndash" => Some('\u{2013}'),
                "hellip" => Some('\u{2026}'),
                "copy" => Some('\u{a9}'),
                "reg" => Some('\u{ae}'),
                "trade" => Some('\u{2122}'),
                "laquo" => Some('\u{ab}'),
                "raquo" => Some('\u{bb}'),
                "deg" => Some('\u{b0}'),
                "plusmn" => Some('\u{b1}'),
                "times" => Some('\u{d7}'),
                "divide" => Some('\u{f7}'),
                "middot" => Some('\u{b7}'),
                "bull" => Some('\u{2022}'),
                "euro" => Some('\u{20ac}'),
                "pound" => Some('\u{a3}'),
                "yen" => Some('\u{a5}'),
                "cent" => Some('\u{a2}'),
                "sect" => Some('\u{a7}'),
                "para" => Some('\u{b6}'),
                _ => None,
            }
        };
        match decoded {
            Some(c) => {
                out.push(c);
                i += k + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// Case-insensitive attribute lookup in a raw tag body; values are entity-decoded.
fn attr(attrs: &str, want: &str) -> Option<String> {
    let b = attrs.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/') {
            i += 1;
        }
        let key = &attrs[start..i];
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        let val = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i];
            i += 1;
            let vs = i;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            let v = &attrs[vs..i];
            if i < b.len() {
                i += 1;
            }
            v
        } else {
            let vs = i;
            while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'>') {
                i += 1;
            }
            &attrs[vs..i]
        };
        if key.eq_ignore_ascii_case(want) {
            return Some(decode_entities(val));
        }
    }
    None
}

fn has_scheme(s: &str) -> bool {
    let Some(colon) = s.find(':') else { return false };
    if colon == 0 || !s[colon + 1..].starts_with("//") {
        return false;
    }
    s[..colon].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Resolve a link against the page it appeared on.
pub fn resolve_href(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return String::new();
    }
    if has_scheme(href) {
        return href.to_string();
    }
    if base.starts_with("loos://") || base.starts_with("about:") {
        let root = base.strip_prefix("loos://").unwrap_or("home");
        if href.starts_with('/') {
            return format!("loos://{}", href.trim_start_matches('/'));
        }
        if let Some(slash) = root.rfind('/') {
            return normalize_path(&format!("loos://{}{}", &root[..=slash], href), "loos://");
        }
        return format!("loos://{href}");
    }
    if base.starts_with('/') {
        return join_path(base, href);
    }
    if base.starts_with("http://") || base.starts_with("https://") {
        if let Some(stripped) = href.strip_prefix("//") {
            let scheme = if base.starts_with("https") { "https:" } else { "http:" };
            return format!("{scheme}{stripped}");
        }
        let scheme_end = base.find("://").map_or(0, |i| i + 3);
        let host_end = base[scheme_end..].find('/').map_or(base.len(), |i| scheme_end + i);
        if href.starts_with('/') {
            return format!("{}{}", &base[..host_end], href);
        }
        let dir_end = base.rfind('/').filter(|&i| i >= scheme_end).map_or(host_end, |i| i + 1);
        return format!("{}{}", &base[..dir_end], href);
    }
    href.to_string()
}

fn join_path(base: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        return normalize_path(rel, "");
    }
    let dir_end = base.rfind('/').map_or(0, |i| i + 1);
    normalize_path(&format!("{}{}", &base[..dir_end], rel), "")
}

/// Collapse `.`/`..` segments; `prefix` is retained verbatim (`loos://` or empty).
fn normalize_path(p: &str, prefix: &str) -> String {
    let body = if prefix.is_empty() { p } else { p.strip_prefix(prefix).unwrap_or(p) };
    let mut parts: Vec<&str> = Vec::new();
    for seg in body.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = String::from(prefix);
    if prefix.is_empty() {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    if body.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    out
}

impl<'a> Builder<'a> {
    fn new(base: &'a str) -> Self {
        Builder {
            base,
            title: String::new(),
            blocks: Vec::new(),
            spans: Vec::new(),
            kind: BlockKind::P,
            marker: None,
            lists: Vec::new(),
            bold: 0,
            italic: 0,
            code: 0,
            link: None,
            in_head: false,
            in_title: false,
            skip: 0,
            skip_tag: None,
            in_style: false,
            css_buf: String::new(),
            css: StyleSheet::default(),
            pre: 0,
            web: false,
            hidden: 0,
            elements: Vec::new(),
            block_style: BlockStyle::default(),
            in_color: None,
            in_size: None,
            in_underline: false,
            in_bold: false,
            inline_restore: Vec::new(),
            page_bg: None,
            page_fg: None,
            page_border: None,
            redirect: None,
        }
    }

    fn run(mut self, html: &str) -> Doc {
        let mut i = 0;
        while i < html.len() {
            if html.as_bytes()[i] == b'<' {
                if html[i..].starts_with("<!--") {
                    i = html[i + 4..].find("-->").map_or(html.len(), |k| i + 4 + k + 3);
                    continue;
                }
                let Some(end) = find_tag_end(html, i) else {
                    self.text("<");
                    i += 1;
                    continue;
                };
                self.tag(&html[i + 1..end]);
                i = end + 1;
            } else {
                let next = html[i..].find('<').map_or(html.len(), |k| i + k);
                let text = decode_entities(&html[i..next]);
                self.text(&text);
                i = next;
            }
        }
        self.finish()
    }

    fn finish(mut self) -> Doc {
        self.flush();
        let mut title = self.title.trim().to_string();
        if title.is_empty() {
            title = self
                .blocks
                .iter()
                .find(|b| matches!(b.kind, BlockKind::H1 | BlockKind::H2))
                .map(|b| b.spans.iter().map(|s| s.text.as_str()).collect::<String>())
                .unwrap_or_default()
                .trim()
                .to_string();
        }
        if title.is_empty() {
            title = "Untitled".to_string();
        }
        Doc { title, blocks: self.blocks, web: self.web, bg: self.page_bg, fg: self.page_fg, border: self.page_border, redirect: self.redirect }
    }

    fn text(&mut self, raw: &str) {
        if self.in_style {
            self.css_buf.push_str(raw);
            return;
        }
        if self.skip > 0 || self.skip_tag.is_some() || self.hidden > 0 {
            return;
        }
        if self.in_title {
            self.title.push_str(raw);
            return;
        }
        if self.in_head {
            return;
        }
        let text = if self.pre > 0 { raw.replace('\t', "    ") } else { collapse_ws(raw) };
        if text.is_empty() {
            return;
        }
        let style = (
            self.bold > 0 || self.in_bold,
            self.italic > 0,
            self.code > 0,
            self.link.clone(),
            self.in_color,
            self.in_size,
            self.in_underline,
        );
        if let Some(last) = self.spans.last_mut() {
            if (last.bold, last.italic, last.code, last.link.as_ref(), last.color, last.size, last.underline)
                == (style.0, style.1, style.2, style.3.as_ref(), style.4, style.5, style.6)
            {
                last.text.push_str(&text);
                return;
            }
        }
        self.spans.push(Span {
            text,
            link: style.3,
            bold: style.0,
            italic: style.1,
            code: style.2,
            color: style.4,
            size: style.5,
            underline: style.6,
        });
    }

    fn decl_for(&self, name: &str, attrs: &str) -> Decl {
        let class_s = attr(attrs, "class").unwrap_or_default();
        let classes: Vec<String> = class_s.split_whitespace().map(|x| x.to_string()).collect();
        let id = attr(attrs, "id");
        let inline = attr(attrs, "style");
        let stack: Vec<Anc> = self.elements.iter().map(|e| e.2.clone()).collect();
        self.css.decl_for(name, id.as_deref(), &classes, &stack, inline.as_deref())
    }

    fn begin_block(&mut self, kind: BlockKind, name: &str, attrs: &str) {
        self.flush();
        self.kind = kind;
        let decl = self.decl_for(name, attrs);
        if decl.pre {
            self.kind = BlockKind::Pre;
            self.pre += 1;
        }
        self.block_style = block_style(kind, &decl);
    }

    fn apply_inline(&mut self, name: &str, d: &Decl) {
        self.inline_restore.push((name.to_string(), self.in_color, self.in_size, self.in_underline, self.in_bold));
        if d.color.is_some() { self.in_color = d.color; }
        if d.size.is_some() { self.in_size = d.size; }
        if let Some(u) = d.underline { self.in_underline = u; }
        if let Some(b) = d.bold { self.in_bold = b; }
    }

    fn restore_inline(&mut self, name: &str) {
        if self.inline_restore.last().map(|x| x.0 == name).unwrap_or(false) {
            if let Some((_, c, s, u, b)) = self.inline_restore.pop() {
                self.in_color = c; self.in_size = s; self.in_underline = u; self.in_bold = b;
            }
        }
    }

    fn close_element(&mut self, name: &str) {
        while let Some((n, causes, _)) = self.elements.pop() {
            if causes {
                self.hidden = self.hidden.saturating_sub(1);
                #[cfg(test)]
                std::eprintln!("UNHIDE </{}> depth={} hidden={}", n, self.elements.len(), self.hidden);
            }
            if n == name { break; }
        }
    }

    fn tag(&mut self, raw: &str) {
        let s = raw.trim();
        if s.is_empty() {
            return;
        }
        let closing = s.starts_with('/');
        let body = s.trim_start_matches('/').trim_end_matches('/').trim();
        let name_end = body.find(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r').unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        let attrs = &body[name_end..];

        if self.in_style {
            if closing && name == "style" {
                self.in_style = false;
                self.css.add(&self.css_buf);
                self.css_buf.clear();
            }
            return;
        }
        if let Some(skip) = &self.skip_tag {
            if closing && name == *skip { self.skip_tag = None; }
            return;
        }
        if self.in_title {
            if closing && name == "title" { self.in_title = false; }
            return;
        }
        if self.skip > 0 {
            if closing && name == "script" { self.skip = self.skip.saturating_sub(1); }
            return;
        }

        if !closing && (name == "script" || name == "svg" || name == "template" || name == "iframe" || name == "canvas") {
            self.skip_tag = Some(name);
            return;
        }
        if closing && (name == "script" || name == "svg" || name == "template" || name == "iframe" || name == "canvas") {
            return;
        }
        if !closing && name == "style" {
            self.in_style = true;
            self.css_buf.clear();
            return;
        }

        let d = if closing { Decl::default() } else { self.decl_for(&name, attrs) };
        if !closing && !is_void(&name) {
            let hide = d.display == Some(true) || d.hidden;
            let id = attr(attrs, "id");
            let classes = attr(attrs, "class").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
            self.elements.push((name.clone(), hide, Anc { tag: name.clone(), id, classes }));
            if hide {
                self.hidden += 1;
                #[cfg(test)]
                std::eprintln!("HIDE <{}> class={:?} style={:?} depth={} hidden={}", name, attr(attrs, "class"), attr(attrs, "style"), self.elements.len(), self.hidden);
            }
        } else if closing {
            self.close_element(&name);
        }

        let inline_tag = matches!(name.as_str(), "a" | "span" | "b" | "strong" | "i" | "em" | "code" | "kbd" | "samp" | "u" | "small" | "big" | "font" | "mark" | "sub" | "sup" | "label");
        if inline_tag {
            if closing {
                self.restore_inline(&name);
            } else {
                self.apply_inline(&name, &d);
            }
        }

        match name.as_str() {
            "title" => self.in_title = !closing,
            "head" => self.in_head = !closing,
            "body" => {
                self.in_head = false;
                if !closing {
                    if d.bg.is_some() { self.page_bg = d.bg; }
                    if d.color.is_some() { self.page_fg = d.color; }
                    if d.border.is_some() { self.page_border = d.border; }
                    if let Some(c) = attr(attrs, "bgcolor").and_then(|v| css::parse_color(&v)) { self.page_bg = Some(c); }
                    if let Some(c) = attr(attrs, "text").and_then(|v| css::parse_color(&v)) { self.page_fg = Some(c); }
                }
            }
            "meta" => {
                if !closing {
                    let equiv = attr(attrs, "http-equiv").unwrap_or_default();
                    if equiv.eq_ignore_ascii_case("refresh") {
                        if let Some(rest) = attr(attrs, "content").and_then(|c| c.split_once(';').map(|x| x.1.to_string())) {
                            let u = rest.trim();
                            let u = if u.len() > 4 && u[..4].eq_ignore_ascii_case("url=") { u[4..].trim().trim_matches('"').trim_matches('\'').to_string() } else { String::new() };
                            if !u.is_empty() && self.redirect.is_none() {
                                self.redirect = Some(resolve_href(self.base, &u));
                            }
                        }
                    }
                }
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside" | "form" | "table"
            | "thead" | "tbody" | "tfoot" | "tr" | "center" => {
                if closing {
                    self.flush();
                    self.kind = BlockKind::P;
                } else {
                    self.begin_block(BlockKind::P, &name, attrs);
                    if name == "center" { self.block_style.align = Some(Align::Center); }
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if closing {
                    self.flush();
                    self.kind = BlockKind::P;
                } else {
                    let kind = match name.as_str() { "h1" => BlockKind::H1, "h2" => BlockKind::H2, _ => BlockKind::H3 };
                    self.begin_block(kind, &name, attrs);
                }
            }
            "br" => if !closing { self.text("\n"); },
            "hr" => {
                if !closing {
                    self.flush();
                    self.blocks.push(Block { kind: BlockKind::Rule, spans: Vec::new(), marker: None, style: block_style(BlockKind::Rule, &d) });
                }
            }
            "ul" | "ol" => {
                self.flush();
                if closing {
                    self.lists.pop();
                    self.kind = BlockKind::P;
                } else {
                    self.lists.push((name == "ol", 0));
                }
            }
            "li" => {
                if closing {
                    self.flush();
                    self.kind = BlockKind::P;
                } else {
                    self.begin_block(BlockKind::Li, &name, attrs);
                    self.marker = match self.lists.last_mut() {
                        Some((true, n)) => { *n += 1; Some(format!("{}. ", n)) }
                        Some((false, _)) => Some("- ".to_string()),
                        None => Some("- ".to_string()),
                    };
                }
            }
            "blockquote" => {
                if closing { self.flush(); self.kind = BlockKind::P; }
                else { self.begin_block(BlockKind::Quote, &name, attrs); }
            }
            "pre" => {
                if closing {
                    self.flush();
                    self.pre = self.pre.saturating_sub(1);
                    self.kind = BlockKind::P;
                } else {
                    self.begin_block(BlockKind::Pre, &name, attrs);
                    self.pre += 1;
                    self.kind = BlockKind::Pre;
                }
            }
            "a" => {
                if closing {
                    self.link = None;
                } else if let Some(href) = attr(attrs, "href") {
                    let target = resolve_href(self.base, &href);
                    self.link = if target.is_empty() { None } else { Some(target) };
                }
            }
            "b" | "strong" => self.bold = bump(self.bold, closing),
            "i" | "em" => self.italic = bump(self.italic, closing),
            "code" | "kbd" | "samp" => self.code = bump(self.code, closing),
            "font" => {
                if !closing {
                    if let Some(c) = attr(attrs, "color").and_then(|v| css::parse_color(&v)) { self.in_color = Some(c); }
                    if let Some(sz) = attr(attrs, "size").and_then(|v| v.trim().parse::<i32>().ok()) {
                        self.in_size = Some((sz.clamp(1, 7) * 3 + 9) as i32);
                    }
                }
            }
            "img" => {
                if !closing {
                    let alt = attr(attrs, "alt").unwrap_or_default();
                    if alt.is_empty() {
                        if !self.web { self.text("[image]"); }
                    } else if self.web {
                        let src = attr(attrs, "src").unwrap_or_default();
                        let wide = attr(attrs, "width").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) >= 100;
                        if src.to_ascii_lowercase().contains("logo") || wide {
                            let parent_align = self.block_style.align;
                            let parent_color = self.block_style.color;
                            self.flush();
                            let st = BlockStyle {
                                size: Some(34),
                                bold: Some(true),
                                color: parent_color,
                                align: Some(parent_align.unwrap_or(Align::Center)),
                                margin_before: Some(6),
                                margin_after: Some(6),
                                ..BlockStyle::default()
                            };
                            self.blocks.push(Block {
                                kind: BlockKind::H1,
                                spans: alloc::vec![Span { text: alt, bold: true, ..Default::default() }],
                                marker: None,
                                style: st,
                            });
                        } else {
                            self.text(&format!("{alt} "));
                        }
                    } else {
                        self.text(&format!("[image: {alt}]"));
                    }
                }
            }
            "input" => {
                if !closing {
                    let typ = attr(attrs, "type").unwrap_or_default().to_ascii_lowercase();
                    if typ != "hidden" {
                        let label = attr(attrs, "placeholder").or_else(|| attr(attrs, "aria-label")).or_else(|| attr(attrs, "title")).or_else(|| attr(attrs, "value")).unwrap_or_default();
                        if label.is_empty() { self.text("[ ] "); } else { self.text(&format!("[{label}] ")); }
                    }
                }
            }
            "td" | "th" => if !closing { self.text("  "); },
            "select" => { /* options are rendered as their text */ }
            _ => { if is_void(&name) { return; } }
        }
    }

    fn flush(&mut self) {
        if self.spans.is_empty() {
            self.marker = None;
            if self.pre == 0 {
                self.kind = BlockKind::P;
            }
            self.block_style = BlockStyle::default();
            return;
        }
        let mut spans = core::mem::take(&mut self.spans);
        if self.pre == 0 {
            if let Some(f) = spans.first_mut() {
                let t = f.text.trim_start();
                if t.len() != f.text.len() {
                    f.text = t.to_string();
                }
            }
            if let Some(l) = spans.last_mut() {
                let t = l.text.trim_end();
                if t.len() != l.text.len() {
                    l.text = t.to_string();
                }
            }
        }
        spans.retain(|s| !s.text.is_empty());
        if !spans.is_empty() && self.blocks.len() < 20_000 {
            let kind = if self.pre > 0 { BlockKind::Pre } else { self.kind };
            let block = Block { kind, spans, marker: self.marker.take(), style: self.block_style.clone() };
            self.blocks.push(block);
        } else {
            self.marker = None;
        }
        if self.pre == 0 {
            self.kind = BlockKind::P;
        }
        self.block_style = BlockStyle::default();
    }
}

fn block_style(kind: BlockKind, d: &Decl) -> BlockStyle {
    let (def_size, def_bold) = match kind {
        BlockKind::H1 => (28, true),
        BlockKind::H2 => (21, true),
        BlockKind::H3 => (17, true),
        BlockKind::Pre => (13, false),
        _ => (14, false),
    };
    BlockStyle {
        size: Some(d.size.unwrap_or(def_size)),
        bold: Some(d.bold.unwrap_or(def_bold)),
        color: d.color,
        bg: d.bg,
        align: d.align,
        margin_before: d.margin_before,
        margin_after: d.margin_after,
        indent: d.indent.unwrap_or(0),
        border: d.border,
        underline: d.underline.unwrap_or(false),
    }
}

fn bump(depth: u32, closing: bool) -> u32 {
    if closing {
        depth.saturating_sub(1)
    } else {
        depth + 1
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && (out.is_empty() || !out.ends_with(' ')) {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// Parse the supported HTML subset. `base` resolves relative links.
pub fn parse_html(html: &str, base: &str) -> Doc {
    Builder::new(base).run(html)
}

fn parse_html_web(html: &str, base: &str) -> Doc {
    let mut b = Builder::new(base);
    b.web = true;
    b.run(html)
}

fn plain_doc(text: &str, title: &str) -> Doc {
    let mut blocks = Vec::new();
    let mut body = text;
    if body.ends_with('\n') {
        body = &body[..body.len() - 1];
    }
    for para in body.split("\n\n") {
        let t = para.trim_matches('\n');
        if t.is_empty() {
            continue;
        }
        blocks.push(Block {
            kind: BlockKind::P,
            spans: alloc::vec![Span { text: t.to_string(), ..Default::default() }],
            marker: None,
            style: BlockStyle::default(),
        });
    }
    if blocks.is_empty() {
        blocks.push(Block {
            kind: BlockKind::P,
            spans: alloc::vec![Span { text: "(empty file)".to_string(), ..Default::default() }],
            marker: None,
            style: BlockStyle::default(),
        });
    }
    Doc { title: title.to_string(), blocks, web: false, bg: None, fg: None, border: None, redirect: None }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// ------------------------------------------------------------------ addresses

#[derive(Clone, Debug, PartialEq, Eq)]
enum Nav {
    Home,
    About,
    History,
    Bookmarks,
    Local(String),
    Web(String),
    Search(String),
    NotFound(String),
}

fn is_windows_path(s: &str) -> bool {
    s.len() >= 3 && s.as_bytes()[1] == b':' && (s.as_bytes()[2] == b'\\' || s.as_bytes()[2] == b'/')
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hexval(b[i + 1]), hexval(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~' | b'/' | b':' | b'?') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

fn classify(input: &str) -> Nav {
    let s = input.trim();
    if s.is_empty() {
        return Nav::Home;
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("loos://").or_else(|| lower.strip_prefix("about:")) {
        if let Some(q) = rest.strip_prefix("search?q=") {
            return Nav::Search(percent_decode(q));
        }
        return match rest.trim_matches('/') {
            "" | "home" | "start" | "welcome" | "blank" => Nav::Home,
            "about" | "loos" => Nav::About,
            "history" => Nav::History,
            "bookmarks" => Nav::Bookmarks,
            other if !other.is_empty() => Nav::NotFound(other.to_string()),
            _ => Nav::Home,
        };
    }
    if lower.starts_with("file://") {
        let rest = &s[7..];
        let p = percent_decode(rest.trim_start_matches("localhost"));
        let p = if p.is_empty() { "/".to_string() } else if p.starts_with('/') { p } else { format!("/{p}") };
        return Nav::Local(p);
    }
    if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || s.starts_with("~/") {
        let expanded = if let Some(rest) = s.strip_prefix("~/") { format!("/home/user/{rest}") } else { s.to_string() };
        return Nav::Local(normalize_path(&expanded, ""));
    }
    if is_windows_path(s) {
        return Nav::Local(loos_vfs::winpath::to_loos(s).unwrap_or_else(|| s.to_string()));
    }
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ws://")
        || lower.starts_with("wss://")
        || lower.starts_with("www.")
        || lower.starts_with("//")
    {
        let url = if lower.starts_with("//") { format!("https:{s}") } else { s.to_string() };
        return Nav::Web(url);
    }
    if !s.contains(' ') && s.contains('.') && looks_like_host(s) {
        return Nav::Web(format!("https://{s}"));
    }
    Nav::Search(s.to_string())
}

fn looks_like_host(s: &str) -> bool {
    let host = s.split('/').next().unwrap_or(s);
    !host.is_empty()
        && host.contains('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
}

fn nav_url(nav: &Nav) -> String {
    match nav {
        Nav::Home => HOME.to_string(),
        Nav::About => "loos://about".to_string(),
        Nav::History => "loos://history".to_string(),
        Nav::Bookmarks => "loos://bookmarks".to_string(),
        Nav::Local(p) => p.clone(),
        Nav::Web(u) => u.clone(),
        Nav::Search(q) => format!("loos://search?q={}", percent_encode(q)),
        Nav::NotFound(n) => format!("loos://{n}"),
    }
}

// ------------------------------------------------------------------ browser state

#[derive(Default)]
struct Tab {
    url: String,
    title: String,
    doc: Doc,
    hist: Vec<String>,
    pos: usize,
    scroll: i32,
    total_h: i32,
    status: String,
    ok: bool,
}

#[derive(Clone, Debug, Default)]
struct Bookmark {
    url: String,
    title: String,
}

/// A clickable link rectangle recorded during the last paint.
#[derive(Clone, Debug)]
struct LinkHit {
    rect: Rect,
    url: String,
}

/// Browser state, owned by the shell through an opaque pointer.
pub struct Browser {
    be: BeHost,
    tabs: Vec<Tab>,
    active: usize,
    addr: [u8; 512],
    caret: i32,
    bookmarks: Vec<Bookmark>,
    history: Vec<Bookmark>,
    links: Vec<LinkHit>,
    status: String,
    blur_next: bool,
}

impl Browser {
    fn new(be: *const c_void) -> Self {
        let mut b = Browser {
            be: BeHost(be),
            tabs: Vec::new(),
            active: 0,
            addr: [0; 512],
            caret: 0,
            bookmarks: Vec::new(),
            history: Vec::new(),
            links: Vec::new(),
            status: "Local-first. Under ./loos qemu, web pages load through the host bridge.".to_string(),
            blur_next: false,
        };
        b.load_state();
        b.tabs.push(Tab::default());
        b.navigate_to(HOME);
        b
    }

    fn valid_backend(&self) -> bool {
        !self.be.0.is_null()
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        if !self.valid_backend() {
            return None;
        }
        Host::read(&self.be, path)
    }

    fn write_file(&self, path: &str, data: &str) -> bool {
        if !self.valid_backend() {
            return false;
        }
        Host::write(&self.be, path, data.as_bytes())
    }

    fn load_state(&mut self) {
        if !self.valid_backend() {
            self.default_bookmarks();
            return;
        }
        if let Some(data) = self.read_file(BOOKMARK_FILE) {
            self.bookmarks = parse_tsv(&String::from_utf8_lossy(&data));
        }
        if let Some(data) = self.read_file(HISTORY_FILE) {
            self.history = parse_tsv(&String::from_utf8_lossy(&data));
        }
        if self.bookmarks.is_empty() {
            self.default_bookmarks();
        }
        self.history.truncate(MAX_HISTORY);
    }

    fn default_bookmarks(&mut self) {
        self.bookmarks = alloc::vec![
            Bookmark { url: HOME.to_string(), title: "LoOS Start".to_string() },
            Bookmark { url: "/home/user/Web/index.html".to_string(), title: "LoOS Local Site".to_string() },
            Bookmark { url: "/home/user/Web/guide.html".to_string(), title: "Browser Guide".to_string() },
        ];
    }

    fn save_state(&self) {
        if !self.valid_backend() {
            return;
        }
        let _ = Host::mkdir(&self.be, "/home/user/.config");
        let _ = Host::mkdir(&self.be, "/home/user/.config/loos-browser");
        self.write_file(BOOKMARK_FILE, &to_tsv(&self.bookmarks));
        self.write_file(HISTORY_FILE, &to_tsv(&self.history));
    }

    fn addr_str(&self) -> String {
        let len = self.addr.iter().position(|&b| b == 0).unwrap_or(self.addr.len());
        String::from_utf8_lossy(&self.addr[..len]).into_owned()
    }

    fn set_addr(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let mut n = bytes.len().min(self.addr.len() - 1);
        while n > 0 && !s.is_char_boundary(n) {
            n -= 1;
        }
        self.addr[..n].copy_from_slice(&bytes[..n]);
        self.addr[n] = 0;
        self.caret = 0;
    }

    fn current(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn current_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    fn navigate_to(&mut self, input: &str) -> bool {
        let nav = classify(input);
        let loaded = self.load(&nav);
        let url = loaded.url.clone().unwrap_or_else(|| nav_url(&nav));
        {
            let tab = self.current_mut();
            if tab.pos + 1 < tab.hist.len() {
                tab.hist.truncate(tab.pos + 1);
            }
            if tab.hist.last() != Some(&url) {
                tab.hist.push(url.clone());
            }
            if tab.hist.len() > 64 {
                tab.hist.remove(0);
            }
            tab.pos = tab.hist.len().saturating_sub(1);
            tab.url = url.clone();
            tab.title = loaded.title.clone();
            tab.doc = loaded.doc;
            tab.scroll = 0;
            tab.total_h = 0;
            tab.status = loaded.status;
            tab.ok = loaded.ok;
        }
        self.set_addr(&url);
        self.blur_next = true;
        self.links.clear();
        self.status = self.current().status.clone();
        self.log_history(&url, &loaded.title);
        loaded.ok
    }

    fn log_history(&mut self, url: &str, title: &str) {
        if url == HOME || url == "loos://history" || url == "loos://bookmarks" || url == "loos://about" {
            return;
        }
        self.history.retain(|h| h.url != url);
        self.history.insert(0, Bookmark { url: url.to_string(), title: title.to_string() });
        self.history.truncate(MAX_HISTORY);
        self.save_state();
    }

    fn reload(&mut self) {
        let url = self.current().url.clone();
        if url.is_empty() {
            self.navigate_to(HOME);
            return;
        }
        let nav = classify(&url);
        let loaded = self.load(&nav);
        let tab = self.current_mut();
        tab.doc = loaded.doc;
        tab.title = loaded.title;
        tab.status = loaded.status;
        tab.ok = loaded.ok;
        tab.scroll = 0;
        self.status = self.current().status.clone();
        self.links.clear();
    }

    fn back(&mut self) -> bool {
        let tab = self.current();
        if tab.pos == 0 {
            return false;
        }
        let pos = tab.pos - 1;
        let url = tab.hist[pos].clone();
        self.restore(url, pos)
    }

    fn forward(&mut self) -> bool {
        let tab = self.current();
        if tab.pos + 1 >= tab.hist.len() {
            return false;
        }
        let pos = tab.pos + 1;
        let url = tab.hist[pos].clone();
        self.restore(url, pos)
    }

    fn restore(&mut self, url: String, pos: usize) -> bool {
        let nav = classify(&url);
        let loaded = self.load(&nav);
        let tab = self.current_mut();
        tab.pos = pos;
        tab.url = url.clone();
        tab.title = loaded.title;
        tab.doc = loaded.doc;
        tab.scroll = 0;
        tab.total_h = 0;
        tab.status = loaded.status;
        tab.ok = loaded.ok;
        self.set_addr(&url);
        self.blur_next = true;
        self.status = self.current().status.clone();
        self.links.clear();
        loaded.ok
    }

    fn new_tab(&mut self) {
        if self.tabs.len() >= MAX_TABS {
            self.status = format!("At most {MAX_TABS} tabs (basics build).");
            return;
        }
        self.tabs.push(Tab::default());
        self.active = self.tabs.len() - 1;
        self.navigate_to(HOME);
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::default());
            self.active = 0;
            self.navigate_to(HOME);
            return;
        }
        if self.active > idx || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        let url = self.current().url.clone();
        self.set_addr(&url);
        self.blur_next = true;
        self.status = self.current().status.clone();
        self.links.clear();
    }

    fn select_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() && idx != self.active {
            self.active = idx;
            let url = self.current().url.clone();
            self.set_addr(&url);
            self.blur_next = true;
            self.status = self.current().status.clone();
            self.links.clear();
        }
    }

    fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    fn toggle_bookmark(&mut self) {
        let url = self.current().url.clone();
        if url.is_empty() {
            return;
        }
        if self.is_bookmarked(&url) {
            self.bookmarks.retain(|b| b.url != url);
            self.status = "Bookmark removed.".to_string();
        } else {
            self.bookmarks.push(Bookmark { url: url.clone(), title: self.current().title.clone() });
            self.status = "Bookmarked.".to_string();
        }
        self.save_state();
    }

    fn load(&self, nav: &Nav) -> Loaded {
        match nav {
            Nav::Home => Loaded::page(self.home_doc()),
            Nav::About => Loaded::page(parse_html(ABOUT_HTML, "loos://about")),
            Nav::History => Loaded::page(self.history_doc()),
            Nav::Bookmarks => Loaded::page(self.bookmarks_doc()),
            Nav::Search(q) => Loaded::page(self.search_doc(q)),
            Nav::NotFound(name) => {
                let url = format!("loos://{name}");
                let doc = parse_html(
                    &format!(
                        "<h1>No such LoOS page</h1><p><b>{}</b> is not a built-in page.</p>\
                         <ul><li><a href=\"{HOME}\">Start page</a></li>\
                         <li><a href=\"loos://bookmarks\">Bookmarks</a></li>\
                         <li><a href=\"loos://about\">About this browser</a></li></ul>",
                        html_escape(&url)
                    ),
                    "loos://home",
                );
                Loaded { title: "Page not found".to_string(), doc, ok: false, status: format!("No such page: {url}"), url: None }
            }
            Nav::Web(url) => self.load_web(url),
            Nav::Local(path) => self.load_local(path),
        }
    }

    fn web_get(&self, url: &str) -> Option<(Vec<u8>, String)> {
        let mut buf = alloc::vec![0u8; WEB_COMPRESSED_MAX];
        let mut final_url = [0u8; 2048];
        let (mut n, mut raw) = (0usize, 0usize);
        // SAFETY: url is NUL-terminated; buf and final_url have their stated capacities; web.c writes within them.
        let ok = unsafe {
            loos_web_fetch(
                cs(url).as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut n,
                &mut raw,
                final_url.as_mut_ptr().cast(),
                final_url.len(),
            )
        };
        if !ok || n == 0 || n > buf.len() || raw == 0 {
            return None;
        }
        buf.truncate(n);
        let data = miniz_oxide::inflate::decompress_to_vec_with_limit(&buf, raw.min(WEB_RAW_MAX) + 1).ok()?;
        let flen = final_url.iter().position(|&b| b == 0).unwrap_or(0);
        let final_s = String::from_utf8_lossy(&final_url[..flen]).into_owned();
        Some((data, if final_s.is_empty() { url.to_string() } else { final_s }))
    }

    fn load_web(&self, url: &str) -> Loaded {
        self.load_web_depth(url, 0)
    }

    fn load_web_depth(&self, url: &str, depth: u32) -> Loaded {
        let Some((data, final_url)) = self.web_get(url) else {
            let doc = parse_html(&format!(
                "<h1>Page could not be loaded</h1>\
                 <p>You asked for <b>{}</b>.</p>\
                 <p><b>Reason:</b> {}</p>\
                 <p>LoOS loads pages over its own network stack (DHCP, DNS, TCP and TLS 1.3 with a built-in list of trusted certificate authorities). In a virtual machine use a network adapter of type <b>e1000</b> (NAT). Under <b>./loos qemu</b> the development web bridge is used when the adapter has no address.</p>\
                 <ul><li><a href=\"{HOME}\">Start page</a></li>\
                 <li><a href=\"loos://about\">How the web bridge works</a></li>\
                 <li><a href=\"loos://bookmarks\">Bookmarks</a></li></ul>",
                html_escape(url),
                html_escape(&web_error())
            ), HOME);
            return Loaded { title: "Page could not be loaded".to_string(), doc, ok: false, status: "Web fetch failed (bridge or request error)".to_string(), url: None };
        };
        let text = String::from_utf8_lossy(&data);
        let head = text.get(..text.len().min(4096)).unwrap_or("").to_ascii_lowercase();
        let looks_html = head.contains("<!doctype html") || head.contains("<html") || head.contains("<head") || head.contains("<body") || head.contains("<div") || head.contains("<p");
        let doc = if looks_html {
            let html = self.inline_stylesheets(&text, &final_url);
            parse_html_web(&html, &final_url)
        } else {
            let name = final_url.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&final_url);
            plain_doc(&text, name)
        };
        if let Some(r) = &doc.redirect {
            if depth < 3 && !r.is_empty() && *r != final_url {
                return self.load_web_depth(r, depth + 1);
            }
        }
        let kb = (data.len() + 1023) / 1024;
        Loaded {
            title: doc.title.clone(),
            doc,
            ok: true,
            status: format!("{final_url}  -  {kb} KB  -  loaded over the network"),
            url: Some(final_url),
        }
    }

    fn inline_stylesheets(&self, html: &str, base: &str) -> String {
        let lower = html.to_ascii_lowercase();
        let mut extra = String::new();
        let mut from = 0usize;
        let mut fetched = 0;
        while fetched < 4 {
            let Some(rel) = lower[from..].find("<link") else { break };
            let start = from + rel;
            let Some(gt) = find_tag_end(html, start) else { break };
            let tag = &html[start + 5..gt];
            from = gt + 1;
            let relv = attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
            if !relv.contains("stylesheet") {
                continue;
            }
            let Some(href) = attr(tag, "href") else { continue };
            let target = resolve_href(base, &href);
            if !(target.starts_with("http://") || target.starts_with("https://")) {
                continue;
            }
            let Some((css, _)) = self.web_get(&target) else { continue };
            if css.len() > 512 * 1024 {
                continue;
            }
            let css = String::from_utf8_lossy(&css).replace("</style", "<\\/style");
            extra.push_str("<style>");
            extra.push_str(&css);
            extra.push_str("</style>");
            fetched += 1;
        }
        if extra.is_empty() {
            return html.to_string();
        }
        if let Some(pos) = lower.find("</head>") {
            let mut out = String::with_capacity(html.len() + extra.len());
            out.push_str(&html[..pos]);
            out.push_str(&extra);
            out.push_str(&html[pos..]);
            out
        } else {
            format!("{extra}{html}")
        }
    }

    fn load_local(&self, path: &str) -> Loaded {
        if !self.valid_backend() {
            return self.error_page(path, "No filesystem backend is attached.");
        }
        let Some(data) = self.read_file(path) else {
            let doc = parse_html(
                &format!(
                    "<h1>Page not found</h1><p>No readable file at <b>{}</b>.</p>\
                     <p>Try one of the sample pages:</p><ul>\
                     <li><a href=\"/home/user/Web/index.html\">LoOS local site</a></li>\
                     <li><a href=\"/home/user/Web/guide.html\">Browser guide</a></li>\
                     <li><a href=\"{HOME}\">Start page</a></li></ul>",
                    html_escape(path)
                ),
                HOME,
            );
            return Loaded { title: "Page not found".to_string(), doc, ok: false, status: format!("Cannot open {path}"), url: None };
        };
        let text = String::from_utf8_lossy(&data);
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        let doc = if lower.ends_with(".html") || lower.ends_with(".htm") || lower.ends_with(".xhtml") {
            parse_html(&text, path)
        } else {
            plain_doc(&text, name)
        };
        let size = data.len();
        Loaded {
            title: doc.title.clone(),
            doc,
            ok: true,
            status: format!("{path}  -  {size} bytes  -  local file"),
            url: None,
        }
    }

    fn error_page(&self, url: &str, message: &str) -> Loaded {
        let doc = parse_html(
            &format!(
                "<h1>Cannot open page</h1><p><b>{}</b></p><p>{}</p><p><a href=\"{HOME}\">Start page</a></p>",
                html_escape(url),
                html_escape(message)
            ),
            HOME,
        );
        Loaded { title: "Cannot open page".to_string(), doc, ok: false, status: message.to_string(), url: None }
    }

    fn home_doc(&self) -> Doc {
        let mut bm = String::new();
        for b in self.bookmarks.iter().take(8) {
            bm.push_str(&format!("<li><a href=\"{}\">{}</a></li>", html_escape(&b.url), html_escape(&b.title)));
        }
        if bm.is_empty() {
            bm.push_str("<li><i>No bookmarks yet. Press Ctrl+D on a page.</i></li>");
        }
        let mut recent = String::new();
        for h in self.history.iter().take(5) {
            recent.push_str(&format!("<li><a href=\"{}\">{}</a></li>", html_escape(&h.url), html_escape(&h.title)));
        }
        if recent.is_empty() {
            recent.push_str("<li><i>Nothing yet.</i></li>");
        }
        parse_html(
            &format!(
                "<h1>LoOS Browser</h1>\
                 <p>A fast, local-first browser for LoOS. No accounts, no telemetry, no remote start page.</p>\
                 <h2>Local pages</h2>\
                 <ul>\
                 <li><a href=\"/home/user/Web/index.html\">LoOS local site</a> - a sample site on this disk</li>\
                 <li><a href=\"/home/user/Web/guide.html\">Browser guide</a> - shortcuts, tabs and bookmarks</li>\
                 <li><a href=\"/home/user/Web/privacy.html\">Privacy in this browser</a> - what leaves the machine: nothing</li>\
                 <li><a href=\"loos://about\">About LoOS Browser</a> - status and the Firefox plan</li>\
                 </ul>\
                 <h2>Bookmarks</h2><ul>{bm}</ul>\
                 <h2>Recently visited</h2><ul>{recent}</ul>\
                 <h2>Web addresses</h2>\
                 <p>Under <b>./loos qemu</b>, HTTP and HTTPS pages load through a development web bridge: the host fetches the page over HTTPS and hands it to the guest over COM2. LoOS itself has no socket code yet. Without the bridge, a web address shows a local explanation page and nothing is sent.</p>"
            ),
            HOME,
        )
    }

    fn history_doc(&self) -> Doc {
        let mut list = String::new();
        for h in self.history.iter().take(100) {
            list.push_str(&format!("<li><a href=\"{}\">{}</a></li>", html_escape(&h.url), html_escape(&h.title)));
        }
        if list.is_empty() {
            list.push_str("<li><i>History is empty.</i></li>");
        }
        parse_html(
            &format!(
                "<h1>History</h1><p>{} entries, stored locally in <b>/home/user/.config/loos-browser/history.tsv</b>. \
                 Nothing is synced.</p><ul>{list}</ul>",
                self.history.len()
            ),
            "loos://history",
        )
    }

    fn bookmarks_doc(&self) -> Doc {
        let mut list = String::new();
        for b in &self.bookmarks {
            list.push_str(&format!("<li><a href=\"{}\">{}</a></li>", html_escape(&b.url), html_escape(&b.title)));
        }
        if list.is_empty() {
            list.push_str("<li><i>No bookmarks yet. Press Ctrl+D on a page.</i></li>");
        }
        parse_html(
            &format!(
                "<h1>Bookmarks</h1><p>{} saved locally in <b>/home/user/.config/loos-browser/bookmarks.tsv</b>.</p><ul>{list}</ul>",
                self.bookmarks.len()
            ),
            "loos://bookmarks",
        )
    }

    fn search_doc(&self, q: &str) -> Doc {
        let needle = q.to_ascii_lowercase();
        let mut results = String::new();
        if !needle.is_empty() {
            for b in &self.bookmarks {
                if b.title.to_ascii_lowercase().contains(&needle) || b.url.to_ascii_lowercase().contains(&needle) {
                    results.push_str(&format!(
                        "<li><a href=\"{}\">{}</a> <i>bookmark</i></li>",
                        html_escape(&b.url),
                        html_escape(&b.title)
                    ));
                }
            }
            if self.valid_backend() {
                if let Some(entries) = Host::list(&self.be, "/home/user/Web") {
                    for (name, is_dir, _) in entries {
                        if !is_dir && name.to_ascii_lowercase().contains(&needle) {
                            let path = format!("/home/user/Web/{name}");
                            results.push_str(&format!(
                                "<li><a href=\"{}\">{}</a> <i>local file</i></li>",
                                html_escape(&path),
                                html_escape(&name)
                            ));
                        }
                    }
                }
            }
        }
        if results.is_empty() {
            results.push_str("<li><i>No local matches.</i></li>");
        }
        parse_html(
            &format!(
                "<h1>Local search: {}</h1>\
                 <p>LoOS Browser searches bookmarks and <b>/home/user/Web</b> on this computer. It has no remote search provider and sends nothing.</p>\
                 <ul>{results}</ul>\
                 <p><a href=\"{HOME}\">Start page</a></p>",
                html_escape(q)
            ),
            "loos://home",
        )
    }

    // -------------------------------------------------------------- painting

    fn frame(&mut self, ui: &mut Ui, w: i32, h: i32) {
        let t = ui.t;
        ui.fill(Rect::new(0, 0, w, h), t.surface);

        // Global shortcuts run before the address field so a focused field cannot eat them.
        if ui.take_key('l' as u32, MOD_CTRL) {
            let id = Ui::textfield_id(PLACEHOLDER, addr_rect(w));
            self.set_addr("");                       /* select-all is not in the toolkit yet: clear like a new tab */
            ui.focus(id);
        }
        if ui.take_key('t' as u32, MOD_CTRL) {
            self.new_tab();
        }
        if ui.take_key('w' as u32, MOD_CTRL) {
            self.close_tab(self.active);
        }
        if ui.take_key('r' as u32, MOD_CTRL) {
            self.reload();
        }
        if ui.take_key('d' as u32, MOD_CTRL) {
            self.toggle_bookmark();
        }
        if ui.take_key(0x102, MOD_ALT) {
            self.back();
        }
        if ui.take_key(0x103, MOD_ALT) {
            self.forward();
        }
        if ui.take_key(27, 0) {
            ui.blur();
        }
        if self.blur_next {
            self.blur_next = false;
            self.caret = 0;
            ui.blur();
        }

        let action = self.draw_tabs(ui, w);
        match action {
            TabAction::Select(i) => self.select_tab(i),
            TabAction::Close(i) => self.close_tab(i),
            TabAction::New => self.new_tab(),
            TabAction::None => {}
        }

        self.draw_toolbar(ui, w);

        let area = Rect::new(0, TAB_H + BAR_H, w, (h - TAB_H - BAR_H - STATUS_H).max(0));

        // Page clicks use the link map from the previous paint; one frame of latency is invisible.
        let (mx, my, pressed, wheel) = ui.pointer();
        if pressed && contains(area, mx, my) {
            let hit = self.links.iter().find(|l| contains(l.rect, mx, my)).map(|l| l.url.clone());
            ui.blur();
            if let Some(url) = hit {
                self.navigate_to(&url);
            }
        }

        // Scrolling and page keys.
        if contains(area, mx, my) && wheel != 0 {
            let tab = self.current_mut();
            tab.scroll = (tab.scroll - wheel * 54).max(0);
        }
        while let Some((k, _mods, is_char)) = ui.input() {
            if is_char {
                continue;
            }
            match k {
                0x106 => self.scroll_by(-(area.h - 48)),
                0x107 => self.scroll_by(area.h - 48),
                0x100 => self.scroll_by(-60),
                0x101 => self.scroll_by(60),
                0x104 => self.set_scroll(0),
                0x105 => {
                    let max = (self.current().total_h - area.h + 24).max(0);
                    self.set_scroll(max)
                }
                _ => {}
            }
        }

        // Clamp against the height measured on the previous paint.
        let max_prev = (self.current().total_h - area.h + 24).max(0);
        if self.current().scroll > max_prev {
            self.current_mut().scroll = max_prev;
        }

        let hover = self.links.iter().find(|l| contains(l.rect, mx, my)).map(|l| l.url.clone());
        let scroll = self.current().scroll;
        let page_bg = {
            let d = &self.current().doc;
            d.bg.unwrap_or(if d.web { 0xffff_ffff } else { t.surface })
        };
        ui.fill(area, page_bg);
        let (total, links) = {
            let doc = &self.current().doc;
            render_doc(ui, doc, area, scroll, hover.as_deref())
        };
        self.current_mut().total_h = total;
        self.links = links;

        // Scrollbar.
        if total > area.h {
            let web = self.current().doc.web;
            let track = Rect::new(area.x + area.w - 9, area.y + 6, 4, area.h - 12);
            ui.round(track, 2, if web { 0xfff1f3f4 } else { t.surface_hi });
            let max = (total - area.h).max(1);
            let th = ((track.h as i64 * area.h as i64) / total as i64).max(24) as i32;
            let ty = track.y + (track.h - th) * self.current().scroll / max;
            ui.round(Rect::new(track.x, ty, track.w, th), 2, if web { 0xffc4c7c5 } else { t.border });
        }

        self.draw_status(ui, w, h, hover.as_deref());
    }

    fn scroll_by(&mut self, d: i32) {
        let tab = self.current_mut();
        tab.scroll = (tab.scroll + d).max(0);
    }

    fn set_scroll(&mut self, v: i32) {
        self.current_mut().scroll = v.max(0);
    }

    fn draw_tabs(&self, ui: &Ui, w: i32) -> TabAction {
        let t = ui.t;
        ui.fill(Rect::new(0, 0, w, TAB_H), t.surface_hi);
        ui.fill(Rect::new(0, TAB_H - 1, w, 1), t.border);
        let (mx, my, pressed, _) = ui.pointer();
        let n = self.tabs.len().max(1) as i32;
        let avail = (w - 16 - 30).max(80);
        let tw = (avail / n).clamp(72, 190);
        let mut x = 8;
        let mut action = TabAction::None;
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let r = Rect::new(x, 4, tw - 4, TAB_H - 8);
            let close = Rect::new(r.x + r.w - 20, r.y + 4, 16, 16);
            if active {
                ui.round(r, 6, t.surface);
                ui.fill(Rect::new(r.x + 6, r.y + 2, r.w - 12, 2), t.accent);
            } else if contains(r, mx, my) {
                ui.round(r, 6, t.surface);
            }
            let title = if tab.title.is_empty() { "New Tab" } else { tab.title.as_str() };
            let max = (r.w - 34).max(10);
            let fit = Ui::fit(title, max);
            let mut cut = fit.min(title.len());
            while cut > 0 && !title.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut label = title[..cut].to_string();
            if cut < title.len() && !label.ends_with('\u{2026}') {
                label.push('\u{2026}');
            }
            ui.text(r.x + 9, r.y + 5, &label, if active { t.text } else { t.text_dim });
            ui.text(close.x + 3, close.y, "x", if contains(close, mx, my) { t.danger } else { t.text_dim });
            if pressed && contains(r, mx, my) {
                action = if contains(close, mx, my) { TabAction::Close(i) } else { TabAction::Select(i) };
            }
            x += tw;
        }
        let plus = Rect::new(x + 2, 6, 24, TAB_H - 12);
        ui.round(plus, 4, if contains(plus, mx, my) { t.border } else { t.surface_hi });
        ui.text(plus.x + 8, plus.y + 2, "+", t.text_dim);
        if pressed && contains(plus, mx, my) {
            action = TabAction::New;
        }
        action
    }

    fn draw_toolbar(&mut self, ui: &mut Ui, w: i32) {
        let t = ui.t;
        let y = TAB_H;
        ui.fill(Rect::new(0, y, w, BAR_H), t.surface);
        ui.fill(Rect::new(0, y + BAR_H - 1, w, 1), t.border);
        let by = y + 6;
        let bh = 28;
        let mut x = 8;
        if ui.button(Rect::new(x, by, 56, bh), "Back", false) {
            self.back();
        }
        x += 62;
        if ui.button(Rect::new(x, by, 72, bh), "Forward", false) {
            self.forward();
        }
        x += 78;
        if ui.button(Rect::new(x, by, 66, bh), "Reload", false) {
            self.reload();
        }
        x += 72;
        if ui.button(Rect::new(x, by, 54, bh), "Home", false) {
            self.navigate_to(HOME);
        }

        let addr = addr_rect(w);
        let id = Ui::textfield_id(PLACEHOLDER, addr);
        let entered = ui.textfield(addr, &mut self.addr, &mut self.caret, PLACEHOLDER);
        if entered {
            let s = self.addr_str();
            self.navigate_to(&s);
        }
        if !ui.focused(id) {
            // Keep the field in step with navigation from links, tabs and buttons.
            let url = self.current().url.clone();
            if self.addr_str() != url && !url.is_empty() {
                self.set_addr(&url);
            }
        }
        let go = Rect::new(addr.x + addr.w + 6, by, 48, bh);
        if ui.button(go, "Go", true) {
            let s = self.addr_str();
            self.navigate_to(&s);
        }
        let bm = Rect::new(go.x + go.w + 6, by, 84, bh);
        if ui.button(bm, "Bookmark", false) {
            self.toggle_bookmark();
        }
    }

    fn draw_status(&self, ui: &Ui, w: i32, h: i32, hover: Option<&str>) {
        let t = ui.t;
        let y = h - STATUS_H;
        ui.fill(Rect::new(0, y, w, STATUS_H), t.surface_hi);
        ui.fill(Rect::new(0, y, w, 1), t.border);
        let web = self.current().url.starts_with("http://") || self.current().url.starts_with("https://");
        let right = if web {
            if self.current().ok { "Web | online" } else { "Web | offline" }
        } else if self.is_bookmarked(&self.current().url) {
            "Bookmarked | Local"
        } else {
            "Local only"
        };
        let rw = Ui::width(right);
        let left = if let Some(u) = hover {
            format!("link: {u}")
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            self.current().status.clone()
        };
        let max = (w - rw - 24).max(40);
        let fit = Ui::fit(&left, max);
        let mut cut = fit.min(left.len());
        while cut > 0 && !left.is_char_boundary(cut) {
            cut -= 1;
        }
        ui.text(8, y + 3, &left[..cut], t.text_dim);
        ui.text(w - 8 - rw, y + 3, right, if self.current().ok { t.text_dim } else { t.danger });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabAction {
    None,
    Select(usize),
    Close(usize),
    New,
}

struct Loaded {
    title: String,
    doc: Doc,
    ok: bool,
    status: String,
    /// Final URL after redirects (web only).
    url: Option<String>,
}

impl Loaded {
    fn page(doc: Doc) -> Self {
        let status = format!("{}  -  {} blocks", doc.title, doc.blocks.len());
        Loaded { title: doc.title.clone(), doc, ok: true, status, url: None }
    }
}

fn addr_rect(w: i32) -> Rect {
    // Back/Forward/Reload/Home take 274 px; Go + Bookmark + margins take 146 px.
    Rect::new(274, TAB_H + 6, (w - 426).max(120), 28)
}

fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

// ------------------------------------------------------------------ persistence

fn parse_tsv(text: &str) -> Vec<Bookmark> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((url, title)) = line.split_once('\t') else { continue };
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        out.push(Bookmark { url: url.to_string(), title: title.trim().to_string() });
    }
    out
}

fn to_tsv(items: &[Bookmark]) -> String {
    let mut s = String::new();
    for b in items {
        let url = b.url.replace('\t', " ").replace('\n', " ").replace('\r', " ");
        let title = b.title.replace('\t', " ").replace('\n', " ").replace('\r', " ");
        s.push_str(&url);
        s.push('\t');
        s.push_str(&title);
        s.push('\n');
    }
    s
}

// ------------------------------------------------------------------ rendering

struct Colors {
    text: u32,
    link: u32,
    border: u32,
    code: u32,
    dim: u32,
}

fn render_doc(ui: &Ui, doc: &Doc, area: Rect, scroll: i32, hover: Option<&str>) -> (i32, Vec<LinkHit>) {
    let mut links = Vec::new();
    let web = doc.web;
    let dark_page = web && doc.bg.map(|c| ((c >> 16 & 0xff) * 30 + (c >> 8 & 0xff) * 59 + (c & 0xff) * 11) / 100 < 110).unwrap_or(false);
    let colors = Colors {
        text: doc.fg.unwrap_or(if dark_page { 0xffe8_eaed } else if web { 0xff20_2124 } else { ui.t.text }),
        link: if dark_page { 0xff8a_b4f8 } else if web { 0xff1a_73e8 } else { ui.t.accent },
        border: doc.border.unwrap_or(if dark_page { 0xff5f_6368 } else if web { 0xffda_dce0 } else { ui.t.border }),
        code: if dark_page { 0xfff4_8fb1 } else if web { 0xffb8_066c } else { ui.t.accent },
        dim: if dark_page { 0xffbd_c1c5 } else if web { 0xff5f_6368 } else { ui.t.text_dim },
    };
    let max_content = if web { 1100 } else { MAX_CONTENT };
    let content_w = (area.w - 2 * MARGIN).min(max_content).max(160);
    let x0 = area.x + (area.w - content_w) / 2;
    let mut y = area.y + PAGE_TOP - scroll;
    let mut first = true;
    for block in &doc.blocks {
        let (db, da) = margins(block.kind);
        let before = block.style.margin_before.unwrap_or(db);
        let after = block.style.margin_after.unwrap_or(da);
        if !first {
            y += before;
        }
        let top = y;
        y = draw_block(ui, block, x0, content_w, y, area, &mut links, hover, &colors, web);
        if let Some(b) = block.style.border {
            if y > top && y >= area.y && top <= area.y + area.h {
                let r = Rect::new(x0 + block.style.indent, top, (content_w - block.style.indent).max(24), y - top);
                ui.frame(r, b);
            }
        }
        y += after;
        first = false;
    }
    let total = (y + scroll - area.y) + 24;
    (total.max(0), links)
}

fn margins(kind: BlockKind) -> (i32, i32) {
    match kind {
        BlockKind::H1 => (4, 14),
        BlockKind::H2 => (14, 8),
        BlockKind::H3 => (10, 6),
        BlockKind::P => (0, 10),
        BlockKind::Li => (0, 3),
        BlockKind::Pre => (8, 12),
        BlockKind::Quote => (8, 12),
        BlockKind::Rule => (8, 12),
    }
}

fn spans_width(spans: &[Span], size: i32, bold: bool) -> i32 {
    let mut w = 0;
    for s in spans {
        if s.text.contains('\n') {
            return i32::MAX;
        }
        w += Ui::width_sized(&s.text, s.size.unwrap_or(size), bold || s.bold);
    }
    w
}

#[allow(clippy::too_many_arguments)]
fn draw_block(
    ui: &Ui,
    block: &Block,
    x0: i32,
    width: i32,
    y: i32,
    area: Rect,
    links: &mut Vec<LinkHit>,
    hover: Option<&str>,
    colors: &Colors,
    web: bool,
) -> i32 {
    let t = ui.t;
    let st = &block.style;
    let indent = st.indent.clamp(0, width / 2);
    let x = x0 + indent;
    let w = (width - indent).max(40);
    let color = st.color.unwrap_or(colors.text);
    let size = st.size.unwrap_or(14);
    let bold = st.bold.unwrap_or(matches!(block.kind, BlockKind::H1 | BlockKind::H2 | BlockKind::H3));
    let mut tx = x;
    let mut tw = w;
    if let Some(align) = st.align {
        let text_w = spans_width(&block.spans, size, bold);
        if text_w > 0 && text_w < w {
            let shift = match align {
                Align::Center => (w - text_w) / 2,
                Align::Right => w - text_w,
                Align::Left => 0,
            };
            tx = x + shift;
            tw = w - shift;
        }
    }
    match block.kind {
        BlockKind::Rule => {
            if y + 1 >= area.y && y <= area.y + area.h {
                ui.fill(Rect::new(x, y, w, 1), st.border.unwrap_or(colors.border));
            }
            y + 1
        }
        BlockKind::Pre => {
            let mut text = String::new();
            for s in &block.spans {
                text.push_str(&s.text);
            }
            let lines: Vec<&str> = text.split('\n').collect();
            let lh = (size + 5).max(18);
            let height = (lines.len() as i32 * lh + 16).max(24);
            if y + height >= area.y && y <= area.y + area.h {
                let bg = st.bg.unwrap_or(if web { 0xfff6_f8fa } else { t.surface_hi });
                ui.round(Rect::new(x, y, w, height), 4, bg);
                for (i, line) in lines.iter().enumerate() {
                    ui.text_sized(x + 10, y + 8 + i as i32 * lh, line, color, size, false);
                }
            }
            y + height
        }
        BlockKind::Quote => {
            let lh = (size + 6).max(20);
            let end = draw_runs(ui, &block.spans, x + 16, y, w - 16, size, lh, color, bold, area, links, hover, colors, st.bg, st.underline);
            if end > y && y + 4 >= area.y && y <= area.y + area.h {
                ui.fill(Rect::new(x, y + 2, 3, (end - y - 2).max(4)), colors.link);
            }
            end
        }
        BlockKind::H1 | BlockKind::H2 | BlockKind::H3 => {
            let lh = (size + 6).max(22);
            draw_runs(ui, &block.spans, tx, y, tw, size, lh, color, bold, area, links, hover, colors, st.bg, st.underline)
        }
        BlockKind::Li => {
            let marker = block.marker.as_deref().unwrap_or("- ");
            let lh = (size + 6).max(20);
            if y + lh >= area.y && y <= area.y + area.h {
                ui.text_sized(x, y + 2, marker, colors.dim, size, false);
            }
            draw_runs(ui, &block.spans, x + 20, y, w - 20, size, lh, color, bold, area, links, hover, colors, st.bg, st.underline)
        }
        BlockKind::P => {
            let lh = (size + 6).max(20);
            draw_runs(ui, &block.spans, tx, y, tw, size, lh, color, bold, area, links, hover, colors, st.bg, st.underline)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_runs(
    ui: &Ui,
    spans: &[Span],
    x_start: i32,
    y_start: i32,
    width: i32,
    size: i32,
    line_h: i32,
    color: u32,
    bold_base: bool,
    area: Rect,
    links: &mut Vec<LinkHit>,
    hover: Option<&str>,
    colors: &Colors,
    bg: Option<u32>,
    underline_base: bool,
) -> i32 {
    let right = x_start + width.max(40);
    let mut x = x_start;
    let mut y = y_start;
    let mut any = false;
    let mut pending_space = false;
    let mut bg_y = i32::MIN;
    let mut prev_link: Option<String> = None;
    for span in spans {
        let c = span.color.unwrap_or(if span.link.is_some() { colors.link } else if span.code { colors.code } else { color });
        let bold = bold_base || span.bold;
        let sz = span.size.unwrap_or(size);
        let space_w = Ui::width_sized(" ", sz, false).max(2);
        if prev_link.is_some() && span.link.is_some() && prev_link.as_deref() != span.link.as_deref() && x > x_start {
            x += space_w;
        }
        if pending_space && x > x_start && !span.text.starts_with(' ') {
            x += space_w;
        }
        pending_space = false;
        if span.text.ends_with(' ') {
            pending_space = true;
        }
        if let Some(b) = bg {
            if bg_y != y && y + line_h >= area.y && y <= area.y + area.h {
                ui.fill(Rect::new(x_start, y, width, line_h), b);
                bg_y = y;
            }
        }
        for (li, line) in span.text.split('\n').enumerate() {
            if li > 0 {
                y += line_h;
                x = x_start;
                if let Some(b) = bg {
                    if bg_y != y && y + line_h >= area.y && y <= area.y + area.h {
                        ui.fill(Rect::new(x_start, y, width, line_h), b);
                        bg_y = y;
                    }
                }
            }
            if line.is_empty() {
                any = true;
                continue;
            }
            for (wi, word) in line.split(' ').enumerate() {
                if word.is_empty() {
                    continue;
                }
                if wi > 0 && x > x_start {
                    x += space_w;
                }
                let w = Ui::width_sized(word, sz, bold);
                if x > x_start && x + w > right {
                    y += line_h;
                    x = x_start;
                    if let Some(b) = bg {
                        if bg_y != y && y + line_h >= area.y && y <= area.y + area.h {
                            ui.fill(Rect::new(x_start, y, width, line_h), b);
                            bg_y = y;
                        }
                    }
                }
                if y + line_h >= area.y && y <= area.y + area.h {
                    ui.text_sized(x, y, word, c, sz, bold);
                    let underline = span.underline || underline_base || span.link.is_some();
                    if underline {
                        let thick = if hover == span.link.as_deref() && span.link.is_some() { 2 } else { 1 };
                        let uc = if span.link.is_some() { colors.link } else { c };
                        ui.fill(Rect::new(x, y + sz + 3, w, thick), uc);
                    }
                    if let Some(url) = &span.link {
                        let r = Rect::new(x, y, w.max(1), line_h);
                        if let Some(last) = links.last_mut() {
                            if last.url == *url && last.rect.y == r.y && last.rect.x + last.rect.w >= r.x - 6 {
                                last.rect.w = r.x + r.w - last.rect.x;
                            } else {
                                links.push(LinkHit { rect: r, url: url.clone() });
                            }
                        } else {
                            links.push(LinkHit { rect: r, url: url.clone() });
                        }
                    }
                }
                x += w;
                any = true;
            }
        }
        prev_link = span.link.clone();
    }
    if any {
        y + line_h
    } else {
        y
    }
}

const ABOUT_HTML: &str = "\
<h1>About LoOS Browser</h1>\
<p>Version 0.2 (real pages in QEMU). A native browser shell written in Rust for the LoOS desktop.</p>\
<h2>What works now</h2>\
<ul>\
<li>Tabs (Ctrl+T, Ctrl+W), back/forward (Alt+Left/Right) and reload (Ctrl+R).</li>\
<li>Live HTTP/HTTPS pages under ./loos qemu: the host bridge fetches them, the guest renders them.</li>\
<li>A CSS subset: class/id/tag selectors, display:none, colours, font size/weight, margins, padding, borders, backgrounds and underlines.</li>\
<li>HTML subset: headings, paragraphs, links, lists, bold, italic, inline code, quotes, rules, preformatted text, forms (as placeholders), image alt text and meta refresh.</li>\
<li>Bookmarks (Ctrl+D) and history, saved in /home/user/.config/loos-browser.</li>\
<li>Local files (file://, /path, Windows paths), loos:// pages and local search.</li>\
</ul>\
<h2>Privacy</h2>\
<p>The guest browser has no socket code. Under ./loos qemu, requests go to a host helper over COM2; the helper validates TLS, refuses file: and non-http(s) URLs, and only runs when you start the QEMU desktop. On real hardware this bridge is absent and the browser stays offline.</p>\
<h2>What is not here yet</h2>\
<ul>\
<li>JavaScript, the full CSS cascade (flex/grid/position), images, media and the full HTML standard.</li>\
<li>A native LoOS TCP/IP + TLS stack. The QEMU bridge is a development step, not the final network design.</li>\
<li>Gecko. See the Firefox/Gecko port plan in docs/BROWSER.md.</li>\
</ul>\
<h2>Full Firefox port (planned)</h2>\
<p>The long-term plan is to bring Gecko (Firefox) up on LoOS, reusing this shell for tabs, history, bookmarks, permissions and the address bar. It is staged: SpiderMonkey first, then Necko, layout/style, WebRender, and the browser chrome on the LoOS compositor. Nothing in this release claims to be Gecko.</p>\
<p><a href=\"loos://home\">Start page</a></p>";

// ------------------------------------------------------------------ C ABI

/// Create a browser. `be` is the shell backend; `url` may be NULL for the start page.
#[no_mangle]
pub extern "C" fn loos_browser_new(be: *const c_void, url: *const c_char) -> *mut Browser {
    let mut b = Box::new(Browser::new(be));
    if !url.is_null() {
        // SAFETY: caller passes a NUL-terminated URL.
        let s = unsafe { core::ffi::CStr::from_ptr(url) }.to_string_lossy().into_owned();
        if !s.is_empty() {
            b.navigate_to(&s);
        }
    }
    Box::into_raw(b)
}

/// Paint one frame.
#[no_mangle]
pub unsafe extern "C" fn loos_browser_frame(app: *mut c_void, u: *mut super::UiCtx, w: i32, h: i32) {
    // SAFETY: app is a Browser from loos_browser_new; u is live for this frame.
    let (b, mut ui) = unsafe { (&mut *(app as *mut Browser), Ui::new(u, w, h)) };
    b.frame(&mut ui, w, h);
}

/// Open a URL/path in an existing browser. Returns true when a real page loaded (web addresses
/// return false because this build is offline by design).
#[no_mangle]
pub unsafe extern "C" fn loos_browser_navigate_to(app: *mut c_void, url: *const c_char) -> bool {
    if app.is_null() || url.is_null() {
        return false;
    }
    // SAFETY: app is a Browser; url is NUL-terminated.
    let b = unsafe { &mut *(app as *mut Browser) };
    let s = unsafe { core::ffi::CStr::from_ptr(url) }.to_string_lossy().into_owned();
    b.navigate_to(&s)
}

/// Copy the active tab's URL (NUL-terminated, truncated) into `out`.
#[no_mangle]
pub unsafe extern "C" fn loos_browser_url(app: *const c_void, out: *mut c_char, cap: usize) -> bool {
    if app.is_null() || out.is_null() || cap == 0 {
        return false;
    }
    // SAFETY: app is a Browser; out has cap bytes.
    let b = unsafe { &*(app as *const Browser) };
    let url = b.current().url.clone();
    let bytes = url.as_bytes();
    let mut n = bytes.len().min(cap - 1);
    while n > 0 && !url.is_char_boundary(n) {
        n -= 1;
    }
    // SAFETY: n <= cap - 1 writable bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), n);
        *out.add(n) = 0;
    }
    true
}

/// Number of open tabs.
#[no_mangle]
pub unsafe extern "C" fn loos_browser_tab_count(app: *const c_void) -> i32 {
    if app.is_null() {
        return 0;
    }
    // SAFETY: app is a Browser.
    unsafe { (*(app as *const Browser)).tabs.len() as i32 }
}

/// Number of saved bookmarks.
#[no_mangle]
pub unsafe extern "C" fn loos_browser_bookmark_count(app: *const c_void) -> i32 {
    if app.is_null() {
        return 0;
    }
    // SAFETY: app is a Browser.
    unsafe { (*(app as *const Browser)).bookmarks.len() as i32 }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn browser() -> Box<Browser> {
        Box::new(Browser::new(core::ptr::null()))
    }

    #[test]
    fn classifies_addresses() {
        assert_eq!(classify(""), Nav::Home);
        assert_eq!(classify("loos://bookmarks"), Nav::Bookmarks);
        assert_eq!(classify("about:loos"), Nav::About);
        assert_eq!(classify("file:///home/user/Web/index.html"), Nav::Local("/home/user/Web/index.html".into()));
        assert_eq!(classify("/home/user/Web/a.html"), Nav::Local("/home/user/Web/a.html".into()));
        assert_eq!(classify("C:\\Users\\user\\a.html"), Nav::Local("/home/user/a.html".into()));
        assert_eq!(classify("~/x.html"), Nav::Local("/home/user/x.html".into()));
        assert_eq!(classify("https://example.com/a"), Nav::Web("https://example.com/a".into()));
        assert_eq!(classify("example.com/a"), Nav::Web("https://example.com/a".into()));
        assert_eq!(classify("hello world"), Nav::Search("hello world".into()));
        assert_eq!(classify("loos://nope"), Nav::NotFound("nope".into()));
    }

    #[test]
    fn resolves_links() {
        assert_eq!(resolve_href("/home/user/Web/a.html", "b.html"), "/home/user/Web/b.html");
        assert_eq!(resolve_href("/home/user/Web/a.html", "../b.html"), "/home/user/b.html");
        assert_eq!(resolve_href("/home/user/Web/a.html", "/x/y.html"), "/x/y.html");
        assert_eq!(resolve_href("loos://home", "loos://about"), "loos://about");
        assert_eq!(resolve_href("https://example.com/a/b", "c.html"), "https://example.com/a/c.html");
        assert_eq!(resolve_href("/a/b.html", "#anchor"), "");
    }

    #[test]
    fn parses_html_subset() {
        let doc = parse_html(
            "<html><head><title>A &amp; B</title><style>x{}</style></head><body>\
             <h1>Hi</h1><p>Hello <b>bold</b> &lt;world&gt;.</p>\
             <ul><li>one</li><li>two</li></ul>\
             <a href=\"next.html\">Next</a><br>line2\
             <script>alert(1)</script></body></html>",
            "/home/user/Web/index.html",
        );
        assert_eq!(doc.title, "A & B");
        assert!(doc.blocks.iter().any(|b| b.kind == BlockKind::H1 && b.spans[0].text == "Hi"));
        let p = doc.blocks.iter().find(|b| b.kind == BlockKind::P).unwrap();
        let text = p.spans.iter().map(|s| s.text.as_str()).collect::<String>();
        assert!(text.contains("Hello "));
        assert!(text.contains("<world>"));
        assert!(p.spans.iter().any(|s| s.bold && s.text.contains("bold")));
        assert_eq!(doc.blocks.iter().filter(|b| b.kind == BlockKind::Li).count(), 2);
        assert!(doc.blocks.iter().any(|b| b.spans.iter().any(|s| s.link.as_deref() == Some("/home/user/Web/next.html"))));
        let all = doc.plain_text();
        assert!(!all.contains("alert"));
        assert!(!all.contains("x{}"));
    }

    #[test]
    fn tabs_history_and_bookmarks() {
        let mut b = browser();
        assert_eq!(b.tabs.len(), 1);
        assert_eq!(b.current().url, HOME);
        assert!(b.navigate_to("loos://about"));
        assert_eq!(b.current().url, "loos://about");
        assert!(b.back());
        assert_eq!(b.current().url, HOME);
        assert!(b.forward());
        assert_eq!(b.current().url, "loos://about");
        // A new branch drops forward history.
        assert!(b.navigate_to("loos://history"));
        assert!(!b.forward());
        let n = b.bookmarks.len();
        b.toggle_bookmark();
        assert_eq!(b.bookmarks.len(), n + 1);
        b.toggle_bookmark();
        assert_eq!(b.bookmarks.len(), n);
        b.new_tab();
        assert_eq!(b.tabs.len(), 2);
        b.close_tab(1);
        assert_eq!(b.tabs.len(), 1);
        b.close_tab(0);
        assert_eq!(b.tabs.len(), 1);
        assert_eq!(b.current().url, HOME);
    }

    #[test]
    fn web_is_offline() {
        let mut b = browser();
        assert!(!b.navigate_to("https://example.com/x"));
        assert_eq!(b.current().url, "https://example.com/x");
        assert!(!b.current().ok);
        assert!(b.current().doc.plain_text().contains("could not be loaded"));
    }

    #[test]
    fn local_pages_need_a_backend() {
        let mut b = browser();
        assert!(!b.navigate_to("/home/user/Web/index.html"));
        assert!(!b.current().ok);
        assert!(b.current().doc.plain_text().contains("Cannot open page"));
    }

    #[test]
    fn local_search_is_local() {
        let mut b = browser();
        assert!(b.navigate_to("privacy"));
        assert!(b.current().url.starts_with("loos://search?q="));
        let text = b.current().doc.plain_text();
        assert!(text.contains("Local search"));
        assert!(text.contains("sends nothing"));
    }

    #[test]
    fn css_hides_and_styles_blocks() {
        let doc = parse_html_web(
            "<style>.gone{display:none}.red{color:#ff0000} p{margin-top:10px}</style>\
             <body bgcolor=\"#fff\"><p class=\"gone\">hidden</p>\
             <p class=\"red\" style=\"font-size:20px\">visible</p></body>",
            "https://example.com/",
        );
        assert_eq!(doc.blocks.len(), 1);
        let b = &doc.blocks[0];
        assert_eq!(b.style.color, Some(0xffff0000));
        assert_eq!(b.style.size, Some(20));
        assert_eq!(b.style.margin_before, Some(10));
        assert_eq!(doc.bg, Some(0xffffffff));
        assert_eq!(b.spans.iter().map(|s| s.text.as_str()).collect::<String>(), "visible");
    }

    #[test]
    fn follows_meta_refresh_target() {
        let doc = parse_html_web(
            "<meta http-equiv=\"refresh\" content=\"0; url=/next?gbv=1\">",
            "https://www.google.com/",
        );
        assert_eq!(doc.redirect.as_deref(), Some("https://www.google.com/next?gbv=1"));
    }

    #[test]
    fn tsv_round_trip() {
        let items = alloc::vec![
            Bookmark { url: "loos://home".into(), title: "Home".into() },
            Bookmark { url: "/a\tb".into(), title: "T\title".into() },
        ];
        let back = parse_tsv(&to_tsv(&items));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].url, "loos://home");
    }
}
