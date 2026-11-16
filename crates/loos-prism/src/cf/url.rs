//! CFURL / NSURL and NSString path logic: RFC 3986 parsing and reference resolution, percent
//! escapes, and path-component operations. Text travels as UTF-8; results are malloc'ed C strings
//! the class layer turns into string objects and frees.

use super::{bytes, cstr_out};
use crate::libsys::{Act, Call, Func};
use alloc::{vec, vec::Vec};

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// The five components of a URL reference (RFC 3986 section 3), as byte ranges of the text.
#[derive(Default, Clone)]
pub(crate) struct Parts<'a> {
    pub scheme: Option<&'a [u8]>,
    pub authority: Option<&'a [u8]>,
    pub path: &'a [u8],
    pub query: Option<&'a [u8]>,
    pub fragment: Option<&'a [u8]>,
}

pub(crate) fn parse(s: &[u8]) -> Parts<'_> {
    let mut p = Parts::default();
    let mut rest = s;
    if let Some(i) = rest.iter().position(|&b| b == b':') {
        let sch = &rest[..i];
        if !sch.is_empty() && sch[0].is_ascii_alphabetic() && sch.iter().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(b)) {
            p.scheme = Some(sch);
            rest = &rest[i + 1..];
        }
    }
    if let Some(i) = rest.iter().position(|&b| b == b'#') {
        p.fragment = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    if let Some(i) = rest.iter().position(|&b| b == b'?') {
        p.query = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    if rest.starts_with(b"//") {
        let a = &rest[2..];
        let end = a.iter().position(|&b| b == b'/').unwrap_or(a.len());
        p.authority = Some(&a[..end]);
        rest = &a[end..];
    }
    p.path = rest;
    p
}

fn unparse(scheme: Option<&[u8]>, auth: Option<&[u8]>, path: &[u8], query: Option<&[u8]>, frag: Option<&[u8]>) -> Vec<u8> {
    let mut o = Vec::new();
    if let Some(s) = scheme {
        o.extend_from_slice(s);
        o.push(b':');
    }
    if let Some(a) = auth {
        o.extend_from_slice(b"//");
        o.extend_from_slice(a);
    }
    o.extend_from_slice(path);
    if let Some(q) = query {
        o.push(b'?');
        o.extend_from_slice(q);
    }
    if let Some(f) = frag {
        o.push(b'#');
        o.extend_from_slice(f);
    }
    o
}

/// RFC 3986 5.2.4.
fn remove_dots(path: &[u8]) -> Vec<u8> {
    let mut out: Vec<&[u8]> = Vec::new();
    let absolute = path.starts_with(b"/");
    let segs: Vec<&[u8]> = path.split(|&b| b == b'/').collect();
    let n = segs.len();
    let mut trailing;
    for (i, s) in segs.iter().enumerate() {
        if i == 0 && absolute {
            continue;
        }
        match *s {
            b"." => trailing = true,
            b".." => {
                out.pop();
                trailing = true;
            }
            _ => {
                out.push(s);
                trailing = false;
            }
        }
        if i + 1 == n && trailing {
            out.push(b"");
        }
    }
    let mut o = Vec::new();
    if absolute {
        o.push(b'/');
    }
    for (i, s) in out.iter().enumerate() {
        if i > 0 {
            o.push(b'/');
        }
        o.extend_from_slice(s);
    }
    o
}

/// RFC 3986 5.2.2: `rel` against `base`.
pub(crate) fn resolve(base: &[u8], rel: &[u8]) -> Vec<u8> {
    let (b, r) = (parse(base), parse(rel));
    if r.scheme.is_some() {
        return unparse(r.scheme, r.authority, &remove_dots(r.path), r.query, r.fragment);
    }
    if r.authority.is_some() {
        return unparse(b.scheme, r.authority, &remove_dots(r.path), r.query, r.fragment);
    }
    if r.path.is_empty() {
        return unparse(b.scheme, b.authority, b.path, r.query.or(b.query), r.fragment);
    }
    let merged: Vec<u8> = if r.path.starts_with(b"/") {
        r.path.to_vec()
    } else if b.authority.is_some() && b.path.is_empty() {
        [b"/".as_slice(), r.path].concat()
    } else {
        let cut = b.path.iter().rposition(|&x| x == b'/').map(|i| i + 1).unwrap_or(0);
        [&b.path[..cut], r.path].concat()
    };
    unparse(b.scheme, b.authority, &remove_dots(&merged), r.query, r.fragment)
}

pub(crate) fn unescape(s: &[u8]) -> Option<Vec<u8>> {
    let mut o = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            let v = u8::from_str_radix(core::str::from_utf8(h).ok()?, 16).ok()?;
            o.push(v);
            i += 3;
        } else {
            o.push(s[i]);
            i += 1;
        }
    }
    core::str::from_utf8(&o).ok()?;
    Some(o)
}

