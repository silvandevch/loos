//! The whole LoOS desktop as one portable object (was `src/shell/desktop.c`): compositor, window manager, taskbar,
//! launcher and the built-in apps drawing into a caller-owned back buffer. No hardware access: the kernel feeds PS/2
//! input and presents the back buffer; host tests feed scripted input. The `desktop_*` C entry points at the bottom are
//! the only surface the kernel host (`src/kernel/boot/shellhost.c`) sees.
#![allow(missing_docs, clippy::missing_safety_doc)]

use crate::apphost::{AppWindow, FrameFn};
use crate::backend::{cfg_bool, cfg_int, cfg_set_bool, cfg_set_int, ShellBackend};
use crate::backend_mem::MemBackend;
use crate::compositor::{Compositor, MOD_ALT, MOD_CTRL, MOD_SUPER, WF_NOCHROME, WF_NORESIZE, WF_NOTASKBAR};
use crate::draw::Rect;
use crate::ffi::{c_bytes, Fb};
use crate::filemgr::ShellFilemgr;
use crate::installer::{ShellInstBackend, ShellInstaller};
use crate::inventory::{ShellInventory, ShellItem, ITEM_SETTING};
use crate::launcher::ShellLauncher;
use crate::session::{ShellAccounts, ShellLogin, ShellSession};
use crate::settings::ShellSettings;
use crate::taskbar::{ShellTaskbar, QS_SETTINGS};
use crate::taskmon::ShellTaskmon;
use crate::ui::UiCtx;
use crate::wm::ShellWm;
use alloc::boxed::Box;
use core::ffi::{c_char, c_void};
use core::mem::MaybeUninit;

pub const MODE_NORMAL: i32 = 0;
pub const MODE_INSTALLER: i32 = 1;
pub const MODE_LOGIN: i32 = 2;

/// App slots: index into [`Desktop::aw`], [`Desktop::surf`] and [`Desktop::app`].
const A_FILES: usize = 0;
const A_SETTINGS: usize = 1;
const A_TASKS: usize = 2;
const A_NOTEPAD: usize = 3;
const A_TERM: usize = 4;
const A_CALC: usize = 5;
const A_BROWSER: usize = 6;
const NAPPS: usize = 7;

pub type AllocFn = unsafe extern "C" fn(usize, *mut c_void) -> *mut c_void;
pub type LogFn = unsafe extern "C" fn(*const c_char, *mut c_void);

/// All desktop state. Zero is a valid initial value (the kernel keeps one in `.bss`); the only non-POD field is
/// `mem`, an `Option<Box<_>>` whose all-zero pattern is `None`.
pub struct Desktop {
    pub mode: i32,
    pub installer: ShellInstaller,
    pub aw_installer: AppWindow,
    pub accounts: ShellAccounts,
    pub session: ShellSession,
    pub login: ShellLogin,
    pub aw_login: AppWindow,
    pub comp: Compositor,
    pub wm: ShellWm,
    pub inv: ShellInventory,
    pub launcher: ShellLauncher,
    pub taskbar: ShellTaskbar,
    pub mem: Option<Box<MemBackend>>,
    pub be: ShellBackend,
    pub settings: ShellSettings,
    pub files: ShellFilemgr,
    pub tasks: ShellTaskmon,
    pub fb: Fb,
    pub aw: [AppWindow; NAPPS],
    pub surf: [*mut u32; NAPPS],
    pub app: [*mut c_void; NAPPS],
    pub alloc: Option<AllocFn>,
    pub alloc_aux: *mut c_void,
    pub mx: i32,
    pub my: i32,
    pub buttons: u32,
    pub log: Option<LogFn>,
    pub aux: *mut c_void,
    /// `settings.changes` last seen by [`Desktop::sync_look`].
    pub seen_changes: i32,
    pub night: i32,
}

fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

/// UI scale (percent) that gives a comfortable size at this resolution.
pub fn auto_scale(w: i32, h: i32) -> i32 {
    let m = w.min(h);
    if m >= 2000 { 200 } else if m >= 1300 { 150 } else { 100 }
}

unsafe extern "C" fn files_cb(a: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    unsafe { crate::filemgr::fm_frame(a as *mut ShellFilemgr, u, w, h) }
}
unsafe extern "C" fn settings_cb(a: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    unsafe { crate::settings::settings_frame(a as *mut ShellSettings, u, w, h) }
}
unsafe extern "C" fn tasks_cb(a: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    unsafe { crate::taskmon::tm_frame(a as *mut ShellTaskmon, u, w, h) }
}
unsafe extern "C" fn installer_cb(a: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    unsafe { crate::installer::installer_frame(a as *mut ShellInstaller, u, w, h) }
}
unsafe extern "C" fn login_cb(a: *mut c_void, u: *mut UiCtx, w: i32, h: i32) {
    unsafe { crate::session::login_frame(a as *mut ShellLogin, u, w, h) }
}

