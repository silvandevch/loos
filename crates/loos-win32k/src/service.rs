//! win32k service dispatcher: the syscall-style entry that `user32.dll`/`gdi32.dll` PE stubs
//! reach (`mov eax, SVC_x; syscall`). Arguments are raw registers; structures are decoded from
//! guest memory with their Win64 layouts. Window procedures registered by guests become
//! [`Proc::User`] and run through the kernel's upcall hook.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::*;

/// Guest (user-mode) memory accessor supplied by the kernel.
pub trait GuestMem {
    /// Copy out; false if unmapped.
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> bool;
    /// Copy in; false if unmapped.
    fn write(&mut self, addr: u64, data: &[u8]) -> bool;
}

/// First service number.
pub const SVC_BASE: u32 = 0x1000;
/// Returned by blocking services when the thread must sleep until its queue has work.
pub const WOULD_BLOCK: i64 = i64::MIN;

macro_rules! svcs {
    ($($name:ident = $n:expr),* $(,)?) => { $(
        /// Service number.
        pub const $name: u32 = SVC_BASE + $n;
    )* };
}
svcs!(
    SVC_REGISTER_CLASS = 0, SVC_CREATE_WINDOW = 1, SVC_SHOW_WINDOW = 2, SVC_DESTROY_WINDOW = 3, SVC_GET_MESSAGE = 4,
    SVC_PEEK_MESSAGE = 5, SVC_TRANSLATE_MESSAGE = 6, SVC_DISPATCH_MESSAGE = 7, SVC_POST_QUIT = 8, SVC_DEF_WINDOW_PROC = 9,
    SVC_POST_MESSAGE = 10, SVC_SEND_MESSAGE = 11, SVC_BEGIN_PAINT = 12, SVC_END_PAINT = 13, SVC_GET_DC = 14,
    SVC_RELEASE_DC = 15, SVC_FILL_RECT = 16, SVC_TEXT_OUT = 17, SVC_SET_TEXT_COLOR = 18, SVC_SET_BK_COLOR = 19,
    SVC_SET_BK_MODE = 20, SVC_CREATE_BRUSH = 21, SVC_CREATE_PEN = 22, SVC_SELECT_OBJECT = 23, SVC_DELETE_OBJECT = 24,
    SVC_INVALIDATE_RECT = 25, SVC_UPDATE_WINDOW = 26, SVC_GET_CLIENT_RECT = 27, SVC_GET_WINDOW_RECT = 28, SVC_SET_TIMER = 29,
    SVC_KILL_TIMER = 30, SVC_MESSAGE_BOX = 31, SVC_SET_WINDOW_TEXT = 32, SVC_MOVE_WINDOW = 33, SVC_LINE_TO = 34,
    SVC_MOVE_TO = 35, SVC_RECTANGLE = 36, SVC_GET_STOCK_OBJECT = 37, SVC_SET_FOCUS = 38, SVC_GET_TICK = 39,
    SVC_MESSAGE_BOX_RESULT = 40, SVC_GET_LAST_ERROR = 41,
);

fn rd<const N: usize>(m: &mut dyn GuestMem, a: u64) -> Option<[u8; N]> {
    let mut b = [0u8; N];
    m.read(a, &mut b).then_some(b)
}
fn u32_at(m: &mut dyn GuestMem, a: u64) -> Option<u32> {
    rd::<4>(m, a).map(u32::from_le_bytes)
}
fn u64_at(m: &mut dyn GuestMem, a: u64) -> Option<u64> {
    rd::<8>(m, a).map(u64::from_le_bytes)
}

/// Read a NUL-terminated UTF-16 string (max 4096 units). Values < 0x10000 are atoms/ints → `None`.
fn wstr(m: &mut dyn GuestMem, a: u64) -> Option<String> {
    if a < 0x1_0000 {
        return None;
    }
    let mut v = Vec::new();
    for i in 0..4096u64 {
        let c = u16::from_le_bytes(rd::<2>(m, a + i * 2)?);
        if c == 0 {
            return Some(String::from_utf16_lossy(&v));
        }
        v.push(c);
    }
    None
}

