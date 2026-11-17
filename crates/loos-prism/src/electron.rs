//! Electron apps. A macOS Electron app is Chromium + Node (in `Electron Framework.framework`)
//! plus the app's own JavaScript and assets in `Contents/Resources`. The JavaScript is the same
//! on every platform, so Prism runs it on the Linux build of the same Electron major version,
//! which the Linux layer already runs, instead of translating Chromium itself.
//!
//! Launching builds a folder shaped like a Linux Electron app, in `/run/prism/electron/<name>`:
//! the runtime's executable (a hard link, so `/proc/self/exe` and Chromium's resource lookup see
//! this folder), symbolic links to the runtime's other files, and `resources` mirroring the
//! bundle's `Contents/Resources`. Electron starts `resources/app.asar` (or `resources/app`,
//! depending on the version), and there Prism puts a small loader: it fills in the macOS-only Electron APIs a macOS build calls unconditionally
//! (`systemPreferences.setUserDefault`, `app.dock`, window vibrancy, ...) with harmless stand-ins,
//! restores the app's name, version and data folder, and loads the app's own `app.asar` (moved to
//! `resources/mac/`) or `app` folder (moved to `app.mac`). The app sees `process.platform == "linux"`.
//!
//! Runtimes are looked up in `/usr/lib/prism/electron/<version or major>/`, then the distribution
//! locations `/usr/lib{,64}/electron<major>/` and `/usr/lib{,64}/electron/`; each holds an
//! `electron` executable. The same version is preferred, then the same major, then the nearest
//! newer major, then the nearest older one (with a note).
//!
//! Native Node modules (`.node`) inside the bundle are Mach-O and cannot load into Linux
//! Electron. Linux builds of them go in `/usr/lib/prism/electron/apps/<bundle id>/`, laid out
//! like `Contents/Resources`; files there replace the bundle's. Mach-O modules without one are
//! reported, and the app runs without them.

#[allow(unused_imports)]
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use loos_lin::fs::{resolve, Kind, LinFs};

use crate::bundle::Bundle;
use crate::cf::plist::{self, V};

/// Where LoOS installs Electron runtimes and per-app Linux replacements.
pub const RUNTIMES: &str = "/usr/lib/prism/electron";
/// Where launch folders are built.
pub const LAUNCH: &str = "/run/prism/electron";

/// Chromium switches Electron needs on LoOS: no sandbox or zygote (no namespaces or seccomp),
/// Wayland output, software rendering in one process.
pub const SWITCHES: &[&str] = &[
    "--no-sandbox",
    "--ozone-platform=wayland",
    "--disable-gpu",
    "--disable-gpu-compositing",
    "--no-zygote",
    "--disable-dev-shm-usage",
    "--in-process-gpu",
];

/// A Linux command line for an Electron bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// Executable inside the launch folder.
    pub exe: String,
    /// Full argument vector (`exe`, switches, then the caller's arguments).
    pub argv: Vec<String>,
    /// Environment with the additions Electron needs.
    pub envp: Vec<String>,
}

/// The Electron version a bundle is built on (`CFBundleVersion` of its Electron Framework), or
/// `None` when it is not an Electron app.
pub fn version(fs: &mut dyn LinFs, b: &Bundle) -> Option<String> {
    let fw = format!("{}/Contents/Frameworks/Electron Framework.framework", b.path.trim_end_matches('/'));
    for rel in ["Resources/Info.plist", "Versions/Current/Resources/Info.plist", "Versions/A/Resources/Info.plist"] {
        let Ok(p) = resolve(fs, "/", &format!("{fw}/{rel}"), true) else { continue };
        let Ok(d) = crate::loader::read_file(fs, &p) else { continue };
        if let Some((V::Dict(kv), _)) = plist::parse(&d) {
            for (k, v) in kv {
                if let (b"CFBundleVersion", V::Str(s)) = (&k[..], v) {
                    return Some(String::from_utf8_lossy(&s).into_owned());
                }
            }
        }
    }
    None
}