unsafe extern "C" fn on_quick(action: i32, value: u32, user: *mut c_void) {
    let d = unsafe { &mut *(user as *mut Desktop) };
    if action == QS_SETTINGS {
        d.open(A_SETTINGS);
    } else {
        crate::settings::quick(&mut d.settings, action, value);
        d.sync_look();
    }
}
unsafe extern "C" fn on_net_toggle(off: bool, user: *mut c_void) {
    let d = unsafe { &mut *(user as *mut Desktop) };
    d.be_ref().net_off_set(off);
    crate::settings::apply(&mut d.settings);
}
unsafe extern "C" fn launch(it: *const ShellItem, user: *mut c_void) {
    let (d, it) = unsafe { (&mut *(user as *mut Desktop), &*it) };
    let target = nul(&it.target);
    if it.kind == ITEM_SETTING {
        if let Some(pg) = crate::settings::page_by_id(target) {
            d.settings.page = pg;
            d.open(A_SETTINGS);
            return;
        }
    }
    if target.starts_with(b"look:") || target.starts_with(b"act:") {
        d.run_action(target);
        return;
    }
    let slot = match target {
        b"loos-taskmgr.exe" => A_TASKS,
        b"loos-notepad.exe" => A_NOTEPAD,
        b"loos-cmd.exe" => A_TERM,
        b"calc.exe" => A_CALC,
        b"loos-browser.exe" => A_BROWSER,
        b"files" => A_FILES,
        _ => {
            if let Some(log) = d.log {
                let mut line = [0u8; 160];
                let msg = b"launch: no process loader yet: ";
                line[..msg.len()].copy_from_slice(msg);
                let n = target.len().min(line.len() - msg.len() - 1);
                line[msg.len()..msg.len() + n].copy_from_slice(&target[..n]);
                // SAFETY: line is NUL-terminated (zero-initialised, last byte untouched).
                unsafe { log(line.as_ptr() as *const c_char, d.aux) };
            }
            return;
        }
    };
    d.open(slot);
}

/// The Rust apps declare `UiCtx` as an opaque pointee of their own; it is the same `ui::UiCtx` the toolkit passes.
fn app_frame(f: *const ()) -> FrameFn {
    // SAFETY: the four app frame functions have exactly the FrameFn ABI (pointer, ctx pointer, w, h); only the pointee's nominal type differs.
    unsafe { core::mem::transmute::<*const (), FrameFn>(f) }
}

fn nul(a: &[u8]) -> &[u8] {
    &a[..a.iter().position(|&b| b == 0).unwrap_or(a.len())]
}

impl Desktop {
    /// A zeroed desktop on the heap (host tests; the kernel uses the `.bss` instance).
    pub fn boxed() -> Box<Desktop> {
        // SAFETY: Desktop's all-zero pattern is valid (see the type docs).
        unsafe { Box::new_zeroed().assume_init() }
    }

    fn be_ref(&self) -> &ShellBackend {
        &self.be
    }

    fn reset(&mut self) {
        drop(self.mem.take());
        let (log, aux) = (self.log, self.aux);
        // SAFETY: all-zero is a valid Desktop; `mem` was just taken.
        unsafe { core::ptr::write_bytes(self as *mut Desktop, 0, 1) };
        self.log = log;
        self.aux = aux;
    }

    unsafe fn alloc_px(&self, bytes: usize) -> *mut u32 {
        match self.alloc {
            Some(f) => unsafe { f(bytes, self.alloc_aux) as *mut u32 },
            None => core::ptr::null_mut(),
        }
    }
    fn surf_bytes(&self) -> usize {
        self.fb.w as usize * self.fb.h as usize * 4
    }

    // ---- window placement (designed in logical px, converted to device px)
    fn arrange(&self, which: i32) -> Rect {
        let sc = self.comp.scale_pct as i32;
        let (lw, lh) = (self.comp.lw(), self.comp.lh() - crate::taskbar::taskbar_h());
        let mut r = match which {
            0 => Rect::new(lw * 3 / 100, lh * 4 / 100, clampi(lw * 40 / 100, 520, 760), clampi(lh * 52 / 100, 360, 560)),
            1 => Rect::new(lw * 36 / 100, lh * 12 / 100, clampi(lw * 42 / 100, 560, 780), clampi(lh * 66 / 100, 440, 640)),
            _ => Rect::new(lw * 6 / 100, lh * 48 / 100, clampi(lw * 46 / 100, 620, 840), clampi(lh * 44 / 100, 320, 460)),
        };
        if r.x + r.w > lw - 8 { r.x = lw - 8 - r.w }
        if r.y + r.h > lh - 8 { r.y = lh - 8 - r.h }
        r.x = r.x.max(0);
        r.y = r.y.max(0);
        Rect::new(r.x * sc / 100, r.y * sc / 100, r.w * sc / 100, r.h * sc / 100)
    }
    fn centered(&self, w: i32, h: i32, nudge: i32) -> Rect {
        let sc = self.comp.scale_pct as i32;
        let (lw, lh) = (self.comp.lw(), self.comp.lh() - crate::taskbar::taskbar_h());
        let (w, h) = (clampi(w, 200, lw - 16), clampi(h, 160, lh - 16));
        let x = clampi((lw - w) / 2 + nudge, 0, lw - w);
        let y = clampi((lh - h) / 2 + nudge / 2, 0, lh - h);
        Rect::new(x * sc / 100, y * sc / 100, w * sc / 100, h * sc / 100)
    }

    fn focus_existing(&mut self, slot: usize) {
        let win = self.aw[slot].win;
        self.comp.restore(win);
        self.comp.raise(win);
        self.comp.focus(win);
    }

    /// Command-palette actions: `look:<id>` applies a Look, `act:<name>` flips one behaviour key.
    pub fn run_action(&mut self, target: &[u8]) {
        let b = &self.be;
        if let Some(id) = target.strip_prefix(b"look:") {
            if let Some(i) = core::str::from_utf8(id).ok().and_then(crate::look::look_index) {
                crate::look::store_look(b, &crate::look::LOOKS[i]);
            }
        } else {
            match target {
                b"act:tiling" => cfg_set_bool(b, b"wm.tiling\0", !cfg_bool(b, b"wm.tiling\0", false)),
                b"act:focus" => cfg_set_int(b, b"wm.focus\0", 1 - cfg_int(b, b"wm.focus\0", 0).clamp(0, 1)),
                b"act:night" => cfg_set_int(b, b"ui.night\0", if cfg_int(b, b"ui.night\0", 0) > 0 { 0 } else { 60 }),
                b"act:motion" => cfg_set_bool(b, b"ui.reduce_motion\0", !cfg_bool(b, b"ui.reduce_motion\0", false)),
                b"act:network" => b.net_off_set(!b.net_off_get()),
                _ => return,
            }
        }
        self.settings.changes += 1;
        crate::settings::apply(&mut self.settings);
        self.sync_look();
    }