fn put_rect(m: &mut dyn GuestMem, a: u64, r: Rect) -> bool {
    let mut b = [0u8; 16];
    for (i, v) in [r.left, r.top, r.right, r.bottom].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    m.write(a, &b)
}
fn get_rect(m: &mut dyn GuestMem, a: u64) -> Option<Rect> {
    let b = rd::<16>(m, a)?;
    let i = |k: usize| i32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap());
    Some(Rect::new(i(0), i(1), i(2), i(3)))
}
fn put_msg(m: &mut dyn GuestMem, a: u64, g: &Msg) -> bool {
    let mut b = [0u8; 48];
    b[0..8].copy_from_slice(&(g.hwnd as u64).to_le_bytes());
    b[8..12].copy_from_slice(&g.message.to_le_bytes());
    b[16..24].copy_from_slice(&(g.wparam as u64).to_le_bytes());
    b[24..32].copy_from_slice(&(g.lparam as i64).to_le_bytes());
    b[32..36].copy_from_slice(&g.time.to_le_bytes());
    b[36..40].copy_from_slice(&g.pt.x.to_le_bytes());
    b[40..44].copy_from_slice(&g.pt.y.to_le_bytes());
    m.write(a, &b)
}
fn get_msg(m: &mut dyn GuestMem, a: u64) -> Option<Msg> {
    let b = rd::<48>(m, a)?;
    Some(Msg {
        hwnd: u64::from_le_bytes(b[0..8].try_into().unwrap()) as u32,
        message: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        wparam: u64::from_le_bytes(b[16..24].try_into().unwrap()) as usize,
        lparam: i64::from_le_bytes(b[24..32].try_into().unwrap()) as isize,
        time: u32::from_le_bytes(b[32..36].try_into().unwrap()),
        pt: Point { x: i32::from_le_bytes(b[36..40].try_into().unwrap()), y: i32::from_le_bytes(b[40..44].try_into().unwrap()) },
    })
}

