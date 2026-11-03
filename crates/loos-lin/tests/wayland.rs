use loos_lin::wayland::{Error, Server};

fn req(object: u32, opcode: u16, mut body: Vec<u8>) -> Vec<u8> {
    let size = (body.len() + 8) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&object.to_ne_bytes());
    out.extend_from_slice(&((size << 16) | opcode as u32).to_ne_bytes());
    out.append(&mut body);
    out
}

fn u32v(v: u32) -> Vec<u8> {
    v.to_ne_bytes().to_vec()
}

fn string(s: &str) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&((s.len() + 1) as u32).to_ne_bytes());
    b.extend_from_slice(s.as_bytes());
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

#[test]
fn registry_advertises_compositor_shm_and_xdg_shell() {
    let mut server = Server::new();
    server.receive(&req(1, 1, u32v(2))).unwrap();
    let events = server.take_events();
    assert!(events.windows(b"wl_compositor\0".len()).any(|w| w == b"wl_compositor\0"));
    assert!(events.windows(b"wl_shm\0".len()).any(|w| w == b"wl_shm\0"));
    assert!(events.windows(b"xdg_wm_base\0".len()).any(|w| w == b"xdg_wm_base\0"));
}

#[test]
fn xdg_surface_lifecycle_accepts_realistic_request_sequence() {
    let mut server = Server::new();
    server.receive(&req(1, 1, u32v(2))).unwrap();
    server.take_events();

    let mut bind = u32v(3);
    bind.extend(string("wl_compositor"));
    bind.extend(u32v(4));
    bind.extend(u32v(10));
    server.receive(&req(2, 0, bind)).unwrap();

    let mut bind_xdg = u32v(3);
    bind_xdg.extend(string("xdg_wm_base"));
    bind_xdg.extend(u32v(6));
    bind_xdg.extend(u32v(11));
    server.receive(&req(2, 0, bind_xdg)).unwrap();

    server.receive(&req(10, 0, u32v(12))).unwrap();
    server.receive(&req(11, 2, u32v(13))).unwrap();
    let events = server.take_events();
    assert!(events.len() >= 12, "xdg_surface.configure was not emitted");
    server.receive(&req(12, 1, u32v(14))).unwrap();
    server.receive(&req(13, 4, u32v(1))).unwrap();
}

#[test]
fn shm_fd_import_and_surface_commit_produce_argb_frame() {
    let mut server = Server::new();
    server.receive(&req(1, 1, u32v(2))).unwrap();
    server.take_events();

    let mut bind = u32v(1);
    bind.extend(string("wl_shm"));
    bind.extend(u32v(1));
    bind.extend(u32v(10));
    server.receive(&req(2, 0, bind)).unwrap();

    let mut pool = u32v(20);
    pool.extend(u32v(8));
    server.receive_with_fds(&req(10, 0, pool), &[loos_lin::wayland::SharedFd {
        data: vec![0x33, 0x22, 0x11, 0xff, 0x66, 0x55, 0x44, 0xff],
    }]).unwrap();

    let mut buffer = u32v(21);
    buffer.extend(u32v(0));
    buffer.extend(u32v(2));
    buffer.extend(u32v(1));
    buffer.extend(u32v(8));
    buffer.extend(u32v(1));
    server.receive(&req(20, 0, buffer)).unwrap();

    let mut bind_comp = u32v(1);
    bind_comp.extend(string("wl_compositor"));
    bind_comp.extend(u32v(4));
    bind_comp.extend(u32v(30));
    server.receive(&req(2, 0, bind_comp)).unwrap();
    server.receive(&req(30, 0, u32v(31))).unwrap();
    server.receive(&req(31, 1, {
        let mut b = u32v(21);
        b.extend(u32v(0));
        b.extend(u32v(0));
        b
    })).unwrap();
    server.receive(&req(31, 6, Vec::new())).unwrap();
    let frames = server.take_frames();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pixels, vec![0xff112233, 0xff445566]);
}

#[test]
fn malformed_wire_is_rejected_without_partial_event() {
    let mut server = Server::new();
    assert_eq!(server.receive(&[1, 0, 0, 0, 7, 0, 0, 0]), Err(Error::Malformed));
    assert!(server.take_events().is_empty());
}
