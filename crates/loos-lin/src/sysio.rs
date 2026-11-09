//! I/O multiplexing and counters for the Linux layer: real readiness for `poll`/`ppoll`/`select`/`pselect6`/`epoll_*`,
//! `eventfd`, and the small credential calls glibc startup makes. Waiting uses [`Wait::Poll`]: a blocked poller is re-run
//! after every syscall and network poll, with its original deadline.

use super::*;

fn ok(v: i64) -> Result<Sys, i64> {
    Ok(Sys::Ret(v))
}

const POLLIN: u16 = 1;
const POLLOUT: u16 = 4;
const POLLERR: u16 = 8;
const POLLHUP: u16 = 0x10;
const POLLNVAL: u16 = 0x20;

const EPOLLIN: u32 = 1;
const EPOLLOUT: u32 = 4;
const EPOLLERR: u32 = 8;
const EPOLLHUP: u32 = 0x10;
const EPOLLRDHUP: u32 = 0x2000;
const EPOLLONESHOT: u32 = 1 << 30;

/// What a file descriptor can do right now.
#[derive(Clone, Copy, Default)]
struct Ready {
    read: bool,
    write: bool,
    hup: bool,
    err: bool,
}

impl<F: LinFs> LinSystem<F> {
    fn sock_ready(&mut self, i: usize) -> Ready {
        let (tcp, udp, listening, peer, rx_empty, peer_closed) = {
            let s = &self.socks[i];
            (s.tcp, s.udp, s.listening, s.peer, s.rx.is_empty(), s.peer_closed)
        };
        if let Some(h) = tcp {
            let Some(net) = self.stack_mut() else { return Ready { hup: true, ..Ready::default() } };
            let st = net.tcp_state(h);
            let failed = net.tcp_failure(h).is_some();
            let eof = net.tcp_eof(h);
            let open = matches!(st, Some(State::Established | State::CloseWait));
            return Ready { read: net.tcp_pending(h) || eof || failed, write: open || failed, hup: eof || st == Some(State::Closed), err: failed };
        }
        if let Some(h) = udp {
            let pending = self.stack_mut().is_some_and(|n| n.udp_pending(h));
            return Ready { read: pending, write: true, ..Ready::default() };
        }
        if listening {
            return Ready { read: !self.socks[i].backlog.is_empty(), ..Ready::default() };
        }
        match peer {
            None => Ready { write: true, hup: true, ..Ready::default() },
            Some(pe) => Ready {
                read: !rx_empty || peer_closed,
                write: !peer_closed && PIPE_CAP > self.socks[pe].rx.len(),
                hup: peer_closed && rx_empty,
                err: false,
            },
        }
    }

    fn readiness(&mut self, f: &FileRef) -> Ready {
        enum K {
            Plain,
            Tty,
            PipeR(usize),
            PipeW(usize),
            Event(usize),
            Sock(usize),
        }
        let k = match f.borrow().kind {
            FKind::Tty => K::Tty,
            FKind::PipeR(i) => K::PipeR(i),
            FKind::PipeW(i) => K::PipeW(i),
            FKind::EventFd(i) => K::Event(i),
            FKind::Sock(i) => K::Sock(i),
            _ => K::Plain,
        };
        match k {
            K::Plain => Ready { read: true, write: true, ..Ready::default() },
            K::Tty => Ready { read: !self.console_in.is_empty() || self.stdin_eof, write: true, ..Ready::default() },
            K::PipeR(i) => {
                let p = &self.pipes[i];
                Ready { read: !p.buf.is_empty() || p.writers == 0, hup: p.writers == 0, ..Ready::default() }
            }
            K::PipeW(i) => {
                let p = &self.pipes[i];
                Ready { write: p.readers > 0 && p.buf.len() < PIPE_CAP, err: p.readers == 0, ..Ready::default() }
            }
            K::Event(i) => Ready { read: self.eventfds[i].0 > 0, write: self.eventfds[i].0 < u64::MAX - 1, ..Ready::default() },
            K::Sock(i) => self.sock_ready(i),
        }
    }

    fn poll_revents(&mut self, p: &Process, fd: i32, want: u16) -> u16 {
        if fd < 0 {
            return 0;
        }
        let Some(ent) = p.fds.get(&fd) else { return POLLNVAL };
        let f = ent.file.clone();
        let r = self.readiness(&f);
        let mut rev = 0;
        if r.read && want & POLLIN != 0 {
            rev |= POLLIN;
        }
        if r.write && want & POLLOUT != 0 {
            rev |= POLLOUT;
        }
        if r.err {
            rev |= POLLERR;
        }
        if r.hup {
            rev |= POLLHUP;
        }
        rev
    }