fn major(v: &str) -> Option<u32> {
    v.split('.').next()?.parse().ok()
}

fn is_file(fs: &mut dyn LinFs, p: &str) -> bool {
    resolve(fs, "/", p, true).ok().and_then(|r| fs.lstat(&r).ok()).is_some_and(|s| s.kind == Kind::File)
}

fn read_text(fs: &mut dyn LinFs, p: &str) -> Option<String> {
    let r = resolve(fs, "/", p, true).ok()?;
    Some(String::from_utf8_lossy(&crate::loader::read_file(fs, &r).ok()?).trim().to_string())
}

/// Installed runtimes as (directory, version or bare major).
fn runtimes(fs: &mut dyn LinFs) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut add = |fs: &mut dyn LinFs, dir: String, named: Option<String>| {
        if !is_file(fs, &format!("{dir}/electron")) {
            return;
        }
        // A `version` file (in every Electron release) beats the folder name.
        let v = read_text(fs, &format!("{dir}/version")).map(|v| v.trim_start_matches('v').to_string()).or(named);
        if let Some(v) = v.filter(|v| major(v).is_some()) {
            out.push((dir, v));
        }
    };
    for e in fs.list(RUNTIMES).unwrap_or_default() {
        if e.name.starts_with(|c: char| c.is_ascii_digit()) {
            add(fs, format!("{RUNTIMES}/{}", e.name), Some(e.name.clone()));
        }
    }
    for lib in ["/usr/lib", "/usr/lib64"] {
        for e in fs.list(lib).unwrap_or_default() {
            if let Some(n) = e.name.strip_prefix("electron") {
                let named = (!n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())).then(|| n.to_string());
                if n.is_empty() || named.is_some() {
                    add(fs, format!("{lib}/{}", e.name), named);
                }
            }
        }
    }
    out
}

/// The best runtime for `want`: same version, same major, nearest newer major, nearest older.
fn pick(list: &[(String, String)], want: &str) -> Option<(String, String)> {
    let wm = major(want)?;
    let rank = |v: &str| {
        let m = major(v).unwrap_or(0);
        if v == want {
            (0, 0)
        } else if m == wm {
            (1, 0)
        } else if m > wm {
            (2, m - wm)
        } else {
            (3, wm - m)
        }
    };
    list.iter().min_by_key(|(_, v)| rank(v)).cloned()
}

fn ensure_dir(fs: &mut dyn LinFs, p: &str) -> Result<(), i64> {
    let mut cur = String::new();
    for c in p.split('/').filter(|c| !c.is_empty()) {
        cur.push('/');
        cur.push_str(c);
        match fs.lstat(&cur) {
            Ok(s) if s.kind == Kind::Dir => {}
            Ok(_) => return Err(loos_lin::errno::ENOTDIR),
            Err(_) => fs.mkdir(&cur, 0o755)?,
        }
    }
    Ok(())
}

/// Remove `p` (recursively) if it exists.
fn remove_tree(fs: &mut dyn LinFs, p: &str) {
    match fs.lstat(p) {
        Ok(s) if s.kind == Kind::Dir => {
            for e in fs.list(p).unwrap_or_default() {
                remove_tree(fs, &format!("{p}/{}", e.name));
            }
            let _ = fs.remove(p, true);
        }
        Ok(_) => {
            let _ = fs.remove(p, false);
        }
        Err(_) => {}
    }
}

/// Is the file at `p` a Mach-O image?
fn is_macho(fs: &mut dyn LinFs, p: &str) -> bool {
    let mut head = [0u8; 8];
    matches!(fs.read(p, 0, &mut head), Ok(8)) && crate::macho::is_macho(&head)
}

