use loos_win32k::*;

struct Guest(Vec<u8>); // flat memory at 0x10000
impl GuestMem for Guest {
    fn read(&mut self, a: u64, b: &mut [u8]) -> bool {
        let o = a.wrapping_sub(0x10000) as usize;
        self.0.get(o..o + b.len()).map(|s| b.copy_from_slice(s)).is_some()
    }
    fn write(&mut self, a: u64, d: &[u8]) -> bool {
        let o = a.wrapping_sub(0x10000) as usize;
        self.0.get_mut(o..o + d.len()).map(|s| s.copy_from_slice(d)).is_some()
    }
}
impl Guest {
    fn put(&mut self, at: u64, b: &[u8]) {
        assert!(self.write(at, b));
    }
    fn wide(&mut self, at: u64, s: &str) -> u64 {
        let v: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        self.put(at, &v);
        at
    }
    fn q(&mut self, at: u64) -> u64 {
        let mut b = [0u8; 8];
        assert!(self.read(at, &mut b));
        u64::from_le_bytes(b)
    }
}

#[test]
fn guest_program_registers_class_creates_window_pumps_and_paints_via_services() {
    let mut k = Win32k::new(640, 480);
    let mut g = Guest(vec![0u8; 0x10000]);
    // The kernel's upcall: the guest WNDPROC at 0x401000 handles WM_PAINT itself (re-enters services)
    // and forwards everything else to DefWindowProc.
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u32>::new()));
    let l2 = log.clone();
    k.set_upcall(Box::new(move |k, addr, h, msg, w, l| {
        assert_eq!(addr, 0x401000);
        l2.borrow_mut().push(msg);
        match msg {
            WM_PAINT => {
                // guest code: BeginPaint; TextOut; EndPaint -- via the service ABI on a scratch buffer
                let mut sg = Guest(vec![0u8; 0x1000]);
                let dc = k.service(1, SVC_BEGIN_PAINT, [h as u64, 0x10000, 0, 0, 0, 0], &mut sg);
                assert!(dc > 0);
                let r = Rect::new(5, 5, 45, 25);
                let mut rb = [0u8; 16];
                for (i, v) in [r.left, r.top, r.right, r.bottom].iter().enumerate() {
                    rb[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                sg.put(0x10100, &rb);
                let brush = k.service(1, SVC_CREATE_BRUSH, [rgb(10, 20, 200) as u64, 0, 0, 0, 0, 0], &mut sg);
                k.service(1, SVC_FILL_RECT, [dc as u64, 0x10100, brush as u64, 0, 0, 0], &mut sg);
                k.service(1, SVC_END_PAINT, [h as u64, 0x10000, 0, 0, 0, 0], &mut sg);
                0
            }
            WM_DESTROY => {
                k.post_quit_message(0);
                0
            }
            _ => k.def_window_proc(h, msg, w, l),
        }
    }));
    // WNDCLASSEXW
    let name = g.wide(0x10400, "GuestWnd");
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&0x401000u64.to_le_bytes());
    wc[48..56].copy_from_slice(&6u64.to_le_bytes()); // COLOR_WINDOW+1
    wc[64..72].copy_from_slice(&name.to_le_bytes());
    g.put(0x10200, &wc);
    assert!(k.service(1, SVC_REGISTER_CLASS, [0x10200, 0, 0, 0, 0, 0], &mut g) > 0);
    assert_eq!(k.service(1, SVC_REGISTER_CLASS, [0x10200, 0, 0, 0, 0, 0], &mut g), 0, "duplicate");
    assert_eq!(k.service(1, SVC_GET_LAST_ERROR, [0; 6], &mut g), 1410);
    // CreateWindowExW
    let title = g.wide(0x10500, "From a PE");
    let cw: [u64; 11] = [0, name, title, (WS_OVERLAPPEDWINDOW | WS_VISIBLE) as u64, 30, 30, 200, 150, 0, 0, 0];
    let b: Vec<u8> = cw.iter().flat_map(|v| v.to_le_bytes()).collect();
    g.put(0x10600, &b);
    let hwnd = k.service(1, SVC_CREATE_WINDOW, [0x10600, 0, 0, 0, 0, 0], &mut g);
    assert!(hwnd > 0);
    assert_eq!(k.get_window_text(hwnd as u32).unwrap(), "From a PE");
    // message loop: GetMessage / Translate / Dispatch until idle
    let mut iters = 0;
    loop {
        let r = k.service(1, SVC_GET_MESSAGE, [0x10700, 0, 0, 0, 0, 0], &mut g);
        if r == WOULD_BLOCK {
            break;
        }
        assert_eq!(r, 1);
        k.service(1, SVC_TRANSLATE_MESSAGE, [0x10700, 0, 0, 0, 0, 0], &mut g);
        k.service(1, SVC_DISPATCH_MESSAGE, [0x10700, 0, 0, 0, 0, 0], &mut g);
        iters += 1;
        assert!(iters < 50);
    }
    assert!(log.borrow().contains(&WM_PAINT) && log.borrow().contains(&WM_CREATE));
    let (px, w, _) = k.window_pixels(hwnd as u32).unwrap();
    assert_eq!(px[(10 * w + 10) as usize], 0xFF0A14C8, "guest-painted blue rect");
    // GetClientRect writes into guest memory
    assert_eq!(k.service(1, SVC_GET_CLIENT_RECT, [hwnd as u64, 0x10800, 0, 0, 0, 0], &mut g), 1);
    assert_eq!(g.q(0x10800) & 0xFFFF_FFFF, 0);
    // close -> WM_QUIT => GetMessage returns 0 with WM_QUIT in MSG
    k.service(1, SVC_DESTROY_WINDOW, [hwnd as u64, 0, 0, 0, 0, 0], &mut g);
    assert_eq!(k.service(1, SVC_GET_MESSAGE, [0x10700, 0, 0, 0, 0, 0], &mut g), 0);
    // bad guest pointers fail cleanly, unknown services too
    assert_eq!(k.service(1, SVC_REGISTER_CLASS, [0xdead0000, 0, 0, 0, 0, 0], &mut g), 0);
    assert_eq!(k.service(1, SVC_BASE + 999, [0; 6], &mut g), -38);
}

#[test]
fn c_header_for_pe_stubs_matches_service_numbers() {
    let h = include_str!("../../../src/win32/include/loos_win32k.h");
    let want = [
        ("SVC_REGISTER_CLASS", SVC_REGISTER_CLASS), ("SVC_CREATE_WINDOW", SVC_CREATE_WINDOW), ("SVC_GET_MESSAGE", SVC_GET_MESSAGE),
        ("SVC_BEGIN_PAINT", SVC_BEGIN_PAINT), ("SVC_TEXT_OUT", SVC_TEXT_OUT), ("SVC_MESSAGE_BOX", SVC_MESSAGE_BOX),
        ("SVC_MESSAGE_BOX_RESULT", SVC_MESSAGE_BOX_RESULT), ("SVC_GET_LAST_ERROR", SVC_GET_LAST_ERROR),
    ];
    for (name, n) in want {
        let line = h.lines().find(|l| l.starts_with(&format!("#define {name} "))).unwrap_or_else(|| panic!("{name} missing"));
        let off: u32 = line.split("+ ").nth(1).unwrap().trim_end_matches(')').trim().parse().unwrap();
        assert_eq!(SVC_BASE + off, n, "{name}");
    }
}
