//! Windows path <-> LoOS path mapping. LoOS uses a Linux-style tree; Windows programs keep
//! seeing their usual drive paths, translated here (longest prefix wins, ASCII case-insensitive):
//!
//! | Windows                        | LoOS                       |
//! |--------------------------------|----------------------------|
//! | `C:\Users\<u>\AppData`         | `/home/<u>/.appdata`       |
//! | `C:\Users\Public`              | `/home/public`             |
//! | `C:\Users\Default`             | `/system/skel`             |
//! | `C:\Users\<u>`                 | `/home/<u>` (lower-case)   |
//! | `C:\Program Files (x86)`       | `/apps/x86`                |
//! | `C:\Program Files`             | `/apps`                    |
//! | `C:\ProgramData`               | `/data`                    |
//! | `C:\Windows\Temp`              | `/tmp`                     |
//! | `C:\Windows`                   | `/system/windows`          |
//! | `C:\<other>`                   | `/mnt/c/<other>`           |
//! | `D:\…` (any other drive)       | `/mnt/d/…`                 |

use alloc::string::String;

const FIXED: &[(&str, &str)] = &[
    ("users\\public", "/home/public"),
    ("users\\default", "/system/skel"),
    ("program files (x86)", "/apps/x86"),
    ("program files", "/apps"),
    ("programdata", "/data"),
    ("windows\\temp", "/tmp"),
    ("windows", "/system/windows"),
    ("users", "/home"),
];

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    let head = s.get(..p.len())?;
    if !head.eq_ignore_ascii_case(p) {
        return None;
    }
    let rest = &s[p.len()..];
    (rest.is_empty() || rest.starts_with('\\')).then_some(rest)
}

fn push_rest(out: &mut String, rest: &str) {
    for c in rest.split('\\').filter(|c| !c.is_empty()) {
        out.push('/');
        out.push_str(c);
    }
}

/// Maps a DOS or NT path (`C:\x`, `\??\C:\x`, `\\?\C:\x`, `/` separators allowed) to a LoOS
/// path. `None` for relative, UNC and device paths, and for `.`/`..` components.
pub fn to_loos(path: &str) -> Option<String> {
    let mut p: String = path.chars().map(|c| if c == '/' { '\\' } else { c }).collect();
    for pre in ["\\??\\", "\\\\?\\"] {
        if let Some(r) = p.strip_prefix(pre) {
            p = String::from(r);
        }
    }
    let b = p.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b.len() > 2 && b[2] != b'\\') {
        return None;
    }
    let drive = (b[0] as char).to_ascii_lowercase();
    let rest = p.get(3..).unwrap_or("");
    if rest.split('\\').any(|c| c == "." || c == "..") {
        return None;
    }
    let mut out = String::new();
    if drive != 'c' {
        out.push_str("/mnt/");
        out.push(drive);
        push_rest(&mut out, rest);
        return Some(out);
    }
    // C:\Users\<u>[\AppData] needs the user component.
    if let Some(r) = strip_prefix_ci(rest, "users") {
        let mut parts = r.trim_start_matches('\\').splitn(2, '\\');
        let user = parts.next().unwrap_or("");
        let tail = parts.next().unwrap_or("");
        let fixed = ["public", "default"].iter().any(|n| user.eq_ignore_ascii_case(n));
        if !user.is_empty() && !fixed {
            out.push_str("/home/");
            out.extend(user.chars().map(|c| c.to_ascii_lowercase()));
            match strip_prefix_ci(tail, "appdata") {
                Some(t) => {
                    out.push_str("/.appdata");
                    push_rest(&mut out, t);
                }
                None => push_rest(&mut out, tail),
            }
            return Some(out);
        }
    }
    for (win, loos) in FIXED {
        if let Some(r) = strip_prefix_ci(rest, win) {
            out.push_str(loos);
            push_rest(&mut out, r);
            return Some(out);
        }
    }
    out.push_str("/mnt/c");
    push_rest(&mut out, rest);
    Some(out)
}

