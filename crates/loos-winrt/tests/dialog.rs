//! Resources, dialog templates (modeless + modal), and DIBs through the user32/gdi32 entry points.
use loos_win32k::{Proc, Win32k, WndClass, WS_VISIBLE};
use loos_winrt::gui::lookup;
use loos_winrt::rsrc::{self, ResId};
use loos_winrt::{Process, Ret};
use std::{cell::RefCell, rc::Rc};

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

fn wz(s: &str) -> Vec<u8> {
    let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    b.extend_from_slice(&[0, 0]);
    b
}

impl T {
    fn new() -> T {
        let mut p = Process::create(1, 1, "t.exe", "", SVC).unwrap();
        let mut w = Win32k::new(800, 600);
        w.set_current_thread(p.threads[0].tid as u32);
        w.register_class(WndClass { name: "Main".into(), style: 0, proc_: Proc::Native(main_proc), background: 6, wnd_extra: 2 }).unwrap();
        let buf = p.mem.map_anywhere(0x20000, 3, &[], "buf").unwrap();
        T { p, w, buf, at: 0x100 }
    }
    fn put(&mut self, data: &[u8]) -> u64 {
        let a = self.buf + self.at;
        self.at += (data.len() as u64 + 15) & !7;
        self.p.mem.write(a, data).unwrap();
        a
    }
    fn mem(&mut self, n: u64) -> u64 {
        let a = self.buf + self.at;
        self.at += (n + 15) & !7;
        a
    }
    fn raw(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> Ret {
        let rsp = self.buf + 0x1f000;
        for (k, v) in stack.iter().enumerate() {
            self.p.mem.write_u64(rsp + 0x28 + 8 * k as u64, *v).unwrap();
        }
        self.p.gui_syscall(lookup(name).unwrap_or_else(|| panic!("{name}")), a, rsp, 10, &mut self.w)
    }
    fn call(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> u64 {
        match self.raw(name, a, stack) {
            Ret::Value(v) => v,
            r => panic!("{name}: {r:?}"),
        }
    }
}

/// DLGTEMPLATE: a captioned dialog "Hi" (200x80 du) with an OK button and a static label.
fn template() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&(0x8000_0000u32 | 0x00C0_0000 | 0x0008_0000 | WS_VISIBLE).to_le_bytes()); // POPUP|CAPTION|SYSMENU|VISIBLE
    t.extend_from_slice(&0u32.to_le_bytes());
    t.extend_from_slice(&2u16.to_le_bytes());
    for v in [0i16, 0, 200, 80] {
        t.extend_from_slice(&v.to_le_bytes());
    }
    t.extend_from_slice(&[0, 0, 0, 0]); // menu, class
    t.extend_from_slice(&wz("Hi"));
    let item = |t: &mut Vec<u8>, style: u32, x: i16, id: u16, class: u16, text: &str| {
        while t.len() % 4 != 0 {
            t.push(0);
        }
        t.extend_from_slice(&(style | 0x4000_0000 | 0x1000_0000 | 0x0001_0000).to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        for v in [x, 40, 60, 14] {
            t.extend_from_slice(&v.to_le_bytes());
        }
        t.extend_from_slice(&id.to_le_bytes());
        t.extend_from_slice(&[0xff, 0xff]);
        t.extend_from_slice(&class.to_le_bytes());
        t.extend_from_slice(&wz(text));
        t.extend_from_slice(&0u16.to_le_bytes());
    };
    item(&mut t, 0, 10, 1, 0x80, "OK");
    item(&mut t, 0, 80, 7, 0x82, "Name");
    t
}

/// Fake module with `.rsrc`: RT_DIALOG/100 and RT_STRING block 1 (string index 3 = "Hello").
fn module(t: &mut T) -> u64 {
    let base = t.mem(0x2000);
    let base = (base + 0xfff) & !0xfff;
    let dlg = template();
    let mut strs = Vec::new();
    for i in 0..16 {
        let s: &str = if i == 3 { "Hello" } else { "" };
        strs.extend_from_slice(&(s.encode_utf16().count() as u16).to_le_bytes());
        strs.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    }
    // Directory layout at rva 0x200: root(2 id entries) -> type dirs -> name dirs -> lang dirs -> data entries.
    let mut img = vec![0u8; 0x1000];
    img[0..2].copy_from_slice(b"MZ");
    img[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    img[0x40..0x44].copy_from_slice(b"PE\0\0");
    let dd = 0x40 + 24 + 112 + 16;
    img[dd..dd + 4].copy_from_slice(&0x200u32.to_le_bytes());
    img[dd + 4..dd + 8].copy_from_slice(&0x300u32.to_le_bytes());
    let put32 = |img: &mut Vec<u8>, at: usize, v: u32| img[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let dir = |img: &mut Vec<u8>, at: usize, entries: &[(u32, u32)]| {
        img[at + 14..at + 16].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        for (i, (k, o)) in entries.iter().enumerate() {
            put32(img, at + 16 + 8 * i, *k);
            put32(img, at + 20 + 8 * i, *o);
        }
    };
    let sub = 0x8000_0000u32;
    dir(&mut img, 0x200, &[(5, sub | 0x30), (6, sub | 0x50)]); // types: RT_DIALOG, RT_STRING
    dir(&mut img, 0x230, &[(100, sub | 0x70)]); // dialog names
    dir(&mut img, 0x250, &[(1, sub | 0x90)]); // string block 1
    dir(&mut img, 0x270, &[(0x409, 0xb0)]); // lang
    dir(&mut img, 0x290, &[(0x409, 0xc0)]);
    // data entries at 0x2b0 (dialog) and 0x2c0 (strings); payloads at 0x400 / 0x800
    put32(&mut img, 0x2b0, 0x400);
    put32(&mut img, 0x2b4, dlg.len() as u32);
    put32(&mut img, 0x2c0, 0x800);
    put32(&mut img, 0x2c4, strs.len() as u32);
    img[0x400..0x400 + dlg.len()].copy_from_slice(&dlg);
    img[0x800..0x800 + strs.len()].copy_from_slice(&strs);
    t.p.mem.write(base, &img).unwrap();
    base
}

#[test]
fn resources_strings_and_template_parsing() {
    let mut t = T::new();
    let m = module(&mut t);
    assert!(rsrc::find(&mut t.p.mem, m, &ResId::Id(5), &ResId::Id(100)).is_some());
    assert!(rsrc::find(&mut t.p.mem, m, &ResId::Id(5), &ResId::Id(101)).is_none());
    let buf = t.mem(64);
    assert_eq!(t.call("LoadStringW", [m, 3 + 16, buf, 32], &[]), 0, "string 19 lives in block 2, absent");
    assert_eq!(t.call("LoadStringW", [m, 3, buf, 32], &[]), 5);
    assert_eq!(t.p.mem.read_vec(buf, 12).unwrap(), wz("Hello"));
    t.p.mem.write_u32(buf + 60, 0).unwrap();
    let r = t.call("FindResourceW", [m, 5, 100, 0], &[]);
    assert_ne!(r, 0);
    assert_eq!(t.call("SizeofResource", [m, r, 0, 0], &[]), template().len() as u64);
    assert_eq!(t.call("LockResource", [r, 0, 0, 0], &[]), r);
    assert_eq!(t.call("FindResourceW", [m, 5, 999, 0], &[]), 0);
    let d = rsrc::parse_dialog(&template()).unwrap();
    assert_eq!((d.title.as_str(), d.items.len(), d.client), ("Hi", 2, (300, 130)));
    assert_eq!((d.items[0].class.as_str(), d.items[0].text.as_str(), d.items[0].id, d.items[0].x), ("Button", "OK", 1, 15));
    assert_eq!(d.items[1].class, "Static");
}

type Log = Rc<RefCell<Vec<(u32, u32)>>>;

/// Installs a DLGPROC: logs (msg, wparam-low) and ends the dialog with 42 on WM_COMMAND/IDOK.
fn install_dlgproc(t: &mut T) -> Log {
    let log: Log = Rc::default();
    let l2 = log.clone();
    t.w.set_upcall(Box::new(move |k, _addr, h, msg, wp, _lp| {
        l2.borrow_mut().push((msg, wp as u32));
        if msg == 0x111 && wp & 0xffff == 1 {
            k.end_dialog(h, 42);
            return 1;
        }
        if msg == 0x110 { 1 } else { 0 }
    }));
    log
}

#[test]
fn modeless_dialog_from_resource_with_item_helpers() {
    let mut t = T::new();
    let m = module(&mut t);
    let log = install_dlgproc(&mut t);
    let dlg = t.call("CreateDialogParamW", [m, 100, 0, 0x1234], &[0]) as u32;
    assert_ne!(dlg, 0);
    assert_eq!(log.borrow()[0].0, 0x110, "WM_INITDIALOG first");
    assert!(t.w.is_window_visible(dlg));
    assert_eq!(t.w.get_window_text(dlg).unwrap(), "Hi");
    let ok = t.call("GetDlgItem", [dlg as u64, 1, 0, 0], &[]);
    assert_ne!(ok, 0);
    // SetDlgItemTextW / GetDlgItemTextW / int helpers / check state.
    let (s, out) = (t.put(&wz("Alice")), t.mem(64));
    assert_eq!(t.call("SetDlgItemTextW", [dlg as u64, 7, s, 0], &[]), 1);
    assert_eq!(t.call("GetDlgItemTextW", [dlg as u64, 7, out, 32], &[]), 5);
    assert_eq!(t.p.mem.read_vec(out, 12).unwrap(), wz("Alice"));
    t.call("SetDlgItemInt", [dlg as u64, 7, (-5i64) as u64, 1], &[]);
    let ok_flag = t.mem(4);
    assert_eq!(t.call("GetDlgItemInt", [dlg as u64, 7, ok_flag, 1], &[]) as i64, -5);
    assert_eq!(t.p.mem.read_u32(ok_flag).unwrap(), 1);
    t.call("CheckDlgButton", [dlg as u64, 1, 1, 0], &[]);
    assert_eq!(t.call("IsDlgButtonChecked", [dlg as u64, 1, 0, 0], &[]), 1);
    // Clicking OK reaches the DLGPROC as WM_COMMAND and ends the dialog.
    t.w.send_message(dlg, 0x111, 1, 0);
    assert!(!t.w.is_window(dlg));
    assert_eq!(t.w.message_box_result(dlg, 0), Some(42));
}

#[test]
fn modal_dialog_box_blocks_until_end_dialog() {
    let mut t = T::new();
    let owner = t.w.create_window_ex(0, "Main", "Owner", WS_VISIBLE, 0, 0, 300, 200, 0, 0, 0).unwrap();
    let tpl = t.put(&template());
    let _log = install_dlgproc(&mut t);
    let args = [0u64, tpl, owner as u64, 0x4000];
    // Empty queue: the call parks like GetMessage and the owner is disabled.
    assert_eq!(t.raw("DialogBoxIndirectParamW", args, &[0]), Ret::Idle);
    assert!(!t.w.is_window_enabled(owner));
    assert_eq!(t.raw("DialogBoxIndirectParamW", args, &[0]), Ret::Idle, "retry keeps waiting, no second dialog");
    let dlgs: Vec<u32> = t.w.top_levels().into_iter().filter(|h| *h != owner).collect();
    assert_eq!(dlgs.len(), 1);
    // The user presses OK (a queued WM_COMMAND is pumped by the nested loop).
    t.w.post_message(dlgs[0], 0x111, 1, 0).unwrap();
    assert_eq!(t.raw("DialogBoxIndirectParamW", args, &[0]), Ret::Value(42));
    assert!(t.w.is_window_enabled(owner), "owner re-enabled");
    assert!(!t.w.is_window(dlgs[0]));
}

fn dib24(w: i32, h: i32, px: &[[u8; 3]]) -> (Vec<u8>, Vec<u8>) {
    // Bottom-up 24 bpp; `px` is given top-down as [r,g,b].
    let mut hdr = vec![0u8; 40];
    hdr[0..4].copy_from_slice(&40u32.to_le_bytes());
    hdr[4..8].copy_from_slice(&w.to_le_bytes());
    hdr[8..12].copy_from_slice(&h.to_le_bytes());
    hdr[12..14].copy_from_slice(&1u16.to_le_bytes());
    hdr[14..16].copy_from_slice(&24u16.to_le_bytes());
    let st = loos_winrt::dib::stride(w, 24);
    let mut bits = vec![0u8; st * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let [r, g, b] = px[y * w as usize + x];
            let o = (h as usize - 1 - y) * st + 3 * x;
            bits[o..o + 3].copy_from_slice(&[b, g, r]);
        }
    }
    (hdr, bits)
}

#[test]
fn dibs_draw_create_section_and_get_bits() {
    let mut t = T::new();
    let win = t.w.create_window_ex(0, "Main", "A", WS_VISIBLE, 0, 0, 100, 100, 0, 0, 0).unwrap();
    let dc = t.call("GetDC", [win as u64, 0, 0, 0], &[]);
    // 2x2 image: red green / blue white. StretchDIBits 1:1 then 2x upscale.
    let (hdr, bits) = dib24(2, 2, &[[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]);
    let (bmi, bp) = (t.put(&hdr), t.put(&bits));
    // (hdc, xD, yD, wD, hD, xS, yS, wS, hS, bits, bmi, usage, rop)
    assert_eq!(t.call("StretchDIBits", [dc, 10, 10, 2], &[2, 0, 0, 2, 2, bp, bmi, 0, 0x00CC_0020]), 2);
    assert_eq!(t.call("GetPixel", [dc, 10, 10, 0], &[]), 0x0000_00ff, "red (COLORREF 0x00BBGGRR)");
    assert_eq!(t.call("GetPixel", [dc, 11, 10, 0], &[]), 0x0000_ff00);
    assert_eq!(t.call("GetPixel", [dc, 10, 11, 0], &[]), 0x00ff_0000, "blue is the bottom-left pixel");
    assert_eq!(t.call("StretchDIBits", [dc, 20, 20, 4], &[4, 0, 0, 2, 2, bp, bmi, 0, 0x00CC_0020]), 2);
    assert_eq!(t.call("GetPixel", [dc, 23, 23, 0], &[]), 0xff_ffff, "2x upscale reaches the far corner");

    // CreateDIBSection: the program writes the bits directly; BitBlt picks them up.
    let ppv = t.mem(8);
    let sec = t.call("CreateDIBSection", [dc, bmi, 0, ppv], &[0, 0]);
    assert_ne!(sec, 0);
    let addr = t.p.mem.read_u64(ppv).unwrap();
    t.p.mem.write(addr, &bits).unwrap();
    let mdc = t.call("CreateCompatibleDC", [dc, 0, 0, 0], &[]);
    t.call("SelectObject", [mdc, sec, 0, 0], &[]);
    assert_eq!(t.call("BitBlt", [dc, 50, 50, 2], &[2, mdc, 0, 0, 0x00CC_0020]), 1);
    assert_eq!(t.call("GetPixel", [dc, 51, 51, 0], &[]), 0xff_ffff);
    assert_eq!(t.call("GetPixel", [dc, 50, 50, 0], &[]), 0x0000_00ff);
    // GetObject reports the DIB section's bits pointer; GetDIBits returns the geometry then the pixels.
    let bm = t.mem(32);
    assert_eq!(t.call("GetObjectW", [sec, 32, bm, 0], &[]), 32);
    assert_eq!(t.p.mem.read_u64(bm + 24).unwrap(), addr);
    let q = t.put(&{
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h
    });
    assert_eq!(t.call("GetDIBits", [dc, sec, 0, 2], &[0, q, 0]), 1);
    assert_eq!(t.p.mem.read_u32(q + 4).unwrap(), 2);
    assert_eq!(t.p.mem.read_u32(q + 8).unwrap(), 2);
    assert_eq!(t.p.mem.read_vec(q + 14, 2).unwrap(), 32u16.to_le_bytes());
    t.p.mem.write(q + 14, &24u16.to_le_bytes()).unwrap();
    let out = t.mem(32);
    assert_eq!(t.call("GetDIBits", [dc, sec, 0, 2], &[out, q, 0]), 2);
    assert_eq!(t.p.mem.read_vec(out, 6).unwrap(), bits[0..6], "bottom-up rows round trip");
    // CreateDIBitmap with CBM_INIT, then SetDIBits replaces its content.
    let hb = t.call("CreateDIBitmap", [dc, bmi, 4, bp], &[bmi, 0]);
    assert_ne!(hb, 0);
    let px = t.w.bitmap_pixels(hb as u32).unwrap().to_vec();
    assert_eq!(px, vec![0xffff0000, 0xff00ff00, 0xff0000ff, 0xffffffff]);
}
