use loos_win32k::*;

fn hello_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_PAINT => {
            let ps = k.begin_paint(h).unwrap();
            k.set_text_color(ps.hdc, rgb(0, 0, 128));
            k.set_bk_mode(ps.hdc, 1);
            k.text_out(ps.hdc, 10, 10, "Hello, LoOS");
            let br = k.create_solid_brush(rgb(255, 0, 0));
            k.fill_rect(ps.hdc, Rect::new(10, 50, 60, 80), br);
            k.end_paint(h, &ps);
            0
        }
        WM_DESTROY => {
            k.post_quit_message(7);
            0
        }
        _ => k.def_window_proc(h, msg, w, l),
    }
}

fn setup() -> (Win32k, Hwnd) {
    let mut k = Win32k::new(640, 480);
    k.register_class(WndClass { name: "Hello".into(), style: CS_HREDRAW | CS_VREDRAW, proc_: Proc::Native(hello_proc), background: COLOR_WINDOW as u32 + 1, wnd_extra: 0 }).unwrap();
    let h = k.create_window_ex(0, "Hello", "Hello Window", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 50, 40, 300, 200, 0, 0, 0).unwrap();
    (k, h)
}

fn pump(k: &mut Win32k) -> Option<i32> {
    for _ in 0..1000 {
        match k.get_message(0, 0, 0) {
            GetMsg::Message(m) => {
                k.translate_message(&m);
                k.dispatch_message(&m);
            }
            GetMsg::Quit(c) => return Some(c),
            GetMsg::Idle => return None,
        }
    }
    panic!("message loop did not settle");
}

#[test]
fn hello_window_paints_text_and_rect() {
    let (mut k, h) = setup();
    assert_eq!(pump(&mut k), None);
    let (px, cw, ch) = k.window_pixels(h).unwrap();
    assert_eq!((cw, ch), (300 - 4, 200 - 4 - CAPTION_H));
    assert_eq!(px[(60 * cw + 20) as usize], 0xFFFF0000, "red rect");
    assert_eq!(px[(100 * cw + 200) as usize], 0xFFFFFFFF, "class background erased white");
    assert!(px[..(40 * cw) as usize].iter().any(|&p| p != 0xFFFFFFFF), "text pixels drawn");
    let mut fb = vec![0u32; 640 * 480];
    k.compose(&mut fb);
    assert_eq!(fb[5], k.desktop_color);
    // title bar of the active window is blue, client origin shows the red rect
    assert_eq!(fb[(40 + 2 + 5) * 640 + 60], 0xFF0A246A);
    assert_eq!(fb[(40 + 2 + CAPTION_H as usize + 60) * 640 + 50 + 2 + 20], 0xFFFF0000);
}

#[test]
fn close_button_click_destroys_and_quits() {
    let (mut k, h) = setup();
    pump(&mut k);
    let b = k.close_button(h);
    k.inject_mouse_move(b.left + 5, b.top + 5);
    k.inject_mouse_button(0, true);
    k.inject_mouse_button(0, false);
    assert_eq!(pump(&mut k), Some(7));
    assert!(!k.is_window(h));
}

#[test]
fn caption_drag_moves_window_and_raises() {
    let (mut k, h) = setup();
    let h2 = k.create_window_ex(0, "Hello", "Other", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 400, 300, 100, 100, 0, 0, 0).unwrap();
    pump(&mut k);
    assert_eq!(k.get_active_window(), h2);
    k.inject_mouse_move(100, 50); // caption of first window
    k.inject_mouse_button(0, true);
    assert_eq!(k.get_active_window(), h);
    k.inject_mouse_move(160, 90);
    k.inject_mouse_button(0, false);
    let r = k.get_window_rect(h).unwrap();
    assert_eq!((r.left, r.top), (110, 80));
    assert_eq!(*k.top_levels().last().unwrap(), h);
}

static mut LOG: Vec<String> = Vec::new();
fn child_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    #[allow(static_mut_refs)]
    unsafe {
        if msg == WM_LBUTTONDOWN {
            LOG.push(format!("click {} {}", lparam_x(l), lparam_y(l)));
        }
        if msg == WM_CHAR {
            LOG.push(format!("char {}", char::from_u32(w as u32).unwrap()));
        }
    }
    k.def_window_proc(h, msg, w, l)
}