/// Percent-encode everything outside the unreserved set and `keep`.
fn escape(s: &[u8], keep: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for &b in s {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || keep.contains(&b) {
            o.push(b);
        } else {
            o.extend_from_slice(alloc::format!("%{b:02X}").as_bytes());
        }
    }
    o
}

/// A URL's part: 0 scheme, 1 user, 2 password, 3 host, 4 port, 5 path, 6 query, 7 fragment,
/// 8 resource specifier, 9 last path component, 10 path extension, 11 file-system path (decoded,
/// no trailing slash), 12 net location (authority).
fn part(s: &[u8], which: u64) -> Option<Vec<u8>> {
    let p = parse(s);
    let auth = p.authority.unwrap_or(b"");
    let (userinfo, hostport) = match auth.iter().rposition(|&b| b == b'@') {
        Some(i) => (Some(&auth[..i]), &auth[i + 1..]),
        None => (None, auth),
    };
    let (host, port) = if hostport.starts_with(b"[") {
        let e = hostport.iter().position(|&b| b == b']').unwrap_or(hostport.len() - 1);
        (&hostport[1..e], hostport.get(e + 1..).and_then(|r| r.strip_prefix(b":")))
    } else {
        match hostport.iter().rposition(|&b| b == b':') {
            Some(i) => (&hostport[..i], Some(&hostport[i + 1..])),
            None => (hostport, None),
        }
    };
    let decoded_path = || {
        let mut d = unescape(p.path).unwrap_or_else(|| p.path.to_vec());
        while d.len() > 1 && d.ends_with(b"/") {
            d.pop();
        }
        d
    };
    match which {
        0 => p.scheme.map(<[u8]>::to_vec),
        1 => userinfo.map(|u| u.split(|&b| b == b':').next().unwrap_or(b"").to_vec()),
        2 => userinfo.and_then(|u| u.iter().position(|&b| b == b':').map(|i| u[i + 1..].to_vec())),
        3 => p.authority.map(|_| host.to_vec()),
        4 => port.filter(|x| !x.is_empty()).map(<[u8]>::to_vec),
        5 => Some(p.path.to_vec()),
        6 => p.query.map(<[u8]>::to_vec),
        7 => p.fragment.map(<[u8]>::to_vec),
        8 => p.scheme.map(|sch| s[sch.len() + 1..].to_vec()),
        9 => Some(path_op(&decoded_path(), 0, b"")),
        10 => Some(path_op(&decoded_path(), 1, b"")),
        11 => Some(decoded_path()),
        12 => p.authority.map(<[u8]>::to_vec),
        _ => None,
    }
}

/// NSString's path operations: 0 lastPathComponent, 1 pathExtension, 2 appending a component,
/// 3 deleting the last component, 4 appending an extension, 5 deleting the extension,
/// 6 standardizing (`.`, `..`, `//`, trailing `/`).
pub(crate) fn path_op(s: &[u8], op: u64, arg: &[u8]) -> Vec<u8> {
    let mut t = s.to_vec();
    while t.len() > 1 && t.ends_with(b"/") {
        t.pop();
    }
    let last_start = t.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0);
    let dot = t[last_start..].iter().rposition(|&b| b == b'.').filter(|&i| i > 0).map(|i| last_start + i);
    match op {
        0 => if t == b"/" { t } else { t[last_start..].to_vec() },
        1 => dot.map(|i| t[i + 1..].to_vec()).unwrap_or_default(),
        2 => {
            if t.is_empty() {
                return arg.to_vec();
            }
            let mut o = t;
            if !o.ends_with(b"/") {
                o.push(b'/');
            }
            o.extend_from_slice(arg.strip_prefix(b"/").unwrap_or(arg));
            o
        }
        3 => match t.iter().rposition(|&b| b == b'/') {
            Some(0) => b"/".to_vec(),
            Some(i) => t[..i].to_vec(),
            None => Vec::new(),
        },
        4 => {
            if t.is_empty() || t == b"/" {
                return t;
            }
            [t.as_slice(), b".", arg].concat()
        }
        5 => dot.map(|i| t[..i].to_vec()).unwrap_or(t),
        6 => {
            let mut c = Vec::with_capacity(t.len());
            for &b in &t {
                if !(b == b'/' && c.last() == Some(&b'/')) {
                    c.push(b);
                }
            }
            if !c.starts_with(b"/") {
                // a relative path keeps its leading `..`s
                let mut parts: Vec<&[u8]> = Vec::new();
                for seg in c.split(|&b| b == b'/') {
                    match seg {
                        b"." | b"" => {}
                        b".." if parts.last().is_some_and(|p| *p != b"..") => {
                            parts.pop();
                        }
                        _ => parts.push(seg),
                    }
                }
                return parts.join(&b'/');
            }
            let mut r = remove_dots(&c);
            while r.len() > 1 && r.ends_with(b"/") {
                r.pop();
            }
            r
        }
        _ => t,
    }
}

