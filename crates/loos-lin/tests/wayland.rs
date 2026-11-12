use loos_lin::mem::PageBuf;
use loos_lin::wayland::{Error, Server, SharedFd};
use std::cell::RefCell;
use std::rc::Rc;

fn req(object: u32, opcode: u16, mut body: Vec<u8>) -> Vec<u8> {
    let size = (body.len() + 8) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&object.to_ne_bytes());
    out.extend_from_slice(&((size << 16) | opcode as u32).to_ne_bytes());
    out.append(&mut body);
    out
}

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

fn string(s: &str) -> Vec<u8> {
    let mut b = ((s.len() + 1) as u32).to_ne_bytes().to_vec();
    b.extend_from_slice(s.as_bytes());
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

/// Decoded events: (object, opcode, body).
fn events(server: &mut Server) -> Vec<(u32, u16, Vec<u8>)> {
    let e = server.take_events();
    let mut out = Vec::new();
    let mut off = 0;
    while off < e.len() {
        let obj = u32::from_ne_bytes(e[off..off + 4].try_into().unwrap());
        let w = u32::from_ne_bytes(e[off + 4..off + 8].try_into().unwrap());
        let size = (w >> 16) as usize;
        out.push((obj, (w & 0xffff) as u16, e[off + 8..off + size].to_vec()));
        off += size;
    }
    out
}

fn bind(server: &mut Server, iface: &str, version: u32, id: u32) {
    let name = server.globals().iter().find(|g| g.interface == iface).unwrap().name;
    let mut b = u32s(&[name]);
    b.extend(string(iface));
    b.extend(u32s(&[version, id]));
    server.receive(&req(2, 0, b)).unwrap();
}

fn connected() -> Server {
    let mut s = Server::new();
    s.receive(&req(1, 1, u32s(&[2]))).unwrap();
    s.take_events();
    s
}

#[test]
fn registry_advertises_desktop_globals() {
    let mut server = Server::new();
    server.receive(&req(1, 1, u32s(&[2]))).unwrap();
    let ev = server.take_events();
    for name in ["wl_compositor", "wl_subcompositor", "wl_shm", "wl_seat", "wl_output", "xdg_wm_base", "wl_data_device_manager"] {
        let needle = format!("{name}\0");
        assert!(ev.windows(needle.len()).any(|w| w == needle.as_bytes()), "{name} missing");
    }
}

#[test]
fn toplevel_configures_and_destroy_recycles_ids() {
    let mut s = connected();
    bind(&mut s, "wl_compositor", 5, 10);
    bind(&mut s, "xdg_wm_base", 5, 11);
    s.receive(&req(10, 0, u32s(&[12]))).unwrap(); // surface
    s.receive(&req(11, 2, u32s(&[13, 12]))).unwrap(); // xdg_surface
    s.take_events();
    s.receive(&req(13, 1, u32s(&[14]))).unwrap(); // toplevel
    assert!(!events(&mut s).iter().any(|e| e.0 == 14 && e.1 == 0), "no configure before the initial commit");
    s.receive(&req(12, 6, vec![])).unwrap(); // initial commit (no buffer)
    let ev = events(&mut s);
    let cfg = ev.iter().find(|e| e.0 == 14 && e.1 == 0).expect("xdg_toplevel.configure");
    assert_eq!(&cfg.2[..8], &u32s(&[0, 0])[..], "0x0 lets the client pick its size");
    assert!(ev.iter().any(|e| e.0 == 13 && e.1 == 0), "xdg_surface.configure");
    assert!(!ev.iter().any(|e| e.0 == 14 && e.1 == 1), "must not ask the new window to close");
    let mut title = Vec::new();
    title.extend(string("Demo"));
    s.receive(&req(14, 2, title)).unwrap();
    s.receive(&req(14, 0, vec![])).unwrap(); // destroy toplevel
    let ev = events(&mut s);
    assert!(ev.contains(&(1, 1, u32s(&[14]))), "wl_display.delete_id");
    assert_eq!(s.take_closed(), vec![12]);
    // the id is reusable after delete_id
    s.receive(&req(13, 1, u32s(&[14]))).unwrap();
}

#[test]
fn shm_pool_is_live_and_frames_crop_to_window_geometry() {
    let mut s = connected();
    bind(&mut s, "wl_shm", 1, 10);
    bind(&mut s, "wl_compositor", 5, 30);
    bind(&mut s, "xdg_wm_base", 5, 40);
    let mut pb = PageBuf::default();
    pb.write_at(0, &[0x33, 0x22, 0x11, 0xff, 0x66, 0x55, 0x44, 0xff]);
    let buf = Rc::new(RefCell::new(pb));
    s.receive_with_fds(&req(10, 0, u32s(&[20, 8])), &[SharedFd { buf: buf.clone() }]).unwrap();
    s.receive(&req(20, 0, u32s(&[21, 0, 2, 1, 8, 0]))).unwrap(); // 2x1 ARGB8888
    s.receive(&req(30, 0, u32s(&[31]))).unwrap();
    s.receive(&req(40, 2, u32s(&[41, 31]))).unwrap();
    s.receive(&req(41, 1, u32s(&[42]))).unwrap();
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    let f = s.take_frames();
    assert_eq!(f[0].pixels, vec![0xff112233, 0xff445566]);
    // The client draws again into the same mapping: the next commit shows it.
    buf.borrow_mut().write_at(0, &[0, 0, 0xff, 0xff]);
    s.receive(&req(41, 3, u32s(&[1, 0, 1, 1]))).unwrap(); // geometry: right pixel only
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    let f = s.take_frames();
    assert_eq!((f[0].width, f[0].height, f[0].pixels.clone()), (1, 1, vec![0xff445566]));
}

#[test]
fn frame_callbacks_fire_after_presentation() {
    let mut s = connected();
    bind(&mut s, "wl_compositor", 5, 10);
    s.receive(&req(10, 0, u32s(&[11]))).unwrap();
    s.take_events();
    s.receive(&req(11, 3, u32s(&[12]))).unwrap();
    s.receive(&req(11, 6, vec![])).unwrap();
    assert!(!events(&mut s).iter().any(|e| e.0 == 12), "not before presentation");
    s.frames_presented(1234);
    let ev = events(&mut s);
    assert!(ev.contains(&(12, 0, u32s(&[1234]))));
    assert!(ev.contains(&(1, 1, u32s(&[12]))));
}

#[test]
fn malformed_wire_is_rejected_without_partial_event() {
    let mut server = Server::new();
    assert_eq!(server.receive(&[1, 0, 0, 0, 7, 0, 0, 0]), Err(Error::Malformed));
    assert!(server.take_events().is_empty());
}

#[test]
fn subsurface_content_is_composited_into_its_window() {
    let mut s = connected();
    bind(&mut s, "wl_shm", 1, 10);
    bind(&mut s, "wl_compositor", 5, 30);
    bind(&mut s, "xdg_wm_base", 5, 40);
    bind(&mut s, "wl_subcompositor", 1, 50);
    // one pool: [window 2x1 fully transparent][content 1x1 opaque red]
    let mut pb = PageBuf::default();
    pb.write_at(0, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]);
    let buf = Rc::new(RefCell::new(pb));
    s.receive_with_fds(&req(10, 0, u32s(&[20, 12])), &[SharedFd { buf }]).unwrap();
    s.receive(&req(20, 0, u32s(&[21, 0, 2, 1, 8, 0]))).unwrap(); // window buffer
    s.receive(&req(20, 0, u32s(&[22, 8, 1, 1, 4, 0]))).unwrap(); // content buffer
    s.receive(&req(30, 0, u32s(&[31]))).unwrap(); // window surface
    s.receive(&req(30, 0, u32s(&[32]))).unwrap(); // content surface
    s.receive(&req(40, 2, u32s(&[41, 31]))).unwrap();
    s.receive(&req(41, 1, u32s(&[42]))).unwrap();
    s.receive(&req(50, 1, u32s(&[51, 32, 31]))).unwrap(); // get_subsurface(content -> window)
    s.receive(&req(51, 1, u32s(&[1, 0]))).unwrap(); // at x=1
    s.receive(&req(32, 1, u32s(&[22, 0, 0]))).unwrap();
    s.receive(&req(32, 6, vec![])).unwrap();
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    let f = s.take_frames();
    assert_eq!(f.len(), 1, "one frame per window");
    assert_eq!((f[0].surface, f[0].pixels.clone()), (31, vec![0, 0xffff0000]), "transparent window with the red content on top at x=1");
}