    /// Absolute deadline for a wait, reusing the one a wake-up carried over. `rel_ns` of `None` waits forever.
    fn wait_deadline(&mut self, tid: Pid, rel_ns: Option<u64>) -> Option<u64> {
        let resumed = self.threads.get_mut(&tid).and_then(|t| t.resume_deadline.take());
        resumed.or_else(|| rel_ns.map(|r| self.now_ns + r))
    }

    fn read_timespec_ns(mem: &mut UserMem, ptr: u64, usec: bool) -> Result<Option<u64>, i64> {
        if ptr == 0 {
            return Ok(None);
        }
        let (s, n) = (mem.read_u64(ptr)?, mem.read_u64(ptr + 8)?);
        Ok(Some(s.saturating_mul(1_000_000_000).saturating_add(if usec { n.saturating_mul(1000) } else { n })))
    }

    pub(super) fn sys_io(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        match nr {
            // poll(fds, nfds, timeout_ms) / ppoll(fds, nfds, timespec*, sigmask, sz)
            7 | 271 => {
                let n = a[1].min(4096);
                let rel = if nr == 7 {
                    let ms = a[2] as i32;
                    (ms >= 0).then(|| ms as u64 * 1_000_000)
                } else {
                    Self::read_timespec_ns(&mut p.mem, a[2], false)?
                };
                let mut ready = 0;
                let mut revs = Vec::new();
                for i in 0..n {
                    let base = a[0] + i * 8;
                    let fd = p.mem.read_u32(base)? as i32;
                    let mut e = [0u8; 2];
                    p.mem.read(base + 4, &mut e)?;
                    let rev = self.poll_revents(p, fd, u16::from_le_bytes(e));
                    ready += i64::from(rev != 0);
                    revs.push(rev);
                }
                let deadline = self.wait_deadline(tid, rel);
                if ready == 0 && rel != Some(0) && !deadline.is_some_and(|d| d <= self.now_ns) {
                    return Ok(self.block(tid, Wait::Poll, nr, a, deadline, true));
                }
                for (i, rev) in revs.iter().enumerate() {
                    p.mem.write(a[0] + i as u64 * 8 + 6, &rev.to_le_bytes())?;
                }
                ok(ready)
            }
            // select(n, r, w, x, timeval*) / pselect6(n, r, w, x, timespec*, sig)
            23 | 270 => {
                let nfds = (a[0] as usize).min(1024);
                let bytes = nfds.div_ceil(64) * 8;
                let mut sets: [Option<Vec<u8>>; 3] = [None, None, None];
                for (k, s) in sets.iter_mut().enumerate() {
                    if a[1 + k] != 0 {
                        *s = Some(p.mem.read_vec(a[1 + k], bytes)?);
                    }
                }
                let rel = Self::read_timespec_ns(&mut p.mem, a[4], nr == 23)?;
                let mut out: [Vec<u8>; 3] = [vec![0; bytes], vec![0; bytes], vec![0; bytes]];
                let mut count = 0i64;
                for fd in 0..nfds {
                    let (byte, bit) = (fd / 8, 1u8 << (fd % 8));
                    let want = |k: usize| sets[k].as_ref().is_some_and(|s| s[byte] & bit != 0);
                    if !(want(0) || want(1) || want(2)) {
                        continue;
                    }
                    let Some(ent) = p.fds.get(&(fd as i32)) else { return err(EBADF) };
                    let f = ent.file.clone();
                    let r = self.readiness(&f);
                    for (k, hit) in [(0, r.read || r.hup), (1, r.write), (2, false)] {
                        if want(k) && hit {
                            out[k][byte] |= bit;
                            count += 1;
                        }
                    }
                }
                let deadline = self.wait_deadline(tid, rel);
                if count == 0 && rel != Some(0) && !deadline.is_some_and(|d| d <= self.now_ns) {
                    return Ok(self.block(tid, Wait::Poll, nr, a, deadline, true));
                }
                for k in 0..3 {
                    if sets[k].is_some() {
                        p.mem.write(a[1 + k], &out[k])?;
                    }
                }
                ok(count)
            }
            // epoll_create / epoll_create1
            213 | 291 => {
                self.epolls.push(Vec::new());
                let i = self.epolls.len() - 1;
                let f = self.new_fd(FKind::Epoll(i), 2);
                let fd = self.alloc_fd(p, 0)?;
                p.fds.insert(fd, FdEnt { file: f, cloexec: nr == 291 && a[0] & 0o2000000 != 0 });
                ok(fd as i64)
            }
            // epoll_ctl(epfd, op, fd, event*)
            233 => {
                let ep = match self.fd(p, a[0] as i32)?.borrow().kind {
                    FKind::Epoll(i) => i,
                    _ => return err(EINVAL),
                };
                let fd = a[2] as i32;
                if !p.fds.contains_key(&fd) {
                    return err(EBADF);
                }
                let (events, data) = if a[1] == 2 { (0, 0) } else { (p.mem.read_u32(a[3])?, p.mem.read_u64(a[3] + 4)?) };
                let list = &mut self.epolls[ep];
                let pos = list.iter().position(|e| e.0 == fd);
                match (a[1], pos) {
                    (1, Some(_)) => err(EEXIST),
                    (1, None) => {
                        list.push((fd, events, data));
                        ok(0)
                    }
                    (2, Some(i)) => {
                        list.remove(i);
                        ok(0)
                    }
                    (3, Some(i)) => {
                        list[i] = (fd, events, data);
                        ok(0)
                    }
                    _ => err(ENOENT),
                }
            }
            // epoll_wait / epoll_pwait(epfd, events*, max, timeout_ms)
            232 | 281 => {
                let ep = match self.fd(p, a[0] as i32)?.borrow().kind {
                    FKind::Epoll(i) => i,
                    _ => return err(EINVAL),
                };
                let max = (a[2] as i32).clamp(1, 1024) as usize;
                let ms = a[3] as i32;
                let rel = (ms >= 0).then(|| ms as u64 * 1_000_000);
                let entries = self.epolls[ep].clone();
                let mut hits: Vec<(usize, u32, u64)> = Vec::new();
                for (idx, (fd, want, data)) in entries.iter().enumerate() {
                    if hits.len() >= max || *want == 0 {
                        continue;
                    }
                    let Some(ent) = p.fds.get(fd) else { continue };
                    let f = ent.file.clone();
                    let r = self.readiness(&f);
                    let mut ev = 0;
                    if r.read && want & EPOLLIN != 0 {
                        ev |= EPOLLIN;
                    }
                    if r.write && want & EPOLLOUT != 0 {
                        ev |= EPOLLOUT;
                    }
                    if r.err {
                        ev |= EPOLLERR;
                    }
                    if r.hup {
                        ev |= EPOLLHUP | if want & EPOLLRDHUP != 0 { EPOLLRDHUP } else { 0 };
                    }
                    if ev != 0 {
                        hits.push((idx, ev, *data));
                    }
                }
                let deadline = self.wait_deadline(tid, rel);
                if hits.is_empty() && rel != Some(0) && !deadline.is_some_and(|d| d <= self.now_ns) {
                    return Ok(self.block(tid, Wait::Poll, nr, a, deadline, true));
                }
                for (k, (idx, ev, data)) in hits.iter().enumerate() {
                    let at = a[1] + k as u64 * 12; // struct epoll_event is packed on x86-64
                    p.mem.write(at, &ev.to_le_bytes())?;
                    p.mem.write(at + 4, &data.to_le_bytes())?;
                    if entries[*idx].1 & EPOLLONESHOT != 0 {
                        self.epolls[ep][*idx].1 = 0;
                    }
                }
                ok(hits.len() as i64)
            }
            // eventfd / eventfd2(initval, flags)
            284 | 290 => {
                let flags = if nr == 290 { a[1] } else { 0 };
                self.eventfds.push((a[0] & 0xffff_ffff, flags & 1 != 0));
                let i = self.eventfds.len() - 1;
                let f = self.new_fd(FKind::EventFd(i), if flags & 0o4000 != 0 { 0o4002 } else { 2 });
                let fd = self.alloc_fd(p, 0)?;
                p.fds.insert(fd, FdEnt { file: f, cloexec: flags & 0o2000000 != 0 });
                ok(fd as i64)
            }
            // getresuid / getresgid: everything is root (uid 0)
            118 | 120 => {
                for k in 0..3 {
                    p.mem.write_u32(a[k], 0)?;
                }
                ok(0)
            }
            // sendmmsg(fd, msgvec*, vlen, flags): each mmsghdr is a 56-byte msghdr plus a u32 length
            307 => {
                let mut sent = 0i64;
                for i in 0..a[2].min(64) {
                    let m = a[1] + i * 64;
                    match self.sys_socket_msg(p, tid, 46, [a[0], m, 0, a[3], 0, 0]) {
                        Ok(Sys::Ret(n)) if n >= 0 => {
                            p.mem.write_u32(m + 56, n as u32)?;
                            sent += 1;
                        }
                        Ok(r) if sent == 0 => return Ok(r),
                        Err(e) if sent == 0 => return Err(e),
                        _ => break,
                    }
                }
                ok(sent)
            }
            _ => err(ENOSYS),
        }
    }

    fn sys_socket_msg(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        self.msg_socket(p, tid, nr, a)
    }
}