    /// Applies a Look by id (kernel command line `loos.look=<id>`).
    pub fn apply_look(&mut self, id: &[u8]) -> bool {
        let mut t = [0u8; 40];
        t[..5].copy_from_slice(b"look:");
        let n = id.len().min(30);
        t[5..5 + n].copy_from_slice(&id[..n]);
        let ok = core::str::from_utf8(&id[..n]).ok().and_then(crate::look::look_index).is_some();
        if ok {
            self.run_action(&t[..5 + n]);
        }
        ok
    }

    fn seed_palette(&mut self) {
        use crate::inventory::{add, ITEM_APP};
        let inv = &mut self.inv;
        add(inv, b"Files", b"files", b"file manager folders browse btrfs", ITEM_APP);
        add(inv, b"Settings", b"settings:privacy", b"preferences control panel", ITEM_SETTING);
        for l in crate::look::LOOKS.iter() {
            let (mut name, mut tgt) = ([0u8; 48], [0u8; 40]);
            let nm = [b"Look: ".as_slice(), l.name.as_bytes()].concat();
            name[..nm.len().min(47)].copy_from_slice(&nm[..nm.len().min(47)]);
            let tg = [b"look:".as_slice(), l.id.as_bytes()].concat();
            tgt[..tg.len()].copy_from_slice(&tg);
            add(inv, &name[..nm.len().min(47)], &tgt[..tg.len()], b"theme preset style appearance look", ITEM_SETTING);
        }
        for (n, t, k) in [
            (&b"Toggle tiling"[..], &b"act:tiling"[..], &b"windows tile tiler layout"[..]),
            (b"Toggle focus follows mouse", b"act:focus", b"sloppy focus windows"),
            (b"Toggle night light", b"act:night", b"blue light warm eyes evening display"),
            (b"Toggle reduce motion", b"act:motion", b"animations accessibility"),
            (b"Toggle network (hard switch)", b"act:network", b"offline airplane block internet privacy"),
        ] {
            add(inv, n, t, k, ITEM_SETTING);
        }
    }

    /// Opens (or focuses) one of the built-in apps.
    pub fn open(&mut self, slot: usize) {
        if self.aw[slot].is_open() {
            return self.focus_existing(slot);
        }
        let (title, owner, frame, rect): (&[u8], u32, FrameFn, Rect) = match slot {
            A_FILES => (b"Files", 100, files_cb as FrameFn, self.arrange(0)),
            A_SETTINGS => (b"Settings", 101, settings_cb as FrameFn, self.arrange(1)),
            A_TASKS => (b"Task Manager", 102, tasks_cb as FrameFn, self.arrange(2)),
            A_NOTEPAD => (b"Notepad", 103, app_frame(crate::apps::loos_notepad_frame as *const ()), self.centered(720, 480, 40)),
            A_TERM => (b"LoCmd", 104, app_frame(crate::apps::loos_term_frame as *const ()), self.centered(760, 440, -60)),
            A_CALC => (b"Calculator", 105, app_frame(crate::apps::loos_calc_frame as *const ()), self.centered(320, 480, 200)),
            _ => (b"LoOS Browser", 106, app_frame(crate::apps::browser::loos_browser_frame as *const ()), self.centered(1000, 720, 0)),
        };
        // Rust apps are created on first open and live for the session (their state survives closing the window).
        if self.app[slot].is_null() {
            let be = &self.be as *const ShellBackend as *const c_void;
            self.app[slot] = match slot {
                A_FILES => &mut self.files as *mut _ as *mut c_void,
                A_SETTINGS => &mut self.settings as *mut _ as *mut c_void,
                A_TASKS => &mut self.tasks as *mut _ as *mut c_void,
                A_NOTEPAD => crate::apps::loos_notepad_new(be, c"/home/user/README.txt".as_ptr()) as *mut c_void,
                A_TERM => crate::apps::loos_term_new(be) as *mut c_void,
                A_CALC => crate::apps::loos_calc_new() as *mut c_void,
                _ => crate::apps::browser::loos_browser_new(be, c"loos://home".as_ptr()) as *mut c_void,
            };
        }
        if self.surf[slot].is_null() {
            self.surf[slot] = unsafe { self.alloc_px(self.surf_bytes()) };
        }
        if self.surf[slot].is_null() || self.app[slot].is_null() {
            return;
        }
        let (c, px, app, cw, ch) = (&mut self.comp as *mut Compositor, self.surf[slot], self.app[slot], self.fb.w, self.fb.h);
        // SAFETY: the compositor, surface and app all live as long as the desktop.
        unsafe { self.aw[slot].open(c, title, rect, 0, owner, px, cw, ch, frame, app) };
    }

    /// Hands a local page to the browser (opening it first if needed).
    pub fn open_url(&mut self, url: &[u8]) {
        self.open(A_BROWSER);
        if !self.app[A_BROWSER].is_null() && !url.is_empty() {
            let mut z = [0u8; 260];
            let n = url.len().min(259);
            z[..n].copy_from_slice(&url[..n]);
            unsafe { crate::apps::browser::loos_browser_navigate_to(self.app[A_BROWSER], z.as_ptr() as *const c_char) };
        }
        if self.aw[A_BROWSER].is_open() {
            let win = self.aw[A_BROWSER].win;
            self.comp.raise(win);
            self.comp.focus(win);
        }
    }