#[test]
fn shown_surfaces_enter_the_output_once() {
    let mut s = connected();
    bind(&mut s, "wl_shm", 1, 10);
    bind(&mut s, "wl_compositor", 5, 30);
    bind(&mut s, "xdg_wm_base", 5, 40);
    bind(&mut s, "wl_output", 4, 60);
    let buf = Rc::new(RefCell::new(PageBuf::default()));
    s.receive_with_fds(&req(10, 0, u32s(&[20, 4])), &[SharedFd { buf }]).unwrap();
    s.receive(&req(20, 0, u32s(&[21, 0, 1, 1, 4, 0]))).unwrap();
    s.receive(&req(30, 0, u32s(&[31]))).unwrap();
    s.receive(&req(40, 2, u32s(&[41, 31]))).unwrap();
    s.receive(&req(41, 1, u32s(&[42]))).unwrap();
    s.take_events();
    s.receive(&req(31, 6, vec![])).unwrap(); // no buffer yet: not on screen
    assert!(!events(&mut s).iter().any(|e| e.0 == 31 && e.1 == 0));
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    assert!(events(&mut s).contains(&(31, 0, u32s(&[60]))), "wl_surface.enter(output) once mapped");
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    assert!(!events(&mut s).iter().any(|e| e.0 == 31 && e.1 == 0), "only once");
    bind(&mut s, "wl_output", 4, 61); // an output bound later is entered too
    assert!(events(&mut s).contains(&(31, 0, u32s(&[61]))));
}