#[test]
fn child_hit_testing_focus_keys_and_timer() {
    let (mut k, h) = setup();
    k.register_class(WndClass { name: "Btn".into(), style: 0, proc_: Proc::Native(child_proc), background: COLOR_BTNFACE as u32 + 1, wnd_extra: 2 }).unwrap();
    let c = k.create_window_ex(0, "Btn", "ok", WS_CHILD | WS_VISIBLE | WS_BORDER, 20, 30, 80, 24, h, 101, 0).unwrap();
    assert_eq!(k.get_parent(c).unwrap(), h);
    k.set_extra(c, 1, 99).unwrap();
    assert_eq!(k.get_extra(c, 1).unwrap(), 99);
    assert!(k.get_extra(c, 2).is_err());
    pump(&mut k);
    let o = k.client_to_screen(c, Point { x: 5, y: 5 }).unwrap();
    k.inject_mouse_move(o.x, o.y);
    k.inject_mouse_button(0, true);
    k.inject_mouse_button(0, false);
    assert_eq!(k.get_focus(), c);
    pump(&mut k);
    k.inject_key(VK_SHIFT, true);
    k.inject_key(b'H' as u32, true);
    pump(&mut k); // translation uses the live shift state, as on a real thread
    k.inject_key(b'H' as u32, false);
    k.inject_key(VK_SHIFT, false);
    k.inject_key(b'I' as u32, true);
    pump(&mut k);
    #[allow(static_mut_refs)]
    let log = unsafe { LOG.clone() };
    assert_eq!(log, vec!["click 5 5", "char H", "char i"]);
    // child bitmap painted with BTNFACE via class background
    let (px, w, _) = k.window_pixels(c).unwrap();
    assert_eq!(px[(5 * w + 5) as usize], 0xFFD4D0C8);
    // timers
    k.set_timer(h, 5, 100);
    assert_eq!(k.get_message(0, WM_TIMER, WM_TIMER), GetMsg::Idle);
    k.advance_time(100);
    match k.get_message(0, WM_TIMER, WM_TIMER) {
        GetMsg::Message(m) => assert_eq!((m.hwnd, m.wparam), (h, 5)),
        o => panic!("{o:?}"),
    }
    assert!(k.kill_timer(h, 5));
}

#[test]
fn maximize_restore_and_userdata_and_find() {
    let (mut k, h) = setup();
    assert_eq!(k.find_window(Some("hello"), Some("Hello Window")), h);
    k.set_user_data(h, 1234).unwrap();
    k.show_window(h, SW_SHOWMAXIMIZED).unwrap();
    assert_eq!(k.get_window_rect(h).unwrap(), Rect::new(0, 0, 640, 480));
    k.show_window(h, SW_RESTORE).unwrap();
    assert_eq!(k.get_window_rect(h).unwrap(), Rect::new(50, 40, 350, 240));
    assert_eq!(k.get_user_data(h).unwrap(), 1234);
    assert_eq!(k.register_class(WndClass { name: "HELLO".into(), style: 0, proc_: Proc::Native(hello_proc), background: 0, wnd_extra: 0 }), Err(Win32kError::ClassExists));
    k.destroy_window(h).unwrap();
    assert!(k.get_window_rect(h).is_err());
}

#[test]
fn user_proc_goes_through_upcall() {
    let mut k = Win32k::new(320, 240);
    k.set_upcall(Box::new(|k, addr, h, msg, w, l| {
        assert_eq!(addr, 0x401000);
        if msg == WM_CREATE { 0 } else { k.def_window_proc(h, msg, w, l) }
    }));
    k.register_class(WndClass { name: "U".into(), style: 0, proc_: Proc::User(0x401000), background: 0, wnd_extra: 0 }).unwrap();
    assert!(k.create_window_ex(0, "U", "", WS_POPUP | WS_VISIBLE, 0, 0, 50, 50, 0, 0, 0).is_ok());
}