    /// The in-memory backend filled with demo volumes, files and processes until the real services exist.
    fn seed_backend(&mut self) {
        let mut m = Box::new(MemBackend::new());
        m.add_volume("/", "LoOS", "btrfs", 500 << 30, 320 << 30, false);
        m.add_volume("/boot/efi", "ESP", "vfat", 512 << 20, 400 << 20, false);
        for (p, dir, sz, t) in [
            ("/home", true, 0, 1_700_000_000),
            ("/home/user", true, 0, 1_700_000_100),
            ("/home/user/Documents", true, 0, 1_700_000_200),
            ("/home/user/Documents/report.txt", false, 2048, 1_700_001_000),
            ("/home/user/Documents/notes.md", false, 512, 1_700_002_000),
            ("/home/user/Music", true, 0, 1_700_000_300),
            ("/home/user/Music/song.ogg", false, 4 << 20, 1_700_003_000),
            ("/home/user/Pictures", true, 0, 1_700_000_350),
            ("/home/user/README.txt", false, 1337, 1_700_009_999),
            ("/home/user/.config", true, 0, 1_700_000_400),
            ("/home/user/Web", true, 0, 1_700_000_500),
            ("/etc", true, 0, 1_700_000_000),
            ("/etc/loos.conf", false, 220, 1_700_000_000),
        ] {
            m.add_node(p, dir, sz, t, 0);
        }
        m.put_file("/home/user/README.txt", README.as_bytes());
        m.put_file("/home/user/Web/index.html", WEB_INDEX.as_bytes());
        m.put_file("/home/user/Web/guide.html", WEB_GUIDE.as_bytes());
        m.put_file("/home/user/Web/privacy.html", WEB_PRIVACY.as_bytes());
        m.add_proc(1, 0, "system", 12, 18 << 20, 5);
        m.add_proc(4, 1, "loos-shell", 4, 42 << 20, 12);
        m.add_proc(9, 4, "loos-notepad.exe", 2, 16 << 20, 0);
        m.add_proc(12, 1, "loos-netd", 3, 9 << 20, 1);
        for a in ["loos-notepad.exe", "calc.exe", "loos-browser.exe"] {
            m.add_app(a);
        }
        self.be = m.vtable();
        self.mem = Some(m);
    }

    pub fn init(&mut self, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void) -> bool {
        if !(640..=8192).contains(&w) || !(400..=8192).contains(&h) {
            return false;
        }
        self.reset();
        self.alloc = Some(alloc);
        self.alloc_aux = aux;
        self.fb = Fb { px: backbuf, w, h, stride: w, scale: 100 };
        let surf = self.surf_bytes();
        for s in [A_FILES, A_SETTINGS, A_TASKS] {
            self.surf[s] = unsafe { self.alloc_px(surf) };
            if self.surf[s].is_null() {
                return false;
            }
        }
        let me = self as *mut Desktop as *mut c_void;
        self.comp.init(w, h);
        self.comp.set_scale(if scale_pct > 0 { scale_pct } else { auto_scale(w, h) } as u32);
        crate::wm::init(&mut self.wm, &mut self.comp);
        unsafe { crate::wm::wm_install(&mut self.wm) };
        self.seed_backend();
        cfg_set_int(&self.be, b"ui.scale\0", self.comp.scale_pct as i32); // Settings reads the scale back
        unsafe {
            crate::inventory::inventory_defaults(&mut self.inv);
            self.seed_palette();
            crate::launcher::launcher_init(&mut self.launcher, &self.inv, Some(launch), me);
            crate::taskbar::taskbar_init(&mut self.taskbar, &mut self.comp, &mut self.launcher);
        }
        let b = &self.be;
        unsafe {
            crate::taskbar::taskbar_configure(
                &mut self.taskbar,
                cfg_bool(b, b"ui.taskbar.center\0", true),
                cfg_bool(b, b"ui.taskbar.labels\0", false),
                cfg_bool(b, b"ui.taskbar.top\0", false),
                cfg_int(b, b"ui.taskbar.size\0", 1),
            );
        }
        self.taskbar.on_quick = Some(on_quick);
        self.taskbar.on_net_toggle = Some(on_net_toggle);
        self.taskbar.user = me;
        let wall = unsafe { self.alloc_px(surf) };
        self.comp.set_wallpaper_cache(wall); // null is fine: the wallpaper is then drawn live
        crate::settings::init(&mut self.settings, &self.be, &mut self.comp, &mut self.taskbar);
        unsafe {
            crate::filemgr::fm_init(&mut self.files, &self.be, c"/home/user".as_ptr());
            crate::taskmon::tm_init(&mut self.tasks, &self.be);
        }
        self.app[A_FILES] = &mut self.files as *mut _ as *mut c_void;
        self.app[A_SETTINGS] = &mut self.settings as *mut _ as *mut c_void;
        self.app[A_TASKS] = &mut self.tasks as *mut _ as *mut c_void;
        self.open(A_FILES);
        self.open(A_TASKS);
        self.open(A_SETTINGS);
        self.seen_changes = -1;
        self.sync_look();
        self.mx = w / 2;
        self.my = h / 2;
        true
    }