impl Win32k {
    /// Execute service `nr` for thread `tid` with raw argument registers. Returns RAX.
    /// Blocking services return [`WOULD_BLOCK`]; the kernel parks the thread and retries when
    /// its queue becomes non-empty (`GetMessage` with an empty queue).
    pub fn service(&mut self, tid: u32, nr: u32, a: [u64; 6], m: &mut dyn GuestMem) -> i64 {
        self.set_current_thread(tid);
        let h = a[0] as Hwnd;
        let ok = |r: Result<(), Win32kError>| r.is_ok() as i64;
        match nr {
            SVC_REGISTER_CLASS => {
                let Some(b) = rd::<80>(m, a[0]) else { return 0 };
                let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
                let Some(name) = wstr(m, q(64)) else { return 0 };
                let def = WndClass {
                    name,
                    style: u32::from_le_bytes(b[4..8].try_into().unwrap()),
                    proc_: Proc::User(q(8)),
                    background: q(48) as u32,
                    wnd_extra: (i32::from_le_bytes(b[20..24].try_into().unwrap()).clamp(0, 4096) as usize).div_ceil(8),
                };
                self.register_class(def).map_or(0, |at| at as i64)
            }
            SVC_CREATE_WINDOW => {
                // a0 -> { exstyle, class*, title*, style, x, y, w, h, parent, id, param } (u64 each)
                let mut f = [0u64; 11];
                for (i, s) in f.iter_mut().enumerate() {
                    let Some(v) = u64_at(m, a[0] + i as u64 * 8) else { return 0 };
                    *s = v;
                }
                let (Some(class), title) = (wstr(m, f[1]), wstr(m, f[2]).unwrap_or_default()) else { return 0 };
                self.create_window_ex(f[0] as u32, &class, &title, f[3] as u32, f[4] as i32, f[5] as i32, f[6] as i32, f[7] as i32, f[8] as u32, f[9] as isize, f[10] as isize).map_or(0, |h| h as i64)
            }
            SVC_SHOW_WINDOW => self.show_window(h, a[1] as i32).map_or(0, |v| v as i64),
            SVC_DESTROY_WINDOW => ok(self.destroy_window(h)),
            SVC_GET_MESSAGE | SVC_PEEK_MESSAGE => {
                let remove = nr == SVC_GET_MESSAGE || a[4] & 1 != 0;
                match self.peek_message(a[1] as Hwnd, a[2] as u32, a[3] as u32, remove) {
                    Some(g) => {
                        if !put_msg(m, a[0], &g) {
                            return -1;
                        }
                        if nr == SVC_GET_MESSAGE && g.message == WM_QUIT { 0 } else { 1 }
                    }
                    None if nr == SVC_GET_MESSAGE => WOULD_BLOCK,
                    None => 0,
                }
            }
            SVC_TRANSLATE_MESSAGE => get_msg(m, a[0]).map_or(0, |g| self.translate_message(&g) as i64),
            SVC_DISPATCH_MESSAGE => get_msg(m, a[0]).map_or(0, |g| self.dispatch_message(&g) as i64),
            SVC_POST_QUIT => {
                self.post_quit_message(a[0] as i32);
                0
            }
            SVC_DEF_WINDOW_PROC => self.def_window_proc(h, a[1] as u32, a[2] as usize, a[3] as isize) as i64,
            SVC_POST_MESSAGE => ok(self.post_message(h, a[1] as u32, a[2] as usize, a[3] as isize)),
            SVC_SEND_MESSAGE => self.send_message(h, a[1] as u32, a[2] as usize, a[3] as isize) as i64,
            SVC_BEGIN_PAINT => match self.begin_paint(h) {
                Ok(ps) => {
                    let mut b = [0u8; 72];
                    b[0..8].copy_from_slice(&(ps.hdc as u64).to_le_bytes());
                    b[8..12].copy_from_slice(&(ps.erase as i32).to_le_bytes());
                    for (i, v) in [ps.rc.left, ps.rc.top, ps.rc.right, ps.rc.bottom].iter().enumerate() {
                        b[12 + i * 4..16 + i * 4].copy_from_slice(&v.to_le_bytes());
                    }
                    if m.write(a[1], &b) { ps.hdc as i64 } else { 0 }
                }
                Err(_) => 0,
            },
            SVC_END_PAINT => {
                let Some(hdc) = u64_at(m, a[1]) else { return 0 };
                self.release_dc(h, hdc as Hdc);
                1
            }
            SVC_GET_DC => self.get_dc(h).map_or(0, |d| d as i64),
            SVC_RELEASE_DC => self.release_dc(h, a[1] as Hdc) as i64,
            SVC_FILL_RECT => get_rect(m, a[1]).map_or(0, |r| self.fill_rect(a[0] as Hdc, r, a[2] as Hgdi) as i64),
            SVC_TEXT_OUT => {
                // (hdc, x, y, wstr*, count)
                let n = a[4].min(4096);
                let mut v = Vec::new();
                for i in 0..n {
                    let Some(b) = rd::<2>(m, a[3] + i * 2) else { return 0 };
                    v.push(u16::from_le_bytes(b));
                }
                self.text_out(a[0] as Hdc, a[1] as i32, a[2] as i32, &String::from_utf16_lossy(&v)) as i64
            }
            SVC_SET_TEXT_COLOR => self.set_text_color(a[0] as Hdc, a[1] as u32) as i64,
            SVC_SET_BK_COLOR => self.set_bk_color(a[0] as Hdc, a[1] as u32) as i64,
            SVC_SET_BK_MODE => self.set_bk_mode(a[0] as Hdc, a[1] as i32) as i64,
            SVC_CREATE_BRUSH => self.create_solid_brush(a[0] as u32) as i64,
            SVC_CREATE_PEN => self.create_pen(a[1] as i32, a[2] as u32) as i64,
            SVC_SELECT_OBJECT => self.select_object(a[0] as Hdc, a[1] as Hgdi) as i64,
            SVC_DELETE_OBJECT => self.delete_object(a[0] as Hgdi) as i64,
            SVC_INVALIDATE_RECT => {
                let r = if a[1] == 0 { None } else { get_rect(m, a[1]) };
                ok(self.invalidate_rect(h, r, a[2] != 0))
            }
            SVC_UPDATE_WINDOW => ok(self.update_window(h)),
            SVC_GET_CLIENT_RECT | SVC_GET_WINDOW_RECT => {
                let r = if nr == SVC_GET_CLIENT_RECT { self.get_client_rect(h) } else { self.get_window_rect(h) };
                r.map_or(0, |r| put_rect(m, a[1], r) as i64)
            }
            SVC_SET_TIMER => self.set_timer(h, a[1] as usize, a[2] as u32) as i64,
            SVC_KILL_TIMER => self.kill_timer(h, a[1] as usize) as i64,
            SVC_MESSAGE_BOX => {
                let (Some(t), Some(c)) = (wstr(m, a[1]), wstr(m, a[2])) else { return 0 };
                self.message_box(h, &t, &c, a[3] as u32).map_or(0, |d| d as i64)
            }
            SVC_MESSAGE_BOX_RESULT => self.message_box_result(a[0] as Hwnd, a[1] as Hwnd).map_or(WOULD_BLOCK, |r| r as i64),
            SVC_SET_WINDOW_TEXT => wstr(m, a[1]).map_or(0, |t| ok(self.set_window_text(h, &t))),
            SVC_MOVE_WINDOW => {
                let (x, y, w, hh) = (a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
                ok(self.move_window(h, x, y, w, hh))
            }
            SVC_LINE_TO => self.line_to(a[0] as Hdc, a[1] as i32, a[2] as i32) as i64,
            SVC_MOVE_TO => {
                self.move_to(a[0] as Hdc, a[1] as i32, a[2] as i32);
                1
            }
            SVC_RECTANGLE => self.rectangle(a[0] as Hdc, a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32) as i64,
            SVC_GET_STOCK_OBJECT => self.get_stock_object(a[0] as i32) as i64,
            SVC_SET_FOCUS => self.set_focus(h) as i64,
            SVC_GET_TICK => self.tick() as i64,
            SVC_GET_LAST_ERROR => self.last_error() as i64,
            _ => -38, // ENOSYS-like: unknown win32k service
        }
    }
}