/// Mirror `src` at `dst` as symbolic links; entries of `over` replace or add to it. Only folders
/// that `over` touches are recreated, everything else is one link.
fn mirror(fs: &mut dyn LinFs, src: &str, over: Option<&str>, dst: &str) -> Result<(), i64> {
    let o_kind = over.and_then(|o| fs.lstat(o).ok()).map(|s| s.kind);
    let s_dir = resolve(fs, "/", src, true).ok().and_then(|r| fs.lstat(&r).ok()).is_some_and(|st| st.kind == Kind::Dir);
    match (o_kind, over) {
        (Some(Kind::Dir), Some(o)) if s_dir => {
            fs.mkdir(dst, 0o755)?;
            mirror_children(fs, src, Some(o), dst, |n| n.to_string())
        }
        (Some(_), Some(o)) => fs.symlink(o, dst),
        _ => fs.symlink(src, dst),
    }
}

/// The entries of `src` and `over` mirrored into the existing folder `dst`, named by `name`.
fn mirror_children(fs: &mut dyn LinFs, src: &str, over: Option<&str>, dst: &str, name: impl Fn(&str) -> String) -> Result<(), i64> {
    let mut names: Vec<String> = fs.list(src).unwrap_or_default().into_iter().map(|e| e.name).collect();
    if let Some(o) = over {
        for e in fs.list(o).unwrap_or_default() {
            if !names.contains(&e.name) {
                names.push(e.name);
            }
        }
    }
    for n in names {
        let o = over.map(|o| format!("{o}/{n}"));
        mirror(fs, &format!("{src}/{n}"), o.as_deref(), &format!("{dst}/{}", name(&n)))?;
    }
    Ok(())
}

fn write_new(fs: &mut dyn LinFs, p: &str, data: &[u8]) -> Result<(), i64> {
    fs.create(p, 0o644)?;
    fs.write(p, 0, data)?;
    Ok(())
}

/// An asar archive of flat `files`: a pickled header size, a pickled JSON index, then the data.
fn asar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut index = String::from("{\"files\":{");
    let mut off = 0;
    for (i, (n, d)) in files.iter().enumerate() {
        index += &format!("{}\"{n}\":{{\"size\":{},\"offset\":\"{off}\"}}", if i > 0 { "," } else { "" }, d.len());
        off += d.len();
    }
    index += "}}";
    let pad = (4 - index.len() % 4) % 4;
    let header = 8 + index.len() + pad; // pickle: payload size, string length, string, padding
    let mut out = Vec::new();
    for v in [4u32, header as u32, (header - 4) as u32, index.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    out.extend(index.as_bytes());
    out.extend(core::iter::repeat_n(0u8, pad));
    for (_, d) in files {
        out.extend(*d);
    }
    out
}

/// `resources/app/package.json`: Electron loads `resources/app` before `resources/app.asar`, so
/// this loader runs first; it names itself after nothing, the loader fixes the name.
const SHIM_PACKAGE: &str = r#"{"name":"prism-electron","main":"prism.js"}"#;

/// The loader: a macOS build calls macOS-only Electron APIs without checking the platform, so
/// the missing ones become harmless stand-ins (only where Electron lacks them); then the app's
/// own package is loaded as Electron would have, with its name, version and data folder.
const SHIM: &str = r#"'use strict';
const path = require('path'), fs = require('fs'), Module = require('module');
const electron = require('electron'), { app } = electron;
const res = process.resourcesPath;
const appPath = fs.existsSync(path.join(res, 'app.mac')) ? path.join(res, 'app.mac') : path.join(res, 'mac', 'app.asar');
const pkg = JSON.parse(fs.readFileSync(path.join(appPath, 'package.json'), 'utf8'));
const fill = (o, names, f) => { if (o) for (const n of names) if (typeof o[n] !== 'function') o[n] = f(n); };
const nop = () => () => undefined;
fill(electron.systemPreferences, ['setUserDefault', 'removeUserDefault', 'registerDefaults', 'unsubscribeNotification',
  'unsubscribeLocalNotification', 'unsubscribeWorkspaceNotification', 'postNotification', 'postLocalNotification',
  'postWorkspaceNotification', 'setAppLevelAppearance'], nop);
