//! user32/gdi32 basics reached through the Win64 entry points (`gui_syscall`) over a real `Win32k`.
use loos_win32k::{Proc, Win32k, WndClass, WS_CHILD, WS_VISIBLE};
use loos_winrt::gui::lookup;
use loos_winrt::{Process, Ret};

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");

fn main_proc(w: &mut Win32k, h: u32, m: u32, wp: usize, lp: isize) -> isize {
    if m == 0x81 { 1 } else { w.def_window_proc(h, m, wp, lp) }
}

struct T {
    p: Process,
    w: Win32k,
    buf: u64,
    at: u64,
}

impl T {
    fn new() -> T {
        let mut p = Process::create(1, 1, "t.exe", "", SVC).unwrap();
        let mut w = Win32k::new(800, 600);
        w.set_current_thread(p.threads[0].tid as u32);
        w.register_class(WndClass { name: "Main".into(), style: 0, proc_: Proc::Native(main_proc), background: 6, wnd_extra: 2 }).unwrap();
        let buf = p.mem.map_anywhere(0x8000, 3, &[], "buf").unwrap();
        T { p, w, buf, at: 0x100 }
    }
    fn mem(&mut self, n: u64) -> u64 {
        let a = self.buf + self.at;
        self.at += n.max(8);
        a
    }
    fn ws(&mut self, s: &str) -> u64 {
        let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend_from_slice(&[0, 0]);
        let a = self.mem(b.len() as u64 + 8);
        self.p.mem.write(a, &b).unwrap();
        a
    }
    fn call(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> u64 {
        let rsp = self.buf + 0x7000;
        for (k, v) in stack.iter().enumerate() {
            self.p.mem.write_u64(rsp + 0x28 + 8 * k as u64, *v).unwrap();
        }
        let nr = lookup(name).unwrap_or_else(|| panic!("{name}"));
        match self.p.gui_syscall(nr, a, rsp, 10, &mut self.w) {
            Ret::Value(v) => v,
            r => panic!("{name}: {r:?}"),
        }
    }
    fn win(&mut self, title: &str, style: u32, parent: u32, id: isize, x: i32, y: i32) -> u32 {
        self.w.create_window_ex(0, "Main", title, style, x, y, 200, 100, parent, id, 0).unwrap()
    }
}

#[test]
fn window_tree_queries_and_long_ptr() {
    let mut t = T::new();
    let top = t.win("Top", WS_VISIBLE, 0, 0, 50, 40);
    let kid = t.win("Kid", WS_CHILD | WS_VISIBLE, top, 7, 10, 20);
    let kid2 = t.win("Kid2", WS_CHILD, top, 8, 0, 0);
    assert_eq!(t.call("IsWindow", [kid as u64, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("IsWindow", [0xdead, 0, 0, 0], &[]), 0);
    assert_eq!(t.call("GetParent", [kid as u64, 0, 0, 0], &[]), top as u64);
    assert_eq!(t.call("GetDlgItem", [top as u64, 8, 0, 0], &[]), kid2 as u64);
    assert_eq!(t.call("GetDlgItem", [top as u64, 99, 0, 0], &[]), 0);
    assert_eq!(t.call("GetWindow", [top as u64, 5, 0, 0], &[]), kid as u64, "GW_CHILD");
    assert_eq!(t.call("GetWindow", [kid as u64, 2, 0, 0], &[]), kid2 as u64, "GW_HWNDNEXT");
    assert_eq!(t.call("IsWindowVisible", [kid as u64, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("IsWindowVisible", [kid2 as u64, 0, 0, 0], &[]), 0);
    assert_eq!(t.call("EnableWindow", [kid as u64, 0, 0, 0], &[]), 0, "was enabled");
    assert_eq!(t.call("IsWindowEnabled", [kid as u64, 0, 0, 0], &[]), 0);
    // GWLP_USERDATA / GWLP_ID / per-window extra bytes / WNDPROC.
    assert_eq!(t.call("SetWindowLongPtrW", [kid as u64, (-21i64) as u64, 0x1234, 0], &[]), 0);
    assert_eq!(t.call("GetWindowLongPtrW", [kid as u64, (-21i64) as u64, 0, 0], &[]), 0x1234);
    assert_eq!(t.call("GetWindowLongPtrW", [kid as u64, (-12i64) as u64, 0, 0], &[]), 7);
    assert_eq!(t.call("SetWindowLongPtrW", [kid as u64, 8, 55, 0], &[]), 0);
    assert_eq!(t.call("GetWindowLongPtrW", [kid as u64, 8, 0, 0], &[]), 55);
    assert_eq!(t.call("SetWindowLongPtrW", [kid as u64, (-4i64) as u64, 0x4000, 0], &[]), 0, "previous proc");
    assert_eq!(t.call("GetWindowLongPtrW", [kid as u64, (-4i64) as u64, 0, 0], &[]), 0x4000);
    let n = t.ws("Top");
    assert_eq!(t.call("FindWindowW", [0, n, 0, 0], &[]), top as u64);
}

#[test]
fn focus_capture_and_coordinates() {
    let mut t = T::new();
    let a = t.win("A", WS_VISIBLE, 0, 0, 50, 40);
    let b = t.win("B", WS_VISIBLE, 0, 0, 300, 200);
    t.call("SetActiveWindow", [a as u64, 0, 0, 0], &[]);
    assert_eq!(t.call("GetActiveWindow", [0; 4], &[]), a as u64);
    assert_eq!(t.call("SetCapture", [b as u64, 0, 0, 0], &[]), 0);
    assert_eq!(t.call("GetCapture", [0; 4], &[]), b as u64);
    t.call("ReleaseCapture", [0; 4], &[]);
    assert_eq!(t.call("GetCapture", [0; 4], &[]), 0);
    let pt = t.mem(8);
    t.p.mem.write(pt, &[10, 0, 0, 0, 5, 0, 0, 0]).unwrap();
    assert_eq!(t.call("ClientToScreen", [b as u64, pt, 0, 0], &[]), 1);
    let (sx, sy) = (t.p.mem.read_u32(pt).unwrap() as i32, t.p.mem.read_u32(pt + 4).unwrap() as i32);
    assert!(sx >= 310 && sy >= 205, "client origin is offset by the window position: {sx},{sy}");
    assert_eq!(t.call("ScreenToClient", [b as u64, pt, 0, 0], &[]), 1);
    assert_eq!((t.p.mem.read_u32(pt).unwrap(), t.p.mem.read_u32(pt + 4).unwrap()), (10, 5));
}

#[test]
fn set_window_pos_moves_and_resizes() {
    let mut t = T::new();
    let a = t.win("A", WS_VISIBLE, 0, 0, 50, 40);
    // SetWindowPos(h, 0, x, y, cx, cy, flags=NOZORDER)
    assert_eq!(t.call("SetWindowPos", [a as u64, 0, 120, 90], &[300, 150, 4]), 1);
    let r = t.w.get_window_rect(a).unwrap();
    assert_eq!((r.left, r.top, r.width(), r.height()), (120, 90, 300, 150));
    // NOSIZE | NOMOVE leave the rectangle alone.
    t.call("SetWindowPos", [a as u64, 0, 1, 2], &[3, 4, 1 | 2 | 4]);
    assert_eq!(t.w.get_window_rect(a).unwrap(), r);
    t.call("SetWindowPos", [a as u64, 0, 0, 0], &[0, 0, 1 | 2 | 4 | 0x80]);
    assert!(!t.w.is_window_visible(a), "SWP_HIDEWINDOW");
}

#[test]
fn memory_dc_pixels_blit_and_text_metrics() {
    let mut t = T::new();
    let win = t.win("A", WS_VISIBLE, 0, 0, 0, 0);
    let wdc = t.call("GetDC", [win as u64, 0, 0, 0], &[]);
    let mdc = t.call("CreateCompatibleDC", [wdc, 0, 0, 0], &[]);
    assert_ne!(mdc, 0);
    let bmp = t.call("CreateCompatibleBitmap", [wdc, 16, 16, 0], &[]);
    assert_ne!(bmp, 0);
    t.call("SelectObject", [mdc, bmp, 0, 0], &[]);
    assert_eq!(t.call("SetPixel", [mdc, 3, 4, 0x00_33_22_11], &[]), 0x33_22_11);
    assert_eq!(t.call("GetPixel", [mdc, 3, 4, 0], &[]), 0x33_22_11);
    // BitBlt(dst, dx, dy, w, h, src, sx, sy, SRCCOPY) copies the pixel; other rops are refused.
    assert_eq!(t.call("BitBlt", [wdc, 20, 20, 16], &[16, mdc, 0, 0, 0x00CC_0020]), 1);
    assert_eq!(t.call("GetPixel", [wdc, 23, 24, 0], &[]), 0x33_22_11);
    assert_eq!(t.call("BitBlt", [wdc, 0, 0, 1], &[1, mdc, 0, 0, 0x0042_0042]), 0);
    // Text metrics grow with the font size.
    let (s, sz) = (t.ws("Hello"), t.mem(8));
    assert_eq!(t.call("GetTextExtentPoint32W", [wdc, s, 5, sz], &[]), 1);
    let w0 = t.p.mem.read_u32(sz).unwrap();
    let big = t.call("CreateFontW", [32, 0, 0, 0], &[]);
    t.call("SelectObject", [wdc, big, 0, 0], &[]);
    t.call("GetTextExtentPoint32W", [wdc, s, 5, sz], &[]);
    assert!(t.p.mem.read_u32(sz).unwrap() > w0, "bigger font, wider text");
    let tm = t.mem(64);
    assert_eq!(t.call("GetTextMetricsW", [wdc, tm, 0, 0], &[]), 1);
    assert!(t.p.mem.read_u32(tm).unwrap() >= 32, "tmHeight");
    assert_eq!(t.call("DeleteDC", [mdc, 0, 0, 0], &[]), 1);
}

#[test]
fn update_rect_and_validate() {
    let mut t = T::new();
    let a = t.win("A", WS_VISIBLE, 0, 0, 0, 0);
    let rc = t.mem(16);
    t.call("ValidateRect", [a as u64, 0, 0, 0], &[]);
    assert_eq!(t.call("GetUpdateRect", [a as u64, rc, 0, 0], &[]), 0, "nothing invalid");
    t.call("InvalidateRect", [a as u64, 0, 1, 0], &[]);
    assert_eq!(t.call("GetUpdateRect", [a as u64, rc, 0, 0], &[]), 1);
    assert_eq!(t.p.mem.read_u32(rc + 8).unwrap(), 200, "whole client area");
    t.call("ValidateRect", [a as u64, 0, 0, 0], &[]);
    assert_eq!(t.call("GetUpdateRect", [a as u64, rc, 0, 0], &[]), 0);
}

fn rc(t: &mut T, l: i32, tp: i32, r: i32, b: i32) -> u64 {
    let a = t.mem(16);
    for (i, x) in [l, tp, r, b].iter().enumerate() {
        t.p.mem.write(a + 4 * i as u64, &x.to_le_bytes()).unwrap();
    }
    a
}
fn rd(t: &mut T, a: u64) -> [i32; 4] {
    let b = t.p.mem.read_vec(a, 16).unwrap();
    std::array::from_fn(|i| i32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()))
}

#[test]
fn rect_helpers_follow_win32_semantics() {
    let mut t = T::new();
    let (a, b, d) = (rc(&mut t, 0, 0, 10, 10), rc(&mut t, 5, 5, 20, 20), rc(&mut t, 0, 0, 0, 0));
    assert_eq!(t.call("IntersectRect", [d, a, b, 0], &[]), 1);
    assert_eq!(rd(&mut t, d), [5, 5, 10, 10]);
    let far = rc(&mut t, 50, 50, 60, 60);
    assert_eq!(t.call("IntersectRect", [d, a, far, 0], &[]), 0);
    assert_eq!(rd(&mut t, d), [0, 0, 0, 0], "empty result is zeroed");
    assert_eq!(t.call("UnionRect", [d, a, b, 0], &[]), 1);
    assert_eq!(rd(&mut t, d), [0, 0, 20, 20]);
    t.call("OffsetRect", [a, 3, (-2i64) as u64, 0], &[]);
    assert_eq!(rd(&mut t, a), [3, -2, 13, 8]);
    t.call("InflateRect", [a, 1, 2, 0], &[]);
    assert_eq!(rd(&mut t, a), [2, -4, 14, 10]);
    let pt = |x: i32, y: i32| (x as u32 as u64) | (y as u32 as u64) << 32;
    assert_eq!(t.call("PtInRect", [a, pt(2, 0), 0, 0], &[]), 1);
    assert_eq!(t.call("PtInRect", [a, pt(14, 0), 0, 0], &[]), 0, "right edge is exclusive");
    assert_eq!(t.call("IsRectEmpty", [d, 0, 0, 0], &[]), 0);
    t.call("SetRectEmpty", [d, 0, 0, 0], &[]);
    assert_eq!(t.call("IsRectEmpty", [d, 0, 0, 0], &[]), 1);
    t.call("SetRect", [d, 1, 2, 3], &[4]);
    assert_eq!(rd(&mut t, d), [1, 2, 3, 4]);
    let c = rc(&mut t, 0, 0, 0, 0);
    t.call("CopyRect", [c, d, 0, 0], &[]);
    assert_eq!(t.call("EqualRect", [c, d, 0, 0], &[]), 1);
    // Overlapped window with a menu: caption + frame grow the outer rect.
    let cl = rc(&mut t, 0, 0, 300, 200);
    t.call("AdjustWindowRect", [cl, 0x00CF_0000, 1, 0], &[]);
    assert_eq!(rd(&mut t, cl), [-2, -48, 302, 202], "border 2, caption 24, menu 22");
}

#[test]
fn shapes_fill_and_stretch() {
    let mut t = T::new();
    let win = t.win("A", WS_VISIBLE, 0, 0, 0, 0);
    let dc = t.call("GetDC", [win as u64, 0, 0, 0], &[]);
    let black = t.call("GetStockObject", [4, 0, 0, 0], &[]); // BLACK_BRUSH
    t.call("SelectObject", [dc, black, 0, 0], &[]);
    assert_eq!(t.call("Ellipse", [dc, 10, 10, 30], &[30]), 1);
    assert_eq!(t.call("GetPixel", [dc, 20, 20, 0], &[]), 0, "centre filled black");
    assert_eq!(t.call("GetPixel", [dc, 10, 10, 0], &[]), 0xff_ffff, "bounding-box corner stays background");
    // Triangle: interior filled, outside not.
    let pts = t.mem(24);
    for (i, (x, y)) in [(60i32, 10i32), (100, 10), (80, 50)].iter().enumerate() {
        t.p.mem.write(pts + 8 * i as u64, &x.to_le_bytes()).unwrap();
        t.p.mem.write(pts + 8 * i as u64 + 4, &y.to_le_bytes()).unwrap();
    }
    assert_eq!(t.call("Polygon", [dc, pts, 3, 0], &[]), 1);
    assert_eq!(t.call("GetPixel", [dc, 80, 20, 0], &[]), 0);
    assert_eq!(t.call("GetPixel", [dc, 62, 40, 0], &[]), 0xff_ffff);
    // PatBlt WHITENESS / DSTINVERT / BLACKNESS.
    assert_eq!(t.call("PatBlt", [dc, 0, 80, 10], &[10, 0x00FF_0062]), 1);
    assert_eq!(t.call("GetPixel", [dc, 3, 83, 0], &[]), 0xff_ffff);
    assert_eq!(t.call("PatBlt", [dc, 0, 80, 10], &[10, 0x0055_0009]), 1);
    assert_eq!(t.call("GetPixel", [dc, 3, 83, 0], &[]), 0);
    assert_eq!(t.call("PatBlt", [dc, 0, 80, 10], &[10, 0x1234_5678]), 0, "unsupported rop");
    // StretchBlt 2x2 -> 4x4 doubles each source pixel.
    let mdc = t.call("CreateCompatibleDC", [dc, 0, 0, 0], &[]);
    let bmp = t.call("CreateCompatibleBitmap", [dc, 2, 2, 0], &[]);
    t.call("SelectObject", [mdc, bmp, 0, 0], &[]);
    t.call("SetPixel", [mdc, 1, 0, 0x0000_00ff], &[]);
    assert_eq!(t.call("StretchBlt", [dc, 120, 60, 4], &[4, mdc, 0, 0, 2, 2, 0x00CC_0020]), 1);
    assert_eq!(t.call("GetPixel", [dc, 122, 60, 0], &[]), 0xff);
    assert_eq!(t.call("GetPixel", [dc, 123, 61, 0], &[]), 0xff);
    assert_eq!(t.call("GetPixel", [dc, 121, 60, 0], &[]), 0xff_ffff);
}

#[test]
fn system_colors_and_metrics() {
    let mut t = T::new();
    assert_eq!(t.call("GetSysColor", [5, 0, 0, 0], &[]), 0xff_ffff, "COLOR_WINDOW");
    assert_eq!(t.call("GetSysColor", [15, 0, 0, 0], &[]), 0xc8_d0_d4, "COLOR_BTNFACE");
    assert_ne!(t.call("GetSysColor", [13, 0, 0, 0], &[]), t.call("GetSysColor", [15, 0, 0, 0], &[]), "highlight differs from face");
    assert_eq!(t.call("GetSystemMetrics", [4, 0, 0, 0], &[]), 24);
    assert_eq!(t.call("GetSystemMetrics", [2, 0, 0, 0], &[]), 17);
    assert_eq!(t.call("GetDeviceCaps", [0, 88, 0, 0], &[]), 96);
}
