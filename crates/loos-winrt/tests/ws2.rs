//! Winsock surface: option storage, WSABUF I/O, WSAPoll, FIONREAD, shutdown, resolver helpers.
use std::{cell::RefCell, rc::Rc};
use loos_winrt::ws2::{lookup, NetOps, TcpState};
use loos_winrt::{Process, Ret};

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");

/// A one-connection echo-ish stack: `rx` is what the peer sent, `tx` what we sent.
#[derive(Default)]
struct St {
    rx: Vec<u8>,
    tx: Vec<u8>,
    closed: bool,
}

struct Mock(Rc<RefCell<St>>);

impl NetOps for Mock {
    fn poll(&mut self, _: u64) {}
    fn tcp_connect(&mut self, _: [u8; 4], _: u16) -> Option<usize> { Some(1) }
    fn tcp_state(&mut self, _: usize) -> TcpState { if self.0.borrow().closed { TcpState::Closed } else { TcpState::Established } }
    fn tcp_send(&mut self, _: usize, d: &[u8]) -> Option<usize> { self.0.borrow_mut().tx.extend_from_slice(d); Some(d.len()) }
    fn tcp_recv(&mut self, _: usize, b: &mut [u8]) -> usize {
        let mut st = self.0.borrow_mut();
        let n = b.len().min(st.rx.len());
        b[..n].copy_from_slice(&st.rx[..n]);
        st.rx.drain(..n);
        n
    }
    fn tcp_eof(&mut self, _: usize) -> bool { false }
    fn tcp_readable(&mut self, _: usize) -> bool { !self.0.borrow().rx.is_empty() }
    fn tcp_available(&mut self, _: usize) -> usize { self.0.borrow().rx.len() }
    fn tcp_close(&mut self, _: usize) { self.0.borrow_mut().closed = true; }
    fn tcp_listen(&mut self, _: u16) -> Option<usize> { None }
    fn tcp_accept(&mut self, _: usize) -> Option<usize> { None }
    fn udp_bind(&mut self, _: u16) -> Option<usize> { None }
    fn udp_send(&mut self, _: usize, _: [u8; 4], _: u16, _: &[u8]) -> bool { false }
    fn udp_recv(&mut self, _: usize) -> Option<([u8; 4], u16, Vec<u8>)> { None }
    fn dns_server(&self) -> [u8; 4] { [10, 0, 2, 3] }
}

struct T {
    st: Rc<RefCell<St>>,
    p: Process,
    buf: u64,
}