fill(electron.systemPreferences, ['subscribeNotification', 'subscribeLocalNotification', 'subscribeWorkspaceNotification'], () => () => 0);
fill(electron.systemPreferences, ['getUserDefault'], () => (key, type) =>
  ({ string: '', boolean: false, integer: 0, float: 0, double: 0, url: '', array: [], dictionary: {} })[type]);
fill(electron.systemPreferences, ['isSwipeTrackingFromScrollEventsEnabled', 'canPromptTouchID', 'isTrustedAccessibilityClient'], () => () => false);
fill(electron.systemPreferences, ['getEffectiveAppearance'], () => () => (electron.nativeTheme.shouldUseDarkColors ? 'dark' : 'light'));
fill(electron.systemPreferences, ['getAppLevelAppearance'], () => () => null);
fill(electron.systemPreferences, ['getMediaAccessStatus'], () => () => 'granted');
fill(electron.systemPreferences, ['askForMediaAccess'], () => () => Promise.resolve(true));
fill(electron.systemPreferences, ['promptTouchID'], () => () => Promise.reject(new Error('Touch ID is not available')));
if (!app.dock) {
  const dock = {};
  fill(dock, ['cancelBounce', 'downloadFinished', 'setBadge', 'hide', 'setMenu', 'setIcon'], nop);
  fill(dock, ['bounce'], () => () => 0);
  fill(dock, ['getBadge'], () => () => '');
  fill(dock, ['getMenu'], () => () => null);
  fill(dock, ['isVisible'], () => () => true);
  fill(dock, ['show'], () => () => Promise.resolve());
  app.dock = dock;
}
fill(app, ['isInApplicationsFolder', 'isSecureKeyboardEntryEnabled'], () => (n) => n === 'isInApplicationsFolder');
fill(app, ['moveToApplicationsFolder', 'isHidden'], () => () => false);
fill(app, ['hide', 'show', 'setActivationPolicy', 'setSecureKeyboardEntryEnabled'], nop);
fill(app, ['configureWebAuthn'], nop);
// WebContents zoom methods some Electron builds lack, on whichever zoom calls this one has
// (captured first: the `zoomLevel` property itself goes through getZoomLevel).
const fillContents = (wc) => {
  const p = Object.getPrototypeOf(wc), gzf = p.getZoomFactor, szf = p.setZoomFactor;
  fill(p, ['getZoomLevel'], () => function () { return gzf ? Math.log(gzf.call(this)) / Math.log(1.2) : (this.__prismZoom || 0); });
  fill(p, ['setZoomLevel'], () => function (l) { if (szf) szf.call(this, Math.pow(1.2, l)); else this.__prismZoom = l; });
  fill(p, ['getZoomFactor'], () => function () { return Math.pow(1.2, this.getZoomLevel()); });
  fill(p, ['setZoomFactor'], () => function (f) { this.setZoomLevel(Math.log(f) / Math.log(1.2)); });
};
app.on('web-contents-created', (e, wc) => fillContents(wc));
const wp = electron.BrowserWindow && electron.BrowserWindow.prototype;
fill(wp, ['setVibrancy', 'setWindowButtonVisibility', 'setTrafficLightPosition', 'setWindowButtonPosition', 'setSheetOffset',
  'setAutoHideCursor', 'setRepresentedFilename', 'setDocumentEdited', 'invalidateShadow', 'setSimpleFullScreen',
  'setHiddenInMissionControl', 'addTabbedWindow', 'toggleTabBar', 'mergeAllWindows', 'moveTabToNewWindow', 'showAllTabs',
  'selectPreviousTab', 'selectNextTab', 'setTouchBar'], nop);