    fn init_fullscreen(&mut self, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void) -> bool {
        if !(640..=8192).contains(&w) || !(400..=8192).contains(&h) {
            return false;
        }
        self.reset();
        self.alloc = Some(alloc);
        self.alloc_aux = aux;
        self.fb = Fb { px: backbuf, w, h, stride: w, scale: 100 };
        self.surf[A_FILES] = unsafe { self.alloc_px(self.surf_bytes()) }; // one surface for the single app
        if self.surf[A_FILES].is_null() {
            return false;
        }
        self.comp.init(w, h);
        self.comp.set_scale(if scale_pct > 0 { scale_pct } else { auto_scale(w, h) } as u32);
        self.mx = w / 2;
        self.my = h / 2;
        true
    }

    pub unsafe fn init_installer(&mut self, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void, be: *const ShellInstBackend) -> bool {
        if !self.init_fullscreen(backbuf, w, h, scale_pct, alloc, aux) {
            return false;
        }
        self.mode = MODE_INSTALLER;
        unsafe { crate::installer::installer_init(&mut self.installer, be) };
        let (c, px) = (&mut self.comp as *mut Compositor, self.surf[A_FILES]);
        let app = &mut self.installer as *mut _ as *mut c_void;
        unsafe { self.aw_installer.open(c, b"LoOS Setup", Rect::new(0, 0, w, h), WF_NOCHROME | WF_NOTASKBAR | WF_NORESIZE, 200, px, w, h, installer_cb, app) }
    }

    pub unsafe fn init_login(&mut self, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void, accounts: *const c_char, iters: u32) -> bool {
        if !self.init_fullscreen(backbuf, w, h, scale_pct, alloc, aux) {
            return false;
        }
        self.mode = MODE_LOGIN;
        unsafe {
            crate::session::accounts_init(&mut self.accounts, iters);
            if crate::session::accounts_load(&mut self.accounts, accounts) < 1 {
                return false;
            }
            crate::session::session_init(&mut self.session, &mut self.accounts);
            crate::session::login_init(&mut self.login, &mut self.session);
        }
        let (c, px) = (&mut self.comp as *mut Compositor, self.surf[A_FILES]);
        let app = &mut self.login as *mut _ as *mut c_void;
        unsafe { self.aw_login.open(c, b"Sign in", Rect::new(0, 0, w, h), WF_NOCHROME | WF_NOTASKBAR | WF_NORESIZE, 201, px, w, h, login_cb, app) }
    }

    /// Applies machine configuration lines (`key=value`, up to an optional `[accounts]` section).
    pub fn apply_config(&mut self, text: &[u8]) {
        if self.mode != MODE_NORMAL {
            return;
        }
        for line in text.split(|&c| c == b'\n') {
            if line.first() == Some(&b'[') {
                break; // section headers start the account list
            }
            let Some(eq) = line.iter().position(|&c| c == b'=') else { continue };
            let (k, v) = (&line[..eq], &line[eq + 1..]);
            if eq == 0 || k.len() >= 48 || v.len() >= 64 {
                continue;
            }
            if matches!(k, b"net.hard_off" | b"updates.enabled" | b"hostname" | b"display") || k.starts_with(b"ui.") || k.starts_with(b"wm.") {
                let (mut kz, mut vz) = ([0u8; 49], [0u8; 65]);
                kz[..k.len()].copy_from_slice(k);
                vz[..v.len()].copy_from_slice(v);
                self.be.cfg_set(&kz, &vz);
            }
        }
        self.settings.changes += 1;
        crate::settings::apply(&mut self.settings);
        self.sync_look();
    }

    pub fn pointer(&mut self, dx: i32, dy: i32, buttons: u32) {
        let (w, h) = (self.comp.screen_w, self.comp.screen_h);
        self.mx = clampi(self.mx + dx, 0, w - 1);
        self.my = clampi(self.my + dy, 0, h - 1);
        let shell_ui = self.mode == MODE_NORMAL; // taskbar + window manager exist only on the desktop
        let (c, tb, wm) = (&mut self.comp as *mut Compositor, &mut self.taskbar as *mut ShellTaskbar, &mut self.wm as *mut ShellWm);
        // SAFETY: the three objects are fields of self and distinct; the callees only touch their own state plus the compositor.
        unsafe {
            if dx != 0 || dy != 0 {
                if shell_ui {
                    crate::taskbar::taskbar_mouse(tb, c, self.mx, self.my, 0, false); // hover
                }
                (*c).mouse_move(self.mx, self.my);
                if shell_ui {
                    crate::wm::wm_after_mouse(wm);
                }
            }
            let changed = buttons ^ self.buttons;
            for b in [1u32, 2, 4] {
                if changed & b == 0 {
                    continue;
                }
                let down = buttons & b != 0;
                if !shell_ui || !crate::taskbar::taskbar_mouse(tb, c, self.mx, self.my, b as i32, down) {
                    (*c).mouse_button(b as i32, down);
                    if shell_ui {
                        crate::wm::wm_after_mouse(wm);
                    }
                }
            }
        }
        self.buttons = buttons;
    }

    pub fn key(&mut self, key: u32, ch: u32, mods: u32, down: bool) {
        let (c, tb, wm) = (&mut self.comp as *mut Compositor, &mut self.taskbar as *mut ShellTaskbar, &mut self.wm as *mut ShellWm);
        // SAFETY: as in `pointer`.
        unsafe {
            if down && self.mode == MODE_NORMAL {
                let was_open = self.launcher.open;
                if crate::taskbar::taskbar_key(tb, c, key, true, mods) {
                    // modal while the launcher is open: it also gets the typed character
                    if was_open && self.launcher.open && ch != 0 && mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 {
                        crate::launcher::launcher_char(&mut self.launcher, ch);
                        (*c).damage_all();
                    }
                    return;
                }
                if crate::wm::wm_key(wm, key, true, mods) {
                    cfg_set_bool(&self.be, b"wm.tiling\0", self.wm.tiling); // Super+T flips it: keep the key in step
                    return;
                }
            }
            (*c).key(key, down, mods);
            if down && ch != 0 && mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 {
                (*c).char_event(ch);
            }
        }
    }

