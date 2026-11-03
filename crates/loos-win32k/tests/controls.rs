use loos_win32k::*;

fn pump(k: &mut Win32k) {
    for _ in 0..200 {
        match k.get_message(0, 0, 0) {
            GetMsg::Message(m) => {
                k.translate_message(&m);
                k.dispatch_message(&m);
            }
            _ => return,
        }
    }
}

static mut CMDS: Vec<usize> = Vec::new();
fn app_proc(k: &mut Win32k, h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_COMMAND {
        #[allow(static_mut_refs)]
        unsafe {
            CMDS.push(w);
        }
        return 0;
    }
    k.def_window_proc(h, msg, w, l)
}

fn app(k: &mut Win32k) -> Hwnd {
    k.register_class(WndClass { name: "App".into(), style: 0, proc_: Proc::Native(app_proc), background: COLOR_WINDOW as u32 + 1, wnd_extra: 0 }).unwrap();
    k.create_window_ex(0, "App", "App", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 20, 20, 400, 300, 0, 0, 0).unwrap()
}

fn click(k: &mut Win32k, p: Point) {
    k.inject_mouse_move(p.x, p.y);
    k.inject_mouse_button(0, true);
    k.inject_mouse_button(0, false);
    pump(k);
}

#[test]
fn button_click_sends_wm_command_to_parent() {
    let mut k = Win32k::new(800, 600);
    let h = app(&mut k);
    let b = k.create_window_ex(0, "Button", "Go", WS_CHILD | WS_VISIBLE, 10, 10, 80, 26, h, 42, 0).unwrap();
    pump(&mut k);
    let o = k.client_to_screen(b, Point { x: 10, y: 10 }).unwrap();
    click(&mut k, o);
    #[allow(static_mut_refs)]
    let cmds = unsafe { CMDS.clone() };
    assert_eq!(cmds, vec![42]);
    // release outside the button does not click
    k.inject_mouse_move(o.x, o.y);
    k.inject_mouse_button(0, true);
    k.inject_mouse_move(o.x + 300, o.y + 100);
    k.inject_mouse_button(0, false);
    pump(&mut k);
    #[allow(static_mut_refs)]
    let n = unsafe { CMDS.len() };
    assert_eq!(n, 1);
    // button face painted (not the white window bg)
    let (px, w, _) = k.window_pixels(b).unwrap();
    assert_eq!(px[(10 * w + 40) as usize] & 0xFFFFFF, 0xD4D0C8 & 0xFFFFFF | 0, "face colour somewhere inside");
}

#[test]
fn menu_bar_dropdown_runs_command() {
    let mut k = Win32k::new(800, 600);
    let h = app(&mut k);
    let before = k.get_client_rect(h).unwrap();
    k.set_menu(h, vec![MenuItem::popup("File", vec![MenuItem::item(100, "Open"), MenuItem::item(101, "Exit")]), MenuItem::item(200, "About")]).unwrap();
    let after = k.get_client_rect(h).unwrap();
    assert_eq!(before.height() - after.height(), 22, "menu bar steals client height");
    pump(&mut k);
    let items = k.menu_bar_items(h);
    let c = |r: Rect| Point { x: r.left + 5, y: r.top + 5 };
    click(&mut k, c(items[0]));
    let rows = k.dropdown_rows();
    assert_eq!(rows.len(), 2);
    let mut fb = vec![0u32; 800 * 600];
    k.compose(&mut fb);
    assert_eq!(fb[(rows[0].top + 2) as usize * 800 + rows[0].right as usize - 3], 0xFFD4D0C8, "dropdown drawn");
    click(&mut k, c(rows[1]));
    assert!(k.dropdown_rows().is_empty());
    click(&mut k, c(items[1])); // leaf item
    #[allow(static_mut_refs)]
    let cmds = unsafe { CMDS.clone() };
    assert!(cmds.ends_with(&[101, 200]), "{cmds:?}");
    // clicking elsewhere closes an open menu
    click(&mut k, c(items[0]));
    assert!(!k.dropdown_rows().is_empty());
    click(&mut k, Point { x: 700, y: 500 });
    assert!(k.dropdown_rows().is_empty());
}

#[test]
fn message_box_modal_result() {
    let mut k = Win32k::new(800, 600);
    let h = app(&mut k);
    let dlg = k.message_box(h, "Save changes?", "Confirm", MB_YESNO).unwrap();
    assert!(!k.is_window_enabled(h));
    assert_eq!(k.get_active_window(), dlg);
    pump(&mut k);
    assert_eq!(k.message_box_result(dlg, h), None);
    let no = k.children(dlg).into_iter().find(|&c| k.get_id(c).unwrap() == IDNO as isize).unwrap();
    let o = k.client_to_screen(no, Point { x: 8, y: 8 }).unwrap();
    click(&mut k, o);
    assert!(!k.is_window(dlg));
    assert_eq!(k.message_box_result(dlg, h), Some(IDNO));
    assert!(k.is_window_enabled(h));
    // close button => IDCANCEL
    let d2 = k.message_box(h, "x", "y", MB_OK).unwrap();
    let cb = k.close_button(d2);
    click(&mut k, Point { x: cb.left + 4, y: cb.top + 4 });
    assert_eq!(k.message_box_result(d2, h), Some(IDCANCEL));
}