fill(wp, ['getTrafficLightPosition', 'getWindowButtonPosition'], () => () => ({ x: 0, y: 0 }));
fill(wp, ['isDocumentEdited', 'isSimpleFullScreen', 'isHiddenInMissionControl'], () => () => false);
fill(wp, ['getRepresentedFilename'], () => () => '');
const name = pkg.productName || pkg.name;
if (name) {
  app.setName(name);
  app.setPath('userData', path.join(app.getPath('appData'), name));
  try { app.setPath('sessionData', app.getPath('userData')); } catch (e) {}
}
if (pkg.version) app.setVersion(pkg.version);
if (pkg.desktopName && app.setDesktopName) app.setDesktopName(pkg.desktopName);
app.setAppPath(appPath);
Module._load(path.join(appPath, pkg.main || 'index.js'), Module, true);
"#;

/// Mach-O `.node` files under `dir` (as paths relative to it) that `over` does not replace.
fn foreign_modules(fs: &mut dyn LinFs, dir: &str, rel: &str, over: &str, out: &mut Vec<String>) {
    for e in fs.list(dir).unwrap_or_default() {
        let (p, r) = (format!("{dir}/{}", e.name), if rel.is_empty() { e.name.clone() } else { format!("{rel}/{}", e.name) });
        match fs.lstat(&p).map(|s| s.kind) {
            Ok(Kind::Dir) => foreign_modules(fs, &p, &r, over, out),
            Ok(Kind::File) if e.name.ends_with(".node") && is_macho(fs, &p) && fs.lstat(&format!("{over}/{r}")).is_err() => out.push(r),
            _ => {}
        }
    }
}

