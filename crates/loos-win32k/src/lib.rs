//! Win32 window manager and GDI (the `win32k` half of user32/gdi32).
//!
//! Single-address-space, cooperative model: every window belongs to a thread id
//! (`set_current_thread`), each thread owns a message queue, and window procedures
//! are either native Rust functions or user-mode addresses reached through the
//! upcall hook installed by the kernel (`set_upcall`). Hardware input is injected
//! with `inject_*`; `compose` renders the desktop to a 0xAARRGGBB framebuffer.
//!
//! Spec: Win32 documented behaviour (CreateWindowEx, GetMessage, DefWindowProc, ...).
#![no_std]
extern crate alloc;
#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

mod controls;
mod gdi;
mod input;
mod menu;
mod service;
mod wm;
pub use wm::adjust_window_rect;

pub use controls::*;
pub use gdi::*;
pub use menu::*;
pub use service::*;
pub use wm::*;

use alloc::collections::BTreeMap;

/// Window handle (0 = NULL).
pub type Hwnd = u32;
/// Device-context handle.
pub type Hdc = u32;
/// GDI object handle (brush/pen/font/bitmap).
pub type Hgdi = u32;

/// Win32 `POINT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
}

/// Win32 `RECT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// Left.
    pub left: i32,
    /// Top.
    pub top: i32,
    /// Right (exclusive).
    pub right: i32,
    /// Bottom (exclusive).
    pub bottom: i32,
}

impl Rect {
    /// New rect.
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }
    /// Width.
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    /// Height.
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    /// True when no pixels are covered.
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
    /// Intersection (may be empty).
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(self.left.max(o.left), self.top.max(o.top), self.right.min(o.right), self.bottom.min(o.bottom))
    }
    /// Bounding union.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::new(self.left.min(o.left), self.top.min(o.top), self.right.max(o.right), self.bottom.max(o.bottom))
    }
    /// Point containment.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
    /// Translate.
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }
}

/// Win32 `MSG`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Msg {
    /// Target window (0 = thread message).
    pub hwnd: Hwnd,
    /// Message id.
    pub message: u32,
    /// WPARAM.
    pub wparam: usize,
    /// LPARAM.
    pub lparam: isize,
    /// Tick (ms) when queued.
    pub time: u32,
    /// Cursor position when queued (screen).
    pub pt: Point,
}

/// Errors (map to `GetLastError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Win32kError {
    /// ERROR_INVALID_WINDOW_HANDLE (1400).
    InvalidWindow,
    /// ERROR_CANNOT_FIND_WND_CLASS (1407).
    NoClass,
    /// ERROR_CLASS_ALREADY_EXISTS (1410).
    ClassExists,
    /// ERROR_INVALID_HANDLE (6).
    InvalidHandle,
    /// ERROR_NOT_ENOUGH_MEMORY (8) — window/bitmap too large.
    TooLarge,
    /// WM_NCCREATE/WM_CREATE rejected creation.
    CreateFailed,
}

impl Win32kError {
    /// Win32 error code.
    pub fn code(self) -> u32 {
        match self {
            Self::InvalidWindow => 1400,
            Self::NoClass => 1407,
            Self::ClassExists => 1410,
            Self::InvalidHandle => 6,
            Self::TooLarge => 8,
            Self::CreateFailed => 1,
        }
    }
}

/// Native window procedure.
pub type NativeProc = fn(&mut Win32k, Hwnd, u32, usize, isize) -> isize;

/// A window procedure: native Rust, or a user-mode address reached via the upcall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proc {
    /// Native function.
    Native(NativeProc),
    /// User-mode `WNDPROC` address.
    User(u64),
}

/// Kernel hook that enters user mode: `(win32k, wndproc_addr, hwnd, msg, wparam, lparam) -> LRESULT`.
pub type Upcall = Box<dyn FnMut(&mut Win32k, u64, Hwnd, u32, usize, isize) -> isize>;

/// `WNDCLASSEX` (the parts the window manager uses).
#[derive(Clone)]
pub struct WndClass {
    /// Class name (case-insensitive).
    pub name: String,
    /// CS_* style.
    pub style: u32,
    /// Window procedure.
    pub proc_: Proc,
    /// Background: a brush handle, or `COLOR_xxx + 1` (small integer), or 0.
    pub background: u32,
    /// Extra bytes (in `isize` slots) per window.
    pub wnd_extra: usize,
}