#[test]
fn maximize_and_fullscreen_configure_a_real_size() {
    // GTK lays out (and Firefox maps its content surface) on the size the compositor configures.
    let mut s = connected();
    s.work_area = (1000, 700);
    s.screen = (1024, 768);
    bind(&mut s, "wl_compositor", 5, 10);
    bind(&mut s, "xdg_wm_base", 5, 11);
    s.receive(&req(10, 0, u32s(&[12]))).unwrap();
    s.receive(&req(11, 2, u32s(&[13, 12]))).unwrap();
    s.receive(&req(13, 1, u32s(&[14]))).unwrap();
    s.receive(&req(12, 6, vec![])).unwrap(); // initial commit
    s.take_events();
    let cfg = |s: &mut Server| {
        let ev = events(s);
        assert!(ev.iter().any(|e| e.0 == 13 && e.1 == 0), "xdg_surface.configure follows");
        ev.into_iter().find(|e| e.0 == 14 && e.1 == 0).expect("xdg_toplevel.configure").2
    };
    s.receive(&req(14, 9, vec![])).unwrap(); // set_maximized
    assert_eq!(cfg(&mut s), u32s(&[1000, 700, 8, 1, 4]), "work area, maximized + activated");
    s.receive(&req(14, 10, vec![])).unwrap(); // unset_maximized
    assert_eq!(cfg(&mut s), u32s(&[0, 0, 4, 4]));
    s.receive(&req(14, 11, u32s(&[0]))).unwrap(); // set_fullscreen(nil)
    assert_eq!(cfg(&mut s), u32s(&[1024, 768, 8, 2, 4]));
}

#[test]
fn buffers_are_released_when_replaced_not_on_commit() {
    let mut s = connected();
    bind(&mut s, "wl_shm", 1, 10);
    bind(&mut s, "wl_compositor", 5, 30);
    let buf = Rc::new(RefCell::new(PageBuf::default()));
    s.receive_with_fds(&req(10, 0, u32s(&[20, 8])), &[SharedFd { buf }]).unwrap();
    s.receive(&req(20, 0, u32s(&[21, 0, 1, 1, 4, 0]))).unwrap(); // buffer A
    s.receive(&req(20, 0, u32s(&[22, 4, 1, 1, 4, 0]))).unwrap(); // buffer B
    s.receive(&req(30, 0, u32s(&[31]))).unwrap();
    s.take_events();
    let released = |s: &mut Server, b: u32| events(s).iter().any(|e| e.0 == b && e.1 == 0);
    s.receive(&req(31, 1, u32s(&[21, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    assert!(!released(&mut s, 21), "A is on screen: held");
    s.receive(&req(31, 6, vec![])).unwrap(); // commit without a new buffer keeps A
    assert!(!released(&mut s, 21));
    s.receive(&req(31, 1, u32s(&[22, 0, 0]))).unwrap();
    s.receive(&req(31, 6, vec![])).unwrap();
    assert!(released(&mut s, 21), "B replaced A: A released");
    s.receive(&req(31, 0, vec![])).unwrap(); // surface destroyed
    assert!(released(&mut s, 22), "B released with its surface");
}