impl T {
    fn new() -> T {
        let mut p = Process::create(1, 1, "t.exe", "", SVC).unwrap();
        let st = Rc::new(RefCell::new(St::default()));
        p.set_net(Box::new(Mock(st.clone())));
        let buf = p.mem.map_anywhere(0x4000, 3, &[], "buf").unwrap();
        p.cur_rsp = buf + 0x3000; // stack args at +0x28...
        T { st, p, buf }
    }
    fn call(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> Ret {
        for (k, v) in stack.iter().enumerate() {
            self.p.mem.write_u64(self.p.cur_rsp + 0x28 + 8 * k as u64, *v).unwrap();
        }
        self.p.syscall(lookup(name).unwrap_or_else(|| panic!("{name} missing")), a, 1000)
    }
    fn val(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> u64 {
        match self.call(name, a, stack) {
            Ret::Value(v) => v,
            r => panic!("{name}: {r:?}"),
        }
    }
    fn sock(&mut self) -> u64 {
        self.val("socket", [2, 1, 6, 0], &[])
    }
    fn connected(&mut self) -> u64 {
        let s = self.sock();
        let sa = self.buf + 0x3800;
        self.p.mem.write(sa, &[2, 0, 0, 80, 10, 0, 2, 2]).unwrap();
        assert_eq!(self.val("connect", [s, sa, 16, 0], &[]), 0);
        s
    }
}

#[test]
fn startup_fills_wsadata_and_refcounts() {
    let mut t = T::new();
    assert_eq!(t.val("WSACleanup", [0; 4], &[]), u64::MAX, "cleanup before startup fails");
    assert_eq!(t.val("WSAGetLastError", [0; 4], &[]), 10093);
    assert_eq!(t.val("WSAStartup", [0x0101, t.buf, 0, 0], &[]), 0);
    assert_eq!(t.p.mem.read_vec(t.buf, 4).unwrap(), [1, 1, 2, 2], "negotiated 1.1, high 2.2");
    assert_eq!(&t.p.mem.read_vec(t.buf + 16, 4).unwrap(), b"LoOS");
    assert_eq!(t.val("WSAStartup", [0x0000, t.buf, 0, 0], &[]), 10092, "version 0 unsupported");
    assert_eq!(t.val("WSACleanup", [0; 4], &[]), 0);
}

#[test]
fn sockopt_roundtrip_and_defaults() {
    let mut t = T::new();
    let s = t.sock();
    let (val, len) = (t.buf + 0x100, t.buf + 0x110);
    t.p.mem.write_u32(val, 7).unwrap();
    assert_eq!(t.val("setsockopt", [s, 6, 1, val], &[4]), 0); // IPPROTO_TCP / TCP_NODELAY-like key
    t.p.mem.write_u32(val, 0).unwrap();
    assert_eq!(t.val("getsockopt", [s, 6, 1, val], &[len]), 0);
    assert_eq!(t.p.mem.read_u32(val).unwrap(), 7);
    assert_eq!(t.p.mem.read_u32(len).unwrap(), 4);
    assert_eq!(t.val("getsockopt", [s, 0xffff, 0x1008, val], &[len]), 0);
    assert_eq!(t.p.mem.read_u32(val).unwrap(), 1, "SO_TYPE = SOCK_STREAM");
    assert_eq!(t.val("getsockopt", [s, 0xffff, 0x1007, val], &[len]), 0);
    assert_eq!(t.p.mem.read_u32(val).unwrap(), 0, "SO_ERROR clear");
}

fn wsabuf(t: &mut T, at: u64, parts: &[(u64, u32)]) {
    for (k, (p, l)) in parts.iter().enumerate() {
        t.p.mem.write_u32(at + 16 * k as u64, *l).unwrap();
        t.p.mem.write_u64(at + 16 * k as u64 + 8, *p).unwrap();
    }
}

#[test]
fn wsabuf_scatter_gather_poll_and_fionread() {
    let mut t = T::new();
    let s = t.connected();
    let (a, b, bufs, sent) = (t.buf + 0x200, t.buf + 0x240, t.buf + 0x300, t.buf + 0x380);
    t.p.mem.write(a, b"hello ").unwrap();
    t.p.mem.write(b, b"world").unwrap();
    wsabuf(&mut t, bufs, &[(a, 6), (b, 5)]);
    assert_eq!(t.val("WSASend", [s, bufs, 2, sent], &[0, 0, 0]), 0);
    assert_eq!(t.p.mem.read_u32(sent).unwrap(), 11);
    assert_eq!(t.st.borrow().tx, b"hello world");

    // Nothing to read yet: WSAPoll with timeout 0 reports 0 for POLLIN, POLLOUT is ready.
    let fds = t.buf + 0x400;
    t.p.mem.write_u64(fds, s).unwrap();
    t.p.mem.write(fds + 8, &0x0300u16.to_le_bytes()).unwrap();
    assert_eq!(t.val("WSAPoll", [fds, 1, 0, 0], &[]), 0);
    t.p.mem.write(fds + 8, &0x0010u16.to_le_bytes()).unwrap();
    assert_eq!(t.val("WSAPoll", [fds, 1, 0, 0], &[]), 1);
    assert_eq!(t.p.mem.read_vec(fds + 10, 2).unwrap(), 0x0010u16.to_le_bytes());

    // Peer data arrives: FIONREAD, poll, scattered receive.
    t.st.borrow_mut().rx.extend_from_slice(b"abcdefgh");
    let n = t.buf + 0x3f0;
    assert_eq!(t.val("ioctlsocket", [s, 0x4004_667f, n, 0], &[]), 0);
    assert_eq!(t.p.mem.read_u32(n).unwrap(), 8);
    t.p.mem.write(fds + 8, &0x0300u16.to_le_bytes()).unwrap();
    assert_eq!(t.val("WSAPoll", [fds, 1, 0, 0], &[]), 1);
    let (r1, r2) = (t.buf + 0x500, t.buf + 0x540);
    wsabuf(&mut t, bufs, &[(r1, 3), (r2, 16)]);
    assert_eq!(t.val("WSARecv", [s, bufs, 2, sent], &[0, 0, 0]), 0);
    assert_eq!(t.p.mem.read_u32(sent).unwrap(), 8);
    assert_eq!(t.p.mem.read_vec(r1, 3).unwrap(), b"abc");
    assert_eq!(t.p.mem.read_vec(r2, 5).unwrap(), b"defgh");
}

#[test]
fn shutdown_blocks_further_send() {
    let mut t = T::new();
    let s = t.connected();
    let d = t.buf + 0x200;
    t.p.mem.write(d, b"x").unwrap();
    assert_eq!(t.val("send", [s, d, 1, 0], &[]), 1);
    assert_eq!(t.val("shutdown", [s, 1, 0, 0], &[]), 0);
    assert!(t.st.borrow().closed, "SD_SEND closes our side of the stack connection");
    assert_eq!(t.val("send", [s, d, 1, 0], &[]), u64::MAX);
    assert_eq!(t.val("WSAGetLastError", [0; 4], &[]), 10058, "WSAESHUTDOWN");
    assert_eq!(t.val("shutdown", [s, 9, 0, 0], &[]), u64::MAX);
}

#[test]
fn resolver_helpers() {
    let mut t = T::new();
    let (src, dst) = (t.buf + 0x100, t.buf + 0x140);
    t.p.mem.write(src, b"192.168.1.20\0").unwrap();
    assert_eq!(t.val("inet_pton", [2, src, dst, 0], &[]), 1);
    assert_eq!(t.p.mem.read_vec(dst, 4).unwrap(), [192, 168, 1, 20]);
    assert_eq!(t.val("inet_ntop", [2, dst, t.buf + 0x180, 32], &[]), t.buf + 0x180);
    assert_eq!(t.p.mem.read_cstr(t.buf + 0x180, 32).unwrap(), "192.168.1.20");
    assert_eq!(t.val("inet_ntop", [2, dst, t.buf + 0x180, 4], &[]), 0, "buffer too small");
    t.p.mem.write(src, b"300.1.1.1\0").unwrap();
    assert_eq!(t.val("inet_pton", [2, src, dst, 0], &[]), 0);

    t.p.mem.write(src, b"https\0tcp\0").unwrap();
    let se = t.val("getservbyname", [src, src + 6, 0, 0], &[]);
    assert_ne!(se, 0);
    assert_eq!(t.p.mem.read_vec(se + 16, 2).unwrap(), 443u16.to_be_bytes());
    let pr = t.val("getprotobynumber", [17, 0, 0, 0], &[]);
    let name = t.p.mem.read_u64(pr).unwrap();
    assert_eq!(t.p.mem.read_cstr(name, 8).unwrap(), "udp");
    t.p.mem.write(src, b"nosuch\0").unwrap();
    assert_eq!(t.val("getservbyname", [src, 0, 0, 0], &[]), 0);
}