/// `CREATESTRUCT` (read it with `Win32k::create_struct` while handling WM_NCCREATE/WM_CREATE).
#[derive(Clone, Debug, Default)]
pub struct CreateStruct {
    /// lpCreateParams.
    pub create_param: isize,
    /// Parent.
    pub parent: Hwnd,
    /// Menu / child id.
    pub id: isize,
    /// Height.
    pub cy: i32,
    /// Width.
    pub cx: i32,
    /// Y.
    pub y: i32,
    /// X.
    pub x: i32,
    /// Style.
    pub style: u32,
    /// Ex style.
    pub exstyle: u32,
    /// Title.
    pub name: String,
}

pub(crate) struct Class {
    pub def: WndClass,
    pub atom: u16,
}

pub(crate) struct Window {
    pub class: usize,
    pub parent: Hwnd,
    pub owner: Hwnd,
    pub children: Vec<Hwnd>,
    pub title: String,
    pub style: u32,
    pub exstyle: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub minimized: bool,
    pub maximized: bool,
    pub restore: Option<(i32, i32, i32, i32)>,
    pub tid: u32,
    pub proc_: Proc,
    pub userdata: isize,
    pub id: isize,
    pub extra: Vec<isize>,
    pub invalid: Option<Rect>,
    pub erase: bool,
    pub bmp: Vec<u32>,
    pub cw: i32,
    pub ch: i32,
    pub create: CreateStruct,
    pub menu: Vec<MenuItem>,
}

#[derive(Default)]
pub(crate) struct Queue {
    pub posted: alloc::collections::VecDeque<Msg>,
    pub quit: Option<i32>,
}

pub(crate) struct Timer {
    pub tid: u32,
    pub hwnd: Hwnd,
    pub id: usize,
    pub interval: u64,
    pub next: u64,
}

pub(crate) enum Gdi {
    Brush { color: u32, null: bool },
    Pen { color: u32, width: i32, null: bool },
    Font { height: i32 },
    Bitmap { w: i32, h: i32, px: Vec<u32> },
}

pub(crate) enum DcKind {
    Window(Hwnd),
    Mem,
}

pub(crate) struct Dc {
    pub kind: DcKind,
    pub brush: Hgdi,
    pub pen: Hgdi,
    pub font: Hgdi,
    pub bitmap: Hgdi,
    pub text_color: u32,
    pub bk_color: u32,
    pub opaque: bool,
    pub cur: Point,
    pub clip: Option<Rect>,
}

/// The window station + desktop.
pub struct Win32k {
    pub(crate) classes: Vec<Class>,
    pub(crate) wins: BTreeMap<Hwnd, Window>,
    pub(crate) tops: Vec<Hwnd>,
    pub(crate) queues: BTreeMap<u32, Queue>,
    pub(crate) cur_tid: u32,
    pub(crate) next_hwnd: u32,
    pub(crate) focus: Hwnd,
    pub(crate) active: Hwnd,
    pub(crate) capture: Hwnd,
    pub(crate) cursor: Point,
    pub(crate) buttons: u32,
    pub(crate) keys: [bool; 256],
    pub(crate) drag: Option<(Hwnd, i32, i32)>,
    pub(crate) close_down: Hwnd,
    pub(crate) timers: Vec<Timer>,
    pub(crate) now_ms: u64,
    pub(crate) gdi: BTreeMap<u32, Gdi>,
    pub(crate) dcs: BTreeMap<u32, Dc>,
    pub(crate) next_gdi: u32,
    pub(crate) upcall: Option<Upcall>,
    pub(crate) cascade: i32,
    pub(crate) last_error: u32,
    pub(crate) results: BTreeMap<Hwnd, i32>,
    /// User-mode `DLGPROC` per dialog window (dialogs created from templates).
    pub(crate) dlg_procs: BTreeMap<Hwnd, u64>,
    pub(crate) open_menu: Option<(Hwnd, usize)>,
    /// Screen size in pixels.
    pub screen: (i32, i32),
    /// Desktop background (0xAARRGGBB).
    pub desktop_color: u32,
}