/// Build the launch folder for an Electron bundle and its command line. Notes go to `diag`.
pub fn prepare(fs: &mut dyn LinFs, b: &Bundle, want: &str, argv: &[String], envp: &[String], diag: &mut String) -> Result<Launch, i64> {
    let list = runtimes(fs);
    let Some((rt, have)) = pick(&list, want) else {
        *diag += &format!("prism: {} needs Electron {want} for Linux; install it in {RUNTIMES}/{}/ (none found)\n", b.exe_path(), major(want).unwrap_or(0));
        return Err(loos_lin::errno::ENOENT);
    };
    if major(&have) != major(want) {
        *diag += &format!("prism: {} is built on Electron {want}; running it on Electron {have} (install {RUNTIMES}/{}/ for an exact match)\n", b.exe_path(), major(want).unwrap_or(0));
    }
    let name = b.executable.replace('/', "_");
    let dir = format!("{LAUNCH}/{name}");
    remove_tree(fs, &dir);
    ensure_dir(fs, &dir)?;
    // The executable is linked, not symlinked: Linux resolves /proc/self/exe through symlinks, and
    // Chromium looks for its files (and Electron for `resources`) next to it.
    let exe = format!("{dir}/{}", name.to_lowercase().replace(' ', "-"));
    let real = resolve(fs, "/", &format!("{rt}/electron"), true)?;
    fs.link(&real, &exe)?;
    for e in fs.list(&rt)? {
        if e.name == "electron" || e.name == "resources" || e.name == "version" || resolve(fs, "/", &format!("{rt}/{}", e.name), true).ok().as_deref() == Some(real.as_str()) {
            continue;
        }
        fs.symlink(&format!("{rt}/{}", e.name), &format!("{dir}/{}", e.name))?;
    }
    // `resources` mirrors the bundle's; its own `app` folder (VS Code has one) moves to `app.mac`
    // to make room for the loader.
    let res = format!("{}/Contents/Resources", b.path.trim_end_matches('/'));
    let over = b.identifier.as_ref().map(|id| format!("{RUNTIMES}/apps/{id}")).filter(|o| fs.lstat(o).is_ok());
    let rdir = format!("{dir}/resources");
    fs.mkdir(&rdir, 0o755)?;
    // Electron versions differ in whether `app.asar` or `app` comes first, so the loader is both;
    // the app's own archive moves to `mac/app.asar` (keeping its `app.asar.unpacked` neighbour).
    mirror_children(fs, &res, over.as_deref(), &rdir, |n| match n {
        "app" => "app.mac".into(),
        "app.asar" => "mac.app.asar".into(),
        _ => n.to_string(),
    })?;
    if fs.lstat(&format!("{rdir}/mac.app.asar")).is_ok() {
        fs.mkdir(&format!("{rdir}/mac"), 0o755)?;
        fs.rename(&format!("{rdir}/mac.app.asar"), &format!("{rdir}/mac/app.asar"))?;
        let o = over.as_ref().map(|o| format!("{o}/app.asar.unpacked"));
        if fs.lstat(&format!("{res}/app.asar.unpacked")).is_ok() {
            mirror(fs, &format!("{res}/app.asar.unpacked"), o.as_deref(), &format!("{rdir}/mac/app.asar.unpacked"))?;
        }
    }
    let files: [(&str, &[u8]); 2] = [("package.json", SHIM_PACKAGE.as_bytes()), ("prism.js", SHIM.as_bytes())];
    write_new(fs, &format!("{rdir}/app.asar"), &asar(&files))?;
    fs.mkdir(&format!("{rdir}/app"), 0o755)?;
    for (n, d) in files {
        write_new(fs, &format!("{rdir}/app/{n}"), d)?;
    }
    let mut missing = Vec::new();
    foreign_modules(fs, &res, "", over.as_deref().unwrap_or("/nonexistent"), &mut missing);
    for m in &missing {
        *diag += &format!("prism: {m} is a macOS native module without a Linux build; the app runs without it\n");
    }
    let mut a = vec![exe.clone()];
    a.extend(SWITCHES.iter().map(|s| s.to_string()));
    a.extend(argv.iter().skip(1).cloned());
    let mut env: Vec<String> = envp.iter().filter(|e| !e.starts_with("ELECTRON_DISABLE_SANDBOX=")).cloned().collect();
    env.push("ELECTRON_DISABLE_SANDBOX=1".into());
    Ok(Launch { exe, argv: a, envp: env })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loos_lin::fs::MemFs;

    const FW_PLIST: &str = "<?xml version=\"1.0\"?><plist><dict><key>CFBundleVersion</key><string>44.9.3</string></dict></plist>";

    fn app(fs: &mut MemFs) -> Bundle {
        let b = "/Applications/Chat.app/Contents";
        fs.add_file(&format!("{b}/Info.plist"), b"<plist><dict><key>CFBundleExecutable</key><string>Chat</string><key>CFBundleIdentifier</key><string>org.example.chat</string></dict></plist>", 0o644);
        fs.add_file(&format!("{b}/MacOS/Chat"), &[0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1], 0o755);
        fs.add_file(&format!("{b}/Frameworks/Electron Framework.framework/Versions/A/Resources/Info.plist"), FW_PLIST.as_bytes(), 0o644);
        fs.symlink("Versions/A/Resources", &format!("{b}/Frameworks/Electron Framework.framework/Resources")).unwrap();
        fs.add_file(&format!("{b}/Resources/app.asar"), b"asar", 0o644);
        fs.add_file(&format!("{b}/Resources/app.asar.unpacked/m/pty.node"), &[0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1], 0o644);
        fs.add_file(&format!("{b}/Resources/app.asar.unpacked/m/up.node"), &[0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1], 0o644);
        crate::bundle::open(fs, "/Applications/Chat.app").unwrap()
    }

    fn runtime(fs: &mut MemFs, dir: &str, version: Option<&str>) {
        fs.add_file(&format!("{dir}/electron"), b"\x7fELF-runtime", 0o755);
        fs.add_file(&format!("{dir}/icudtl.dat"), b"icu", 0o644);
        fs.add_file(&format!("{dir}/resources/default_app.asar"), b"x", 0o644);
        if let Some(v) = version {
            fs.add_file(&format!("{dir}/version"), v.as_bytes(), 0o644);
        }
    }

    #[test]
    fn picks_a_runtime_and_builds_the_launch_folder() {
        let mut fs = MemFs::new();
        let b = app(&mut fs);
        assert_eq!(version(&mut fs, &b).as_deref(), Some("44.9.3"));
        runtime(&mut fs, "/usr/lib/electron43", None);
        runtime(&mut fs, "/usr/lib/prism/electron/44", Some("v44.10.0"));
        fs.add_file("/usr/lib/prism/electron/apps/org.example.chat/app.asar.unpacked/m/pty.node", b"\x7fELF-pty", 0o644);
        let mut diag = String::new();
        let l = prepare(&mut fs, &b, "44.9.3", &["Chat".into(), "--x".into()], &["HOME=/home".into()], &mut diag).unwrap();
        assert_eq!(l.exe, "/run/prism/electron/Chat/chat");
        assert_eq!(l.argv.first().map(String::as_str), Some("/run/prism/electron/Chat/chat"));
        assert_eq!(l.argv.last().map(String::as_str), Some("--x"));
        assert!(l.envp.contains(&"ELECTRON_DISABLE_SANDBOX=1".to_string()));
        let rd = |fs: &mut MemFs, p: &str| {
            let r = resolve(fs, "/", p, true).unwrap();
            crate::loader::read_file(fs, &r).unwrap()
        };
        assert_eq!(rd(&mut fs, &l.exe), b"\x7fELF-runtime");
        assert_eq!(fs.lstat(&l.exe).unwrap().kind, Kind::File, "a real file, so /proc/self/exe stays in the folder");
        assert_eq!(rd(&mut fs, "/run/prism/electron/Chat/icudtl.dat"), b"icu");
        assert_eq!(rd(&mut fs, "/run/prism/electron/Chat/resources/mac/app.asar"), b"asar");
        assert_eq!(rd(&mut fs, "/run/prism/electron/Chat/resources/mac/app.asar.unpacked/m/pty.node"), b"\x7fELF-pty");
        let shim = rd(&mut fs, "/run/prism/electron/Chat/resources/app.asar");
        assert_eq!(u32::from_le_bytes(shim[4..8].try_into().unwrap()) as usize % 4, 0);
        assert!(String::from_utf8_lossy(&shim).contains("\"prism.js\":{\"size\":"));
        assert!(String::from_utf8(rd(&mut fs, "/run/prism/electron/Chat/resources/app/package.json")).unwrap().contains("prism.js"));
        assert!(String::from_utf8(rd(&mut fs, "/run/prism/electron/Chat/resources/app/prism.js")).unwrap().contains("app.setAppPath"));
        assert_eq!(rd(&mut fs, "/run/prism/electron/Chat/resources/app.asar.unpacked/m/pty.node"), b"\x7fELF-pty");
        assert!(diag.contains("m/up.node is a macOS native module"), "{diag}");
        assert!(!diag.contains("pty.node"), "{diag}");
        assert!(!diag.contains("running it on"), "{diag}");
        // A second launch rebuilds the folder.
        prepare(&mut fs, &b, "44.9.3", &["Chat".into()], &[], &mut diag).unwrap();
    }

    #[test]
    fn falls_back_to_the_nearest_major_and_says_so() {
        let mut fs = MemFs::new();
        let b = app(&mut fs);
        runtime(&mut fs, "/usr/lib/electron42", None);
        runtime(&mut fs, "/usr/lib/electron46", None);
        let mut diag = String::new();
        prepare(&mut fs, &b, "44.9.3", &["Chat".into()], &[], &mut diag).unwrap();
        assert!(diag.contains("running it on Electron 46"), "{diag}");
        let mut none = MemFs::new();
        let b = app(&mut none);
        assert!(prepare(&mut none, &b, "44.9.3", &["Chat".into()], &[], &mut diag).is_err());
        assert!(diag.contains("needs Electron 44.9.3 for Linux"));
    }
}
