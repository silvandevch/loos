//! L6: `liblooswin` bridge. Linux programs reach the LoOS Win32 window system through
//! syscalls in the range `BRIDGE_BASE + LW_*`, so no Linux-side loader magic is needed:
//! `liblooswin.so` is a thin wrapper that issues `syscall` with these numbers.
//!
//! Calling convention: args in rdi,rsi,rdx,r10,r8,r9 like any Linux syscall; strings are
//! NUL-terminated user pointers; COLORREFs are 0x00BBGGRR; result in rax (negative errno).

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::errno::*;
use crate::mem::UserMem;
use crate::sys::{Bridge, Pid};
use loos_win32k::*;
use core::cell::RefCell;
use alloc::rc::Rc;

/// `LW_VERSION`: returns 0x0A00_0000 | bridge ABI level (1).
pub const LW_VERSION: u32 = 0;
/// `LW_TICK`: milliseconds since start.
pub const LW_TICK: u32 = 1;
/// `LW_CREATE_WINDOW(title, x, y, w, h, style)` → hwnd.
pub const LW_CREATE_WINDOW: u32 = 2;
/// `LW_SHOW(hwnd, cmd)`.
pub const LW_SHOW: u32 = 3;
/// `LW_NEXT_EVENT(hwnd, out*)`: writes 32 bytes `{hwnd u32, msg u32, wparam u64, lparam i64, x i32, y i32}`; returns 1, 0 (idle) or -1 (quit).
pub const LW_NEXT_EVENT: u32 = 4;
/// `LW_FILL_RECT(hwnd, l, t, r, b, colorref)`.
pub const LW_FILL_RECT: u32 = 5;
/// `LW_TEXT(hwnd, x, y, str, colorref)`.
pub const LW_TEXT: u32 = 6;
/// `LW_DESTROY(hwnd)`.
pub const LW_DESTROY: u32 = 7;
/// `LW_INVALIDATE(hwnd)`.
pub const LW_INVALIDATE: u32 = 8;
/// `LW_SET_TITLE(hwnd, str)`.
pub const LW_SET_TITLE: u32 = 9;
/// `LW_LINE(hwnd, x0, y0, x1, y1, colorref)`.
pub const LW_LINE: u32 = 10;
/// `LW_POST_QUIT(code)`.
pub const LW_POST_QUIT: u32 = 11;

fn lin_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_DESTROY => {
            k.post_quit_message(0);
            0
        }
        _ => k.def_window_proc(h, msg, w, l),
    }
}

/// Bridge into a shared window station.
pub struct Win32Bridge {
    k: Rc<RefCell<Win32k>>,
}

impl Win32Bridge {
    /// Wrap the window station; registers the `LooSLinux` class.
    pub fn new(k: Rc<RefCell<Win32k>>) -> Self {
        k.borrow_mut().register_class(WndClass { name: "LooSLinux".into(), style: CS_HREDRAW | CS_VREDRAW, proc_: Proc::Native(lin_proc), background: COLOR_WINDOW as u32 + 1, wnd_extra: 0 }).ok();
        Self { k }
    }
}

impl Bridge for Win32Bridge {
    fn call(&mut self, tid: Pid, mem: &mut UserMem, func: u32, a: [u64; 6]) -> i64 {
        let mut k = self.k.borrow_mut();
        k.set_current_thread(tid);
        let hwnd = a[0] as Hwnd;
        let res = |r: Result<(), Win32kError>| -> i64 { if r.is_ok() { 0 } else { -EINVAL } };
        let s = |mem: &mut UserMem, p: u64| mem.read_cstr(p, 1024);
        match func {
            LW_VERSION => 0x0A00_0001,
            LW_TICK => k.tick() as i64,
            LW_CREATE_WINDOW => {
                let Ok(title) = s(mem, a[0]) else { return -EFAULT };
                match k.create_window_ex(0, "LooSLinux", &title, a[5] as u32 | WS_OVERLAPPEDWINDOW & 0, a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, 0, 0, 0) {
                    Ok(h) => h as i64,
                    Err(e) => -(e.code() as i64),
                }
            }
            LW_SHOW => k.show_window(hwnd, a[1] as i32).map(|_| 0).unwrap_or(-EINVAL),
            LW_NEXT_EVENT => {
                let Some(m) = k.peek_message(hwnd, 0, 0, true) else { return 0 };
                if m.message == WM_QUIT {
                    return -1;
                }
                if m.message == WM_PAINT {
                    // Paint events are reported after the background is erased and the region validated.
                    if let Ok(ps) = k.begin_paint(m.hwnd) {
                        k.end_paint(m.hwnd, &ps);
                    }
                }
                let mut b = [0u8; 32];
                b[0..4].copy_from_slice(&m.hwnd.to_le_bytes());
                b[4..8].copy_from_slice(&m.message.to_le_bytes());
                b[8..16].copy_from_slice(&(m.wparam as u64).to_le_bytes());
                b[16..24].copy_from_slice(&(m.lparam as i64).to_le_bytes());
                b[24..28].copy_from_slice(&m.pt.x.to_le_bytes());
                b[28..32].copy_from_slice(&m.pt.y.to_le_bytes());
                if mem.write(a[1], &b).is_err() {
                    return -EFAULT;
                }
                // Route window-management messages (close button etc.) through the default procedure.
                if matches!(m.message, WM_SYSCOMMAND | WM_CLOSE) {
                    k.dispatch_message(&m);
                }
                1
            }
            LW_FILL_RECT => match k.get_dc(hwnd) {
                Ok(dc) => {
                    let br = k.create_solid_brush(a[5] as u32);
                    k.fill_rect(dc, Rect::new(a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32), br);
                    k.release_dc(hwnd, dc);
                    k.delete_object(br);
                    0
                }
                Err(_) => -EINVAL,
            },
            LW_TEXT => {
                let Ok(txt) = s(mem, a[3]) else { return -EFAULT };
                match k.get_dc(hwnd) {
                    Ok(dc) => {
                        k.set_text_color(dc, a[4] as u32);
                        k.set_bk_mode(dc, 1);
                        k.text_out(dc, a[1] as i32, a[2] as i32, &txt);
                        k.release_dc(hwnd, dc);
                        0
                    }
                    Err(_) => -EINVAL,
                }
            }
            LW_LINE => match k.get_dc(hwnd) {
                Ok(dc) => {
                    let pen = k.create_pen(1, a[5] as u32);
                    let old = k.select_object(dc, pen);
                    k.move_to(dc, a[1] as i32, a[2] as i32);
                    k.line_to(dc, a[3] as i32, a[4] as i32);
                    k.select_object(dc, old);
                    k.release_dc(hwnd, dc);
                    k.delete_object(pen);
                    0
                }
                Err(_) => -EINVAL,
            },
            LW_DESTROY => res(k.destroy_window(hwnd)),
            LW_INVALIDATE => res(k.invalidate_rect(hwnd, None, true)),
            LW_SET_TITLE => match s(mem, a[1]) {
                Ok(t) => res(k.set_window_text(hwnd, &t)),
                Err(e) => -e,
            },
            LW_POST_QUIT => {
                k.post_quit_message(a[0] as i32);
                0
            }
            _ => -ENOSYS,
        }
    }
}