    pub fn tick(&mut self, now_ms: u32, hour: i32, minute: i32) {
        self.comp.now_ms = now_ms;
        match self.mode {
            MODE_INSTALLER => {
                let before = self.installer.page;
                if before == crate::installer::IP_INSTALL {
                    self.aw_installer.dirty = true; // the progress page advances the install every frame
                }
                self.aw_installer.pump();
                if self.installer.page != before {
                    self.aw_installer.dirty = true; // the frame that finished (or failed) the install must repaint as the next page
                    self.aw_installer.pump();
                }
                return;
            }
            MODE_LOGIN => {
                if (self.login.now_ms / 1000) as u32 != now_ms / 1000 {
                    self.aw_login.dirty = true; // clock and lockout countdown tick once a second
                }
                self.login.now_ms = now_ms as u64;
                self.login.hour = hour;
                self.login.minute = minute;
                self.aw_login.pump();
                return;
            }
            _ => {}
        }
        for i in 0..NAPPS {
            if self.aw[i].is_open() {
                self.aw[i].pump();
            }
        }
        if self.aw[A_TERM].is_open() && !self.app[A_TERM].is_null() && unsafe { crate::apps::loos_term_take_exit(self.app[A_TERM]) } {
            self.aw[A_TERM].close();
        }
        unsafe { crate::taskbar::taskbar_set_time(&mut self.taskbar, hour, minute) };
        self.sync_look();
    }

    /// Repaints what changed into the back buffer; returns the dirty rectangle (device px), empty if nothing changed.
    pub fn render(&mut self) -> Rect {
        if self.comp.damage.n == 0 {
            return Rect::new(0, 0, 0, 0);
        }
        let r = self.comp.damage.bounds();
        self.comp.render(&mut self.fb);
        let r = clip_to_fb(r, &self.fb);
        crate::look::night_tint(&mut self.fb, r, self.night);
        r
    }
    pub fn render_all(&mut self) {
        self.comp.render_full(&mut self.fb);
        let all = Rect::new(0, 0, self.fb.w, self.fb.h);
        crate::look::night_tint(&mut self.fb, all, self.night);
    }

    /// Re-reads the behaviour keys (`wm.*`, `ui.night`, ...) when Settings reports a change.
    pub fn sync_look(&mut self) {
        if self.mode != MODE_NORMAL || self.seen_changes == self.settings.changes {
            return;
        }
        self.seen_changes = self.settings.changes;
        let night = crate::look::apply_behavior(&self.be, &mut self.wm, &mut self.comp, &mut self.taskbar);
        if night != self.night {
            self.night = night;
            self.comp.damage_all();
        }
    }
}

fn clip_to_fb(r: Rect, fb: &Fb) -> Rect {
    let i = r.intersect(Rect::new(0, 0, fb.w, fb.h));
    if i.is_empty() { Rect::new(i.x, i.y, 0, 0) } else { i }
}

// ---------------------------------------------------------------- seed content
const README: &str = "Welcome to LoOS.\n\n\
Everything stays on this computer: no telemetry, no account, no cloud.\n\n\
Try these:\n\
  - Super opens Start. Type to search apps, files and settings.\n\
  - LoCmd is the terminal. Type HELP. Windows paths work too: cd C:\\Users\\user\n\
  - Notepad edits text files. Ctrl+S saves.\n\
  - LoOS Browser opens local pages. Web addresses explain why they are offline.\n\
  - Click the clock for a calendar, the tray light for Quick Settings.\n\
  - Settings > Looks swaps the whole desktop style in one click; Settings > Behavior tunes tiling,\n\
    focus, snap, night light and the clock. Every value there is yours to change.\n";
const WEB_INDEX: &str = "<html><head><title>LoOS Local Site</title></head><body>\n\
<h1>LoOS local site</h1>\n\
<p>This sample site lives on the LoOS disk. It exists so the browser can be tested without a network.</p>\n\
<h2>Pages</h2><ul>\n\
<li><a href=\"guide.html\">Browser guide</a> - tabs, history and bookmarks</li>\n\
<li><a href=\"privacy.html\">Privacy</a> - what this browser does and does not do</li>\n\
</ul>\n\
<h2>Local-first</h2>\n\
<p>No accounts, no trackers, no remote start page. Bookmarks and history are files under \
<b>/home/user/.config/loos-browser</b>.</p>\n\
<h2>An external link</h2>\n\
<p><a href=\"https://example.com\">example.com</a> opens an offline explanation page; no request is made.</p>\n\
<pre>local site /home/user/Web\n\
offline by design</pre>\n\
</body></html>\n";
const WEB_GUIDE: &str = "<html><head><title>Browser Guide</title></head><body>\n\
<h1>Browser guide</h1>\n\
<p>The address bar accepts internal pages, local paths and web addresses.</p>\n\
<h2>Keys</h2><ul>\n\
<li><b>Ctrl+L</b> - focus the address bar</li>\n\
<li><b>Ctrl+T</b> / <b>Ctrl+W</b> - new / close tab</li>\n\
<li><b>Ctrl+R</b> - reload; <b>Ctrl+D</b> - bookmark this page</li>\n\
<li><b>Alt+Left</b> / <b>Alt+Right</b> - back / forward</li>\n\
<li><b>Page Up</b> / <b>Page Down</b> - scroll</li>\n\
</ul>\n\
<h2>Addresses</h2><ul>\n\
<li><b>loos://home</b> - start page</li>\n\
<li><b>loos://bookmarks</b> and <b>loos://history</b></li>\n\
<li><b>/home/user/Web/index.html</b> or <b>file:///home/user/Web/index.html</b></li>\n\
<li>A bare word searches bookmarks and /home/user/Web locally.</li>\n\
</ul>\n\
<p><a href=\"index.html\">Back to the local site</a></p>\n\
</body></html>\n";
const WEB_PRIVACY: &str = "<html><head><title>Privacy in this browser</title></head><body>\n\
<h1>Privacy</h1>\n\
<p>LoOS Browser has no socket code. It cannot send a request, a telemetry ping or a crash report.</p>\n\
<ul><li>No network: HTTP and HTTPS show a local explanation page.</li>\n\
<li>No telemetry and no accounts.</li>\n\
<li>History and bookmarks are plain files you can delete.</li>\n\
<li>No remote search provider. Search is local.</li></ul>\n\
<blockquote>Nothing leaves the machine unless the user started it, and this build cannot start it at all.</blockquote>\n\
<p><a href=\"index.html\">Back</a></p>\n\
</body></html>\n";

