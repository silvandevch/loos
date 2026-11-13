//! `.app` bundles: `Name.app/Contents/Info.plist` names the executable in `Contents/MacOS/`.

#[allow(unused_imports)]
use alloc::{format, string::{String, ToString}, vec::Vec};
use loos_lin::fs::{resolve, Kind, LinFs};

/// Bundle metadata from `Info.plist`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    /// Bundle directory (`/Applications/Foo.app`).
    pub path: String,
    /// `CFBundleExecutable`.
    pub executable: String,
    /// `CFBundleIdentifier`.
    pub identifier: Option<String>,
    /// `CFBundleName` / `CFBundleDisplayName`.
    pub name: Option<String>,
    /// `CFBundleShortVersionString`.
    pub version: Option<String>,
}

impl Bundle {
    /// Full path of the main executable.
    pub fn exe_path(&self) -> String {
        format!("{}/Contents/MacOS/{}", self.path.trim_end_matches('/'), self.executable)
    }
}

/// String values of the top-level dictionary of an XML property list (`<key>K</key><string>V</string>`).
pub fn plist_strings(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(k) = rest.find("<key>") {
        rest = &rest[k + 5..];
        let Some(ke) = rest.find("</key>") else { break };
        let key = unescape(&rest[..ke]);
        rest = &rest[ke + 6..];
        let t = rest.trim_start();
        if let Some(v) = t.strip_prefix("<string>") {
            if let Some(ve) = v.find("</string>") {
                out.push((key, unescape(&v[..ve])));
            }
        } else if t.starts_with("<string/>") {
            out.push((key, String::new()));
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.trim().replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// If `path` is an application bundle, read its metadata.
pub fn open(fs: &mut dyn LinFs, path: &str) -> Option<Bundle> {
    let dir = resolve(fs, "/", path, true).ok()?;
    if fs.lstat(&dir).ok()?.kind != Kind::Dir {
        return None;
    }
    let plist = format!("{}/Contents/Info.plist", dir.trim_end_matches('/'));
    let data = crate::loader::read_file(fs, &plist).ok();
    let mut b = Bundle { path: dir.clone(), ..Bundle::default() };
    if let Some(d) = data {
        if d.starts_with(b"bplist") {
            return None; // binary plists are not decoded yet
        }
        for (k, v) in plist_strings(&String::from_utf8_lossy(&d)) {
            match k.as_str() {
                "CFBundleExecutable" => b.executable = v,
                "CFBundleIdentifier" => b.identifier = Some(v),
                "CFBundleName" if b.name.is_none() => b.name = Some(v),
                "CFBundleDisplayName" => b.name = Some(v),
                "CFBundleShortVersionString" => b.version = Some(v),
                _ => {}
            }
        }
    }
    if b.executable.is_empty() {
        // No plist (or no key): the executable is named after the bundle.
        let base = dir.trim_end_matches('/').rsplit('/').next()?.trim_end_matches(".app").to_string();
        b.executable = base;
    }
    fs.lstat(&b.exe_path()).ok().map(|_| b)
}