/// The trap functions of this module.
pub static FUNCS: &[(&str, Func)] = &[
    // (utf8, length, part) → malloc'ed text, or NULL when the URL has no such part
    f!("___prism_cf_url_part", |c| {
        let (p, n, which) = (c.arg(), c.arg(), c.arg());
        let s = bytes(c, p, n);
        match part(&s, which) {
            Some(v) => { let r = cstr_out(c, &v); c.ret(r) }
            None => c.ret(0),
        }
    }),
    // (base, length, relative, length) → malloc'ed absolute URL
    f!("___prism_cf_url_resolve", |c| {
        let (bp, bn, rp, rn) = (c.arg(), c.arg(), c.arg(), c.arg());
        let (b, r) = (bytes(c, bp, bn), bytes(c, rp, rn));
        let v = resolve(&b, &r);
        let r = cstr_out(c, &v);
        c.ret(r)
    }),
    // (utf8, length) → malloc'ed decoded text, or NULL when an escape is malformed
    f!("___prism_cf_url_unescape", |c| {
        let (p, n) = (c.arg(), c.arg());
        let s = bytes(c, p, n);
        match unescape(&s) {
            Some(v) => { let r = cstr_out(c, &v); c.ret(r) }
            None => c.ret(0),
        }
    }),
    // (utf8, length, kind): 0 a path, 1 a query, 2 a fragment, 3 a single component
    f!("___prism_cf_url_escape", |c| {
        let (p, n, kind) = (c.arg(), c.arg(), c.arg());
        let s = bytes(c, p, n);
        let keep: &[u8] = match kind {
            0 => b"/:@!$&'()*+,;=",
            1 | 2 => b"/?:@!$&'()*+,;=",
            _ => b"",
        };
        let v = escape(&s, keep);
        let r = cstr_out(c, &v);
        c.ret(r)
    }),
    // (utf8, length, op, argument, length) → malloc'ed result (see `path_op`)
    f!("___prism_cf_path", |c| {
        let (p, n, op, ap, an) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let (s, a) = (bytes(c, p, n), bytes(c, ap, an));
        let v = path_op(&s, op, &a);
        let r = cstr_out(c, &v);
        c.ret(r)
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_splits() {
        let base = b"http://a/b/c/d;p?q";
        for (r, want) in [("g", "http://a/b/c/g"), ("./g", "http://a/b/c/g"), ("g/", "http://a/b/c/g/"), ("/g", "http://a/g"),
            ("//g", "http://g"), ("?y", "http://a/b/c/d;p?y"), ("#s", "http://a/b/c/d;p?q#s"), ("../g", "http://a/b/g"),
            ("../..", "http://a/"), ("../../../g", "http://a/g"), ("", "http://a/b/c/d;p?q")] {
            assert_eq!(resolve(base, r.as_bytes()), want.as_bytes(), "{r}");
        }
        let u = b"https://me:pw@example.com:8080/a%20b/c.txt?x=1#top";
        let p = |w| part(u, w).map(|v| alloc::string::String::from_utf8(v).unwrap());
        assert_eq!((p(0), p(1), p(2), p(3), p(4)), (Some("https".into()), Some("me".into()), Some("pw".into()), Some("example.com".into()), Some("8080".into())));
        assert_eq!((p(6), p(7), p(9), p(10), p(11)), (Some("x=1".into()), Some("top".into()), Some("c.txt".into()), Some("txt".into()), Some("/a b/c.txt".into())));
        assert_eq!(path_op(b"/a/b.tar.gz", 5, b""), b"/a/b.tar");
        assert_eq!(path_op(b"/a//./b/../c/", 6, b""), b"/a/c");
        assert_eq!(path_op(b"/", 2, b"x"), b"/x");
        assert_eq!(path_op(b"/x", 3, b""), b"/");
        assert_eq!(escape(b"a b/c", b"/"), b"a%20b/c");
        let _ = vec![0u8];
    }
}