// ---------------------------------------------------------------- C ABI (src/kernel/boot/shellabi.h)
static mut INSTANCE: MaybeUninit<Desktop> = MaybeUninit::zeroed();

/// The single desktop object, kept in `.bss` (it is ~1 MB). All `desktop_*` calls take this pointer.
#[no_mangle]
pub extern "C" fn desktop_instance() -> *mut Desktop {
    // SAFETY: only the address is taken; the shell is single-threaded.
    unsafe { core::ptr::addr_of_mut!(INSTANCE) as *mut Desktop }
}

#[no_mangle]
pub unsafe extern "C" fn desktop_set_log(d: *mut Desktop, log: Option<LogFn>, aux: *mut c_void) {
    let d = unsafe { &mut *d };
    d.log = log;
    d.aux = aux;
}
/// Sign-in check used by the boot self-test. Returns the `auth_result` code (0 = ok, 1 = bad password, 2 = locked, 3 = no such user).
#[no_mangle]
pub unsafe extern "C" fn desktop_login_try(d: *mut Desktop, user: *const c_char, password: *const c_char, now_ms: u64) -> i32 {
    let d = unsafe { &mut *d };
    let mut retry = 0u32;
    unsafe { crate::session::session_login(&mut d.session, user, password, now_ms, &mut retry) }
}
#[no_mangle]
pub unsafe extern "C" fn desktop_apply_look(d: *mut Desktop, id: *const c_char) -> bool {
    unsafe { &mut *d }.apply_look(unsafe { c_bytes(id) })
}
#[no_mangle]
pub unsafe extern "C" fn desktop_theme_desktop(d: *const Desktop) -> u32 {
    unsafe { &*d }.comp.theme.desktop
}
#[no_mangle]
pub unsafe extern "C" fn desktop_window_count(d: *const Desktop) -> i32 {
    unsafe { &*d }.comp.nwin
}
#[no_mangle]
pub unsafe extern "C" fn desktop_scale_pct(d: *const Desktop) -> u32 {
    unsafe { &*d }.comp.scale_pct
}