impl Win32k {
    /// New desktop of `w`×`h` pixels with the stock GDI objects.
    pub fn new(w: i32, h: i32) -> Self {
        let mut s = Self {
            classes: Vec::new(),
            wins: BTreeMap::new(),
            tops: Vec::new(),
            queues: BTreeMap::new(),
            cur_tid: 1,
            next_hwnd: 0x0001_0000,
            focus: 0,
            active: 0,
            capture: 0,
            cursor: Point { x: w / 2, y: h / 2 },
            buttons: 0,
            keys: [false; 256],
            drag: None,
            close_down: 0,
            timers: Vec::new(),
            now_ms: 0,
            gdi: BTreeMap::new(),
            dcs: BTreeMap::new(),
            next_gdi: 0x10,
            upcall: None,
            cascade: 0,
            last_error: 0,
            results: BTreeMap::new(),
            dlg_procs: BTreeMap::new(),
            open_menu: None,
            screen: (w, h),
            desktop_color: 0xFF3A6EA5,
        };
        s.init_stock();
        s.init_controls();
        s
    }

    /// Thread the following calls run on (selects the message queue and new-window owner).
    pub fn set_current_thread(&mut self, tid: u32) {
        self.cur_tid = tid;
    }

    /// Install the user-mode upcall used for `Proc::User` window procedures.
    pub fn set_upcall(&mut self, f: Upcall) {
        self.upcall = Some(f);
    }

    /// `GetLastError` for the last failing call.
    pub fn last_error(&self) -> u32 {
        self.last_error
    }

    pub(crate) fn fail<T>(&mut self, e: Win32kError) -> Result<T, Win32kError> {
        self.last_error = e.code();
        Err(e)
    }

    /// Milliseconds since start (`GetTickCount`).
    pub fn tick(&self) -> u32 {
        self.now_ms as u32
    }
}

// Window messages
/// WM_NULL.
pub const WM_NULL: u32 = 0x0000;
/// WM_CREATE.
pub const WM_CREATE: u32 = 0x0001;
/// WM_DESTROY.
pub const WM_DESTROY: u32 = 0x0002;
/// WM_MOVE.
pub const WM_MOVE: u32 = 0x0003;
/// WM_SIZE.
pub const WM_SIZE: u32 = 0x0005;
/// WM_ACTIVATE.
pub const WM_ACTIVATE: u32 = 0x0006;
/// WM_SETFOCUS.
pub const WM_SETFOCUS: u32 = 0x0007;
/// WM_KILLFOCUS.
pub const WM_KILLFOCUS: u32 = 0x0008;
/// WM_ENABLE.
pub const WM_ENABLE: u32 = 0x000A;
/// WM_SETTEXT.
pub const WM_SETTEXT: u32 = 0x000C;
/// WM_GETTEXTLENGTH.
pub const WM_GETTEXTLENGTH: u32 = 0x000E;
/// WM_PAINT.
pub const WM_PAINT: u32 = 0x000F;
/// WM_CLOSE.
pub const WM_CLOSE: u32 = 0x0010;
/// WM_QUERYENDSESSION.
pub const WM_QUERYENDSESSION: u32 = 0x0011;
/// WM_QUIT.
pub const WM_QUIT: u32 = 0x0012;
/// WM_ERASEBKGND.
pub const WM_ERASEBKGND: u32 = 0x0014;
/// WM_SHOWWINDOW.
pub const WM_SHOWWINDOW: u32 = 0x0018;
/// WM_COMMAND.
pub const WM_COMMAND: u32 = 0x0111;
/// WM_NCCREATE.
pub const WM_NCCREATE: u32 = 0x0081;
/// WM_NCDESTROY.
pub const WM_NCDESTROY: u32 = 0x0082;
/// WM_NCHITTEST.
pub const WM_NCHITTEST: u32 = 0x0084;
/// WM_KEYDOWN.
pub const WM_KEYDOWN: u32 = 0x0100;
/// WM_KEYUP.
pub const WM_KEYUP: u32 = 0x0101;
/// WM_CHAR.
pub const WM_CHAR: u32 = 0x0102;
/// WM_SYSKEYDOWN.
pub const WM_SYSKEYDOWN: u32 = 0x0104;
/// WM_SYSKEYUP.
pub const WM_SYSKEYUP: u32 = 0x0105;
/// WM_SYSCOMMAND.
pub const WM_SYSCOMMAND: u32 = 0x0112;
/// WM_TIMER.
pub const WM_TIMER: u32 = 0x0113;
/// WM_MOUSEMOVE.
pub const WM_MOUSEMOVE: u32 = 0x0200;
/// WM_LBUTTONDOWN.
pub const WM_LBUTTONDOWN: u32 = 0x0201;
/// WM_LBUTTONUP.
pub const WM_LBUTTONUP: u32 = 0x0202;
/// WM_RBUTTONDOWN.
pub const WM_RBUTTONDOWN: u32 = 0x0204;
/// WM_RBUTTONUP.
pub const WM_RBUTTONUP: u32 = 0x0205;
/// WM_MBUTTONDOWN.
pub const WM_MBUTTONDOWN: u32 = 0x0207;
/// WM_MBUTTONUP.
pub const WM_MBUTTONUP: u32 = 0x0208;
/// WM_USER.
pub const WM_USER: u32 = 0x0400;