/// Inverse of [`to_loos`] for paths a Windows program may be shown (`/home/u/x` -> `C:\Users\u\x`).
/// Paths outside the mapped tree go to `Z:\` (whole LoOS tree), like Wine.
pub fn to_windows(path: &str) -> String {
    let rev: &[(&str, &str)] = &[
        ("/home/public", "C:\\Users\\Public"),
        ("/system/skel", "C:\\Users\\Default"),
        ("/apps/x86", "C:\\Program Files (x86)"),
        ("/apps", "C:\\Program Files"),
        ("/data", "C:\\ProgramData"),
        ("/tmp", "C:\\Windows\\Temp"),
        ("/system/windows", "C:\\Windows"),
        ("/mnt/c", "C:"),
    ];
    let seg = |p: &str, pre: &str| p.strip_prefix(pre).filter(|r| r.is_empty() || r.starts_with('/')).map(String::from);
    let win = |base: &str, rest: &str| {
        let mut s = String::from(base);
        s.extend(rest.chars().map(|c| if c == '/' { '\\' } else { c }));
        if s.len() == 2 {
            s.push('\\');
        }
        s
    };
    if let Some(r) = seg(path, "/home") {
        let mut parts = r.trim_start_matches('/').splitn(2, '/');
        let user = parts.next().unwrap_or("");
        let tail = parts.next().map(|t| alloc::format!("/{t}")).unwrap_or_default();
        if !user.is_empty() && user != "public" {
            let tail = match seg(&tail, "/.appdata") {
                Some(t) => alloc::format!("/AppData{t}"),
                None => tail,
            };
            return win(&alloc::format!("C:\\Users\\{user}"), &tail);
        }
    }
    for (l, w) in rev {
        if let Some(r) = seg(path, l) {
            return win(w, &r);
        }
    }
    if let Some(r) = path.strip_prefix("/mnt/") {
        let mut it = r.chars();
        if let (Some(d), rest) = (it.next(), it.as_str()) {
            if d.is_ascii_alphabetic() && (rest.is_empty() || rest.starts_with('/')) {
                return win(&alloc::format!("{}:", d.to_ascii_uppercase()), rest);
            }
        }
    }
    win("Z:", path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_windows_to_loos() {
        let t = |w: &str| to_loos(w).unwrap();
        assert_eq!(t("C:\\Users\\Alice\\Documents\\a.txt"), "/home/alice/Documents/a.txt");
        assert_eq!(t("c:/users/alice/AppData/Roaming/x"), "/home/alice/.appdata/Roaming/x");
        assert_eq!(t("\\??\\C:\\Program Files\\App\\app.exe"), "/apps/App/app.exe");
        assert_eq!(t("\\\\?\\C:\\Program Files (x86)\\Old"), "/apps/x86/Old");
        assert_eq!(t("C:\\ProgramData\\Vendor"), "/data/Vendor");
        assert_eq!(t("C:\\Windows\\System32\\kernel32.dll"), "/system/windows/System32/kernel32.dll");
        assert_eq!(t("C:\\Windows\\Temp\\x.tmp"), "/tmp/x.tmp");
        assert_eq!(t("C:\\Users\\Public\\Music"), "/home/public/Music");
        assert_eq!(t("C:\\Users"), "/home");
        assert_eq!(t("C:\\"), "/mnt/c");
        assert_eq!(t("C:\\games\\x"), "/mnt/c/games/x");
        assert_eq!(t("D:\\media"), "/mnt/d/media");
        assert_eq!(t("C:\\Program FilesX\\y"), "/mnt/c/Program FilesX/y");
        for bad in ["relative\\x", "\\\\server\\share", "C:\\Users\\..\\x", "C:", "CC:\\x"] {
            assert_eq!(to_loos(bad), if bad == "C:" { Some("/mnt/c".into()) } else { None }, "{bad}");
        }
    }

    #[test]
    fn maps_loos_to_windows_and_round_trips() {
        assert_eq!(to_windows("/home/alice/Documents"), "C:\\Users\\alice\\Documents");
        assert_eq!(to_windows("/home/alice/.appdata/Local"), "C:\\Users\\alice\\AppData\\Local");
        assert_eq!(to_windows("/apps/x86/A"), "C:\\Program Files (x86)\\A");
        assert_eq!(to_windows("/mnt/d"), "D:\\");
        assert_eq!(to_windows("/etc/hosts"), "Z:\\etc\\hosts");
        for w in ["C:\\Users\\bob\\Desktop", "C:\\Program Files\\X", "C:\\Windows", "D:\\a\\b"] {
            assert!(to_windows(&to_loos(w).unwrap()).eq_ignore_ascii_case(w), "{w}");
        }
    }
}