#[no_mangle]
pub extern "C" fn desktop_auto_scale(w: i32, h: i32) -> i32 {
    auto_scale(w, h)
}
#[no_mangle]
pub unsafe extern "C" fn desktop_init(d: *mut Desktop, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void) -> bool {
    unsafe { &mut *d }.init(backbuf, w, h, scale_pct, alloc, aux)
}
#[no_mangle]
pub unsafe extern "C" fn desktop_init_installer(d: *mut Desktop, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void, be: *const ShellInstBackend) -> bool {
    unsafe { (&mut *d).init_installer(backbuf, w, h, scale_pct, alloc, aux, be) }
}
#[no_mangle]
pub unsafe extern "C" fn desktop_installer_wants_live(d: *const Desktop) -> bool {
    let d = unsafe { &*d };
    d.mode == MODE_INSTALLER && d.installer.live_requested
}
#[no_mangle]
pub unsafe extern "C" fn desktop_init_login(d: *mut Desktop, backbuf: *mut u32, w: i32, h: i32, scale_pct: i32, alloc: AllocFn, aux: *mut c_void, accounts: *const c_char, iters: u32) -> bool {
    unsafe { (&mut *d).init_login(backbuf, w, h, scale_pct, alloc, aux, accounts, iters) }
}
#[no_mangle]
pub unsafe extern "C" fn desktop_login_done(d: *const Desktop, user_out: *mut c_char, cap: usize) -> bool {
    let d = unsafe { &*d };
    if d.mode != MODE_LOGIN || d.session.state != 1 {
        return false;
    }
    if !user_out.is_null() && cap > 0 {
        let u = nul(&d.session.user);
        let n = u.len().min(cap - 1);
        // SAFETY: caller provides `cap` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(u.as_ptr(), user_out as *mut u8, n);
            *user_out.add(n) = 0;
        }
    }
    true
}
#[no_mangle]
pub unsafe extern "C" fn desktop_apply_config(d: *mut Desktop, text: *const c_char) {
    unsafe { &mut *d }.apply_config(unsafe { c_bytes(text) });
}
#[no_mangle]
pub unsafe extern "C" fn desktop_open_url(d: *mut Desktop, url: *const c_char) {
    unsafe { &mut *d }.open_url(unsafe { c_bytes(url) });
}
#[no_mangle]
pub unsafe extern "C" fn desktop_pointer(d: *mut Desktop, dx: i32, dy: i32, buttons: u32) {
    unsafe { &mut *d }.pointer(dx, dy, buttons);
}
#[no_mangle]
pub unsafe extern "C" fn desktop_pointer_to(d: *mut Desktop, x: i32, y: i32, buttons: u32) {
    let d = unsafe { &mut *d };
    let (dx, dy) = (x - d.mx, y - d.my);
    d.pointer(dx, dy, buttons);
}
#[no_mangle]
pub unsafe extern "C" fn desktop_key(d: *mut Desktop, key: u32, ch: u32, mods: u32, down: bool) {
    unsafe { &mut *d }.key(key, ch, mods, down);
}
#[no_mangle]
pub unsafe extern "C" fn desktop_tick(d: *mut Desktop, now_ms: u32, hour: i32, minute: i32) {
    unsafe { &mut *d }.tick(now_ms, hour, minute);
}
#[no_mangle]
pub unsafe extern "C" fn desktop_set_date(d: *mut Desktop, day: i32, month: i32, year: i32) {
    unsafe { crate::taskbar::taskbar_set_date(&mut (*d).taskbar, day, month, year) };
}
#[no_mangle]
pub unsafe extern "C" fn desktop_render(d: *mut Desktop) -> Rect {
    unsafe { &mut *d }.render()
}
#[no_mangle]
pub unsafe extern "C" fn desktop_render_all(d: *mut Desktop) {
    unsafe { &mut *d }.render_all();
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub unsafe extern "C" fn test_alloc(bytes: usize, _aux: *mut c_void) -> *mut c_void {
        let mut v = alloc::vec![0u32; bytes / 4];
        let p = v.as_mut_ptr();
        core::mem::forget(v);
        p as *mut c_void
    }

    pub fn desk(w: i32, h: i32) -> (Box<Desktop>, alloc::vec::Vec<u32>) {
        let mut back = alloc::vec![0u32; (w * h) as usize];
        let mut d = Desktop::boxed();
        assert!(d.init(back.as_mut_ptr(), w, h, 0, test_alloc, core::ptr::null_mut()));
        d.tick(1000, 9, 41);
        d.render_all();
        (d, back)
    }

    #[test]
    fn boots_renders_and_opens_apps() {
        let _g = crate::test_lock();
        let (mut d, back) = desk(1280, 720);
        assert!(back.iter().any(|&p| p != 0), "something was painted");
        d.open(A_CALC);
        d.open(A_TERM);
        d.tick(1100, 9, 42);
        assert!(d.render().w > 0, "opening apps damages the screen");
        assert!(d.aw[A_CALC].is_open() && d.aw[A_TERM].is_open());
    }

    #[test]
    fn config_lines_reach_the_backend_and_account_section_stops_parsing() {
        let _g = crate::test_lock();
        let (mut d, _b) = desk(1024, 600);
        d.apply_config(b"hostname=box\nui.base=nord\nbogus=1\n[accounts]\nui.base=dark\n");
        let mut v = [0u8; 16];
        assert!(d.be.cfg_get(b"hostname\0", &mut v));
        assert_eq!(nul(&v), b"box");
        assert!(d.be.cfg_get(b"ui.base\0", &mut v));
        assert_eq!(nul(&v), b"nord");
        assert!(!d.be.cfg_get(b"bogus\0", &mut v));
    }

    #[test]
    fn a_look_restyles_taskbar_and_window_manager_and_night_light_reaches_the_blit() {
        let _g = crate::test_lock();
        let (mut d, _b) = desk(1280, 720);
        let tiler = crate::look::look_index("tiler").unwrap();
        crate::look::store_look(&d.be, &crate::look::LOOKS[tiler]);
        crate::settings::apply(&mut d.settings);
        d.settings.changes += 1; // what the Settings card does
        d.tick(2000, 9, 42);
        assert!(d.taskbar.top && !d.taskbar.centered, "taskbar follows the look");
        assert!(d.wm.tiling && d.wm.gap == 8 && d.wm.focus_mode == 1, "window manager follows the look");

        cfg_set_int(&d.be, b"ui.night\0", 100);
        d.settings.changes += 1;
        d.tick(2100, 9, 42);
        assert_eq!(d.night, 100);
        d.render_all();
        let px = d.fb.px;
        // SAFETY: the back buffer holds w*h pixels.
        let any_warm = (0..1280 * 720).any(|i| unsafe { *px.add(i) } & 0xFF < (unsafe { *px.add(i) } >> 16 & 0xFF) / 2);
        assert!(any_warm, "blue is cut well below red somewhere");
    }

    #[test]
    fn palette_actions_and_kernel_look_option_work() {
        let _g = crate::test_lock();
        let (mut d, _b) = desk(1024, 600);
        assert!(d.apply_look(b"tiler") && !d.apply_look(b"nope"));
        assert!(d.wm.tiling && d.taskbar.top);
        d.run_action(b"act:tiling");
        assert!(!d.wm.tiling);
        d.run_action(b"act:night");
        assert_eq!(d.night, 60);
        d.apply_config(b"ui.night=0\n");
        assert_eq!(d.night, 0);
        let looks = (0..d.inv.n as usize).filter(|&i| nul(&d.inv.item[i].target).starts_with(b"look:")).count();
        assert_eq!(looks, crate::look::LOOKS.len());
    }

    #[test]
    fn rejects_unsupported_sizes() {
        let _g = crate::test_lock();
        let mut d = Desktop::boxed();
        let mut b = alloc::vec![0u32; 16];
        assert!(!d.init(b.as_mut_ptr(), 320, 200, 0, tests::test_alloc, core::ptr::null_mut()));
    }
}