// Window styles
/// WS_POPUP.
pub const WS_POPUP: u32 = 0x8000_0000;
/// WS_CHILD.
pub const WS_CHILD: u32 = 0x4000_0000;
/// WS_VISIBLE.
pub const WS_VISIBLE: u32 = 0x1000_0000;
/// WS_DISABLED.
pub const WS_DISABLED: u32 = 0x0800_0000;
/// WS_BORDER.
pub const WS_BORDER: u32 = 0x0080_0000;
/// WS_CAPTION.
pub const WS_CAPTION: u32 = 0x00C0_0000;
/// WS_SYSMENU.
pub const WS_SYSMENU: u32 = 0x0008_0000;
/// WS_OVERLAPPEDWINDOW (simplified: caption + sysmenu + frame).
pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
/// CS_HREDRAW.
pub const CS_HREDRAW: u32 = 0x0002;
/// CS_VREDRAW.
pub const CS_VREDRAW: u32 = 0x0001;
/// CW_USEDEFAULT.
pub const CW_USEDEFAULT: i32 = i32::MIN;

// ShowWindow
/// SW_HIDE.
pub const SW_HIDE: i32 = 0;
/// SW_SHOWNORMAL.
pub const SW_SHOWNORMAL: i32 = 1;
/// SW_SHOWMINIMIZED.
pub const SW_SHOWMINIMIZED: i32 = 2;
/// SW_SHOWMAXIMIZED.
pub const SW_SHOWMAXIMIZED: i32 = 3;
/// SW_SHOW.
pub const SW_SHOW: i32 = 5;
/// SW_MINIMIZE.
pub const SW_MINIMIZE: i32 = 6;
/// SW_RESTORE.
pub const SW_RESTORE: i32 = 9;

// Hit-test codes
/// HTCLIENT.
pub const HTCLIENT: isize = 1;
/// HTCAPTION.
pub const HTCAPTION: isize = 2;
/// HTBORDER.
pub const HTBORDER: isize = 18;
/// HTMENU.
pub const HTMENU: isize = 5;
/// HTCLOSE.
pub const HTCLOSE: isize = 20;
/// HTTRANSPARENT.
pub const HTTRANSPARENT: isize = -1;
/// SC_CLOSE.
pub const SC_CLOSE: usize = 0xF060;
/// SC_MINIMIZE.
pub const SC_MINIMIZE: usize = 0xF020;
/// SC_MAXIMIZE.
pub const SC_MAXIMIZE: usize = 0xF030;
/// SC_RESTORE.
pub const SC_RESTORE: usize = 0xF120;

// Virtual keys
/// VK_BACK.
pub const VK_BACK: u32 = 0x08;
/// VK_TAB.
pub const VK_TAB: u32 = 0x09;
/// VK_RETURN.
pub const VK_RETURN: u32 = 0x0D;
/// VK_SHIFT.
pub const VK_SHIFT: u32 = 0x10;
/// VK_CONTROL.
pub const VK_CONTROL: u32 = 0x11;
/// VK_MENU (Alt).
pub const VK_MENU: u32 = 0x12;
/// VK_ESCAPE.
pub const VK_ESCAPE: u32 = 0x1B;
/// VK_SPACE.
pub const VK_SPACE: u32 = 0x20;

// MK_* mouse flags
/// MK_LBUTTON.
pub const MK_LBUTTON: u32 = 1;
/// MK_RBUTTON.
pub const MK_RBUTTON: u32 = 2;
/// MK_MBUTTON.
pub const MK_MBUTTON: u32 = 0x10;

/// `MAKELPARAM(lo, hi)`.
pub fn make_lparam(lo: i32, hi: i32) -> isize {
    ((lo as u16 as u32) | ((hi as u16 as u32) << 16)) as i32 as isize
}
/// `GET_X_LPARAM`.
pub fn lparam_x(l: isize) -> i32 {
    l as u16 as i16 as i32
}
/// `GET_Y_LPARAM`.
pub fn lparam_y(l: isize) -> i32 {
    (l >> 16) as u16 as i16 as i32
}
/// `RGB(r,g,b)` → COLORREF.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
