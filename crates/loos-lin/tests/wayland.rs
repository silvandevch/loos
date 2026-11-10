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
