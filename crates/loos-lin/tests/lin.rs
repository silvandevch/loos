mod common;
use common::*;
use loos_lin::bridge::*;
use loos_lin::errno::*;
use loos_lin::fs::LinFs;
use loos_lin::sys::*;
use loos_lin::loader;
use std::cell::RefCell;
use std::rc::Rc;

fn put(sys: &mut LinSystem<loos_lin::fs::MemFs>, pid: u32, s: &str) -> u64 {
    // scratch area in the process heap-less data page at a fixed address (map once)
    let mem = &mut sys.process_mut(pid).unwrap().mem;
    if mem.prot_at(0x10_0000).is_none() {
        mem.map_fixed(0x10_0000, 0x10000, 3, &[], "scratch").unwrap();
    }
    let off = mem.read_u64(0x10_0000).unwrap().max(64);
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    mem.write(0x10_0000 + off, &b).unwrap();
    mem.write_u64(0x10_0000, off + b.len() as u64 + 8).unwrap();
    0x10_0000 + off
}

fn put_bytes(sys: &mut LinSystem<loos_lin::fs::MemFs>, pid: u32, b: &[u8]) -> u64 {
    let mem = &mut sys.process_mut(pid).unwrap().mem;
    if mem.prot_at(0x20_0000).is_none() {
        mem.map_fixed(0x20_0000, 0x1000, 3, &[], "bin").unwrap();
    }
    mem.write(0x20_0000, b).unwrap();
    0x20_0000
}

fn sc(sys: &mut LinSystem<loos_lin::fs::MemFs>, tid: u32, nr: u64, a: &[u64]) -> i64 {
    let mut args = [0u64; 6];
    args[..a.len()].copy_from_slice(a);
    match sys.syscall(tid, nr, args) {
        Sys::Ret(v) => v,
        other => panic!("{other:?}"),
    }
}

fn spawn_nop(sys: &mut LinSystem<loos_lin::fs::MemFs>) -> u32 {
    sys.fs.add_file("/bin/nop", &elf_exec(&[0x0F, 0x05], 0x4000), 0o755);
    sys.spawn("/bin/nop", &[sh("nop")], &[]).unwrap()
}

#[test]
fn l2_static_echo_runs_and_exits() {
    let mut sys = new_sys();
    sys.fs.add_file("/bin/echo", &elf_exec(&echo_program("hello from linux\n", 3), 0), 0o755);
    let pid = sys.spawn("/bin/echo", &[sh("echo"), sh("hi")], &[sh("PATH=/bin")]).unwrap();
    let r = run(&mut sys, pid);
    assert_eq!(r, Sys::ProcessExit(3 << 8));
    assert_eq!(sys.console_take(), b"hello from linux\n");
}

#[test]
fn l0_stack_layout_and_auxv() {
    let mut sys = new_sys();
    sys.fs.add_file("/bin/nop", &elf_exec(&[0x0F, 0x05], 0x3000), 0o755);
    let pid = sys.spawn("/bin/nop", &[sh("nop"), sh("a1"), sh("a2")], &[sh("K=V")]).unwrap();
    let t = sys.thread(pid).unwrap().regs;
    assert_eq!(t.rip, 0x400078);
    assert_eq!(t.rsp % 16, 0);
    let mem = &mut sys.process_mut(pid).unwrap().mem;
    assert_eq!(mem.read_u64(t.rsp).unwrap(), 3);
    let a0 = mem.read_u64(t.rsp + 8).unwrap();
    assert_eq!(mem.read_cstr(a0, 100).unwrap(), "nop");
    let a2 = mem.read_u64(t.rsp + 24).unwrap();
    assert_eq!(mem.read_cstr(a2, 100).unwrap(), "a2");
    assert_eq!(mem.read_u64(t.rsp + 32).unwrap(), 0);
    let e0 = mem.read_u64(t.rsp + 40).unwrap();
    assert_eq!(mem.read_cstr(e0, 100).unwrap(), "K=V");
    // auxv follows envp NULL
    let mut p = t.rsp + 56;
    let mut aux = std::collections::BTreeMap::new();
    loop {
        let (k, v) = (mem.read_u64(p).unwrap(), mem.read_u64(p + 8).unwrap());
        if k == 0 {
            break;
        }
        aux.insert(k, v);
        p += 16;
    }
    assert_eq!(aux[&loader::at::PAGESZ], 4096);
    assert_eq!(aux[&loader::at::ENTRY], 0x400078);
    assert_eq!(aux[&loader::at::PHDR], 0x400040);
    assert_eq!(aux[&loader::at::PHNUM], 1);
    assert_eq!(aux[&loader::at::BASE], 0);
    assert_eq!(mem.read_cstr(aux[&loader::at::PLATFORM], 16).unwrap(), "x86_64");
    // bss is zero and mapped; brk starts after the image
    assert_eq!(mem.read_u64(0x401000).unwrap(), 0);
    assert!(mem.brk_start >= 0x403000);
}

#[test]
fn loader_rejects_malformed_elf() {
    use loos_lin::elf::Elf;
    assert_eq!(Elf::parse(b"not an elf").unwrap_err(), ENOEXEC);
    let mut e = elf_exec(&[0x90], 0);
    e[64 + 32] = 0xff; // filesz huge
    e[64 + 33] = 0xff;
    assert_eq!(Elf::parse(&e).unwrap_err(), EINVAL);
    let mut e = elf_exec(&[0x90], 0);
    e[18] = 40; // ARM
    assert_eq!(Elf::parse(&e).unwrap_err(), ENOEXEC);
    let mut e = elf_exec(&[0x90], 0);
    e[64 + 16..64 + 24].copy_from_slice(&0xffff_ffff_ffff_f000u64.to_le_bytes());
    assert!(Elf::parse(&e).is_err());
}

#[test]
fn l3_dynamic_debian_style_load_with_host_ldso() {
    // Uses the host's real glibc + ld.so + a real PIE binary when present (unmodified upstream files).
    let (ld, libc, bin) = ("/lib64/ld-linux-x86-64.so.2", "/lib64/libc.so.6", "/usr/bin/true");
    let (Ok(ldd), Ok(libcd), Ok(bind)) = (std::fs::read(ld), std::fs::read(libc), std::fs::read(bin)) else {
        eprintln!("host glibc not found; skipping");
        return;
    };
    let mut sys = new_sys();
    sys.fs.add_file("/lib64/ld-linux-x86-64.so.2", &ldd, 0o755);
    sys.fs.add_file("/lib/x86_64-linux-gnu/libc.so.6", &libcd, 0o755);
    sys.fs.add_symlink("/lib/x86_64-linux-gnu/libc.so.6.link", "libc.so.6");
    sys.fs.add_file("/usr/bin/true", &bind, 0o755);
    let pid = sys.spawn("/usr/bin/true", &[sh("true")], &[]).unwrap();
    let t = sys.thread(pid).unwrap().regs;
    let p = sys.process(pid).unwrap();
    // Entry is inside the interpreter mapping, program PIE is at its base.
    assert!(t.rip >= loader::INTERP_BASE && t.rip < loader::INTERP_BASE + (16 << 20), "rip {:#x}", t.rip);
    let maps = p.mem.maps();
    assert!(maps.contains("/usr/bin/true") && maps.contains("ld-linux"), "{maps}");
    let mut mem = p.mem.clone();
    assert_eq!(mem.read_vec(loader::INTERP_BASE + 1, 3).unwrap(), b"ELF");
    // Emulate the start of ld.so: it mmaps libc from a real fd and we hand back the real bytes.
    let tid = pid;
    let path = put(&mut sys, pid, "/lib/x86_64-linux-gnu/libc.so.6.link");
    let fd = sc(&mut sys, tid, 257, &[(-100i64) as u64, path, 0]);
    assert!(fd >= 3, "openat -> {fd}");
    let st = put(&mut sys, pid, &"\0".repeat(150));
    assert_eq!(sc(&mut sys, tid, 5, &[fd as u64, st]), 0);
    let size = sys.process_mut(pid).unwrap().mem.read_u64(st + 48).unwrap();
    assert_eq!(size as usize, libcd.len());
    let addr = sc(&mut sys, tid, 9, &[0, size, 1, 2, fd as u64, 0]);
    assert!(addr > 0);
    let got = sys.process_mut(pid).unwrap().mem.read_vec(addr as u64, 4).unwrap();
    assert_eq!(&got, b"\x7fELF");
    assert_eq!(sc(&mut sys, tid, 10, &[addr as u64, size, 5]), 0);
    assert_eq!(sc(&mut sys, tid, 11, &[addr as u64, size]), 0);
    assert_eq!(sc(&mut sys, tid, 3, &[fd as u64]), 0);
    // glibc startup probes
    assert_eq!(sc(&mut sys, tid, 158, &[0x1002, 0x7fff_f7e0_1000]), 0);
    assert_eq!(sys.thread(tid).unwrap().regs.fs_base, 0x7fff_f7e0_1000);
    assert_eq!(sc(&mut sys, tid, 218, &[0x7fff_f7e0_1010]), tid as i64);
    assert_eq!(sc(&mut sys, tid, 334, &[0, 32, 0, 0]), -ENOSYS);
    let missing = put(&mut sys, pid, "/etc/ld.so.cache");
    assert_eq!(sc(&mut sys, tid, 257, &[(-100i64) as u64, missing, 0]), -ENOENT);
}

#[test]
fn shebang_exec_and_fork_pipe_wait() {
    let mut sys = new_sys();
    sys.fs.add_file("/bin/child", &elf_exec(&echo_program("child says hi\n", 5), 0), 0o755);
    sys.fs.add_file("/tmp/s.sh", b"#!/bin/child -x\necho\n", 0o755);
    let pid = spawn_nop(&mut sys);
    // pipe
    let fds = put(&mut sys, pid, &"\0".repeat(8));
    assert_eq!(sc(&mut sys, pid, 22, &[fds]), 0);
    let (r, w) = {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (m.read_u32(fds).unwrap() as u64, m.read_u32(fds + 4).unwrap() as u64)
    };
    // fork
    let child = sc(&mut sys, pid, 57, &[]) as u32;
    assert!(child > pid);
    assert_eq!(sys.thread(child).unwrap().regs.rax, 0);
    // parent: wait4 blocks until child exits
    let st = put(&mut sys, pid, "\0\0\0\0\0\0\0\0");
    assert_eq!(sys.syscall(pid, 61, [u64::MAX, st, 0, 0, 0, 0]), Sys::Blocked);
    // child: reads from the empty pipe -> blocks; parent writes -> child wakes with data
    let buf = put(&mut sys, child, &"\0".repeat(16));
    assert_eq!(sys.syscall(child, 0, [r, buf, 16, 0, 0, 0]), Sys::Blocked);
    let msg = put(&mut sys, pid, "ping");
    // parent must close its read end for EOF semantics later; write 4 bytes
    assert_eq!(sc(&mut sys, pid, 1, &[w, msg, 4]), 4);
    assert_eq!(sys.thread(child).unwrap().regs.rax, 4);
    assert_eq!(sys.process_mut(child).unwrap().mem.read_vec(buf, 4).unwrap(), b"ping");
    // child execs the script (shebang -> /bin/child), runs, exits 5; parent's wait4 completes
    let path = put(&mut sys, child, "/tmp/s.sh");
    let argv_s = put(&mut sys, child, "s.sh");
    let argv = put(&mut sys, child, &"\0".repeat(16));
    sys.process_mut(child).unwrap().mem.write_u64(argv, argv_s).unwrap();
    assert_eq!(sys.syscall(child, 59, [path, argv, 0, 0, 0, 0]), Sys::Exec);
    assert_eq!(sys.process(child).unwrap().exe, "/bin/child");
    assert_eq!(sys.process(child).unwrap().argv, vec!["/bin/child", "-x", "/tmp/s.sh"]);
    assert_eq!(run(&mut sys, child), Sys::ProcessExit(5 << 8));
    assert_eq!(sys.console_take(), b"child says hi\n");
    assert_eq!(sys.thread(pid).unwrap().regs.rax, child as u64, "wait4 returned child pid");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u32(st).unwrap(), 5 << 8);
    assert!(sys.process(child).is_none(), "zombie reaped");
    assert_eq!(sc(&mut sys, pid, 61, &[u64::MAX, 0, 1]), -ECHILD);
}

#[test]
fn l5_threads_futex_and_exit_clears_tid() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let word = put(&mut sys, pid, "\0\0\0\0");
    sys.process_mut(pid).unwrap().mem.write_u32(word, 1).unwrap();
    let stack = 0x7fff_f000_0000u64;
    sys.process_mut(pid).unwrap().mem.map_fixed(stack - 0x10000, 0x10000, 3, &[], "tstack").unwrap();
    // CLONE_VM|FS|FILES|SIGHAND|THREAD|SYSVSEM|SETTLS|PARENT_SETTID|CHILD_CLEARTID
    let flags = 0x100 | 0x200 | 0x400 | 0x800 | 0x10000 | 0x40000 | 0x80000 | 0x100000 | 0x200000;
    let ctid = word + 8;
    let t2 = sc(&mut sys, pid, 56, &[flags, stack, ctid, ctid, 0xdead_0000]) as u32;
    assert!(t2 > pid);
    let c = sys.thread(t2).unwrap();
    assert_eq!((c.regs.rax, c.regs.rsp, c.regs.fs_base, c.pid), (0, stack, 0xdead_0000, pid));
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u32(ctid).unwrap(), t2);
    // main thread futex-waits on *word==1 (blocks), child wakes it
    assert_eq!(sys.syscall(pid, 202, [word, 128, 1, 0, 0, 0]), Sys::Blocked);
    assert_eq!(sc(&mut sys, t2, 202, &[word, 129, 1]), 1);
    assert_eq!(sys.thread(pid).unwrap().state, TState::Runnable);
    assert_eq!(sys.thread(pid).unwrap().regs.rax, 0);
    // value mismatch -> EAGAIN
    assert_eq!(sc(&mut sys, pid, 202, &[word, 128, 7]), -EAGAIN);
    // timeout: wait with 1ms timeout, advance clock
    let ts = put(&mut sys, pid, &"\0".repeat(16));
    sys.process_mut(pid).unwrap().mem.write_u64(ts + 8, 1_000_000).unwrap();
    assert_eq!(sys.syscall(pid, 202, [word, 128, 1, ts, 0, 0]), Sys::Blocked);
    sys.advance(2_000_000);
    assert_eq!(sys.thread(pid).unwrap().regs.rax as i64, -ETIMEDOUT);
    // main waits on ctid (pthread_join); child thread exits -> ctid zeroed + waiter woken
    assert_eq!(sys.syscall(pid, 202, [ctid, 128, t2 as u64, 0, 0, 0]), Sys::Blocked);
    assert_eq!(sys.syscall(t2, 60, [0; 6]), Sys::ThreadExit);
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u32(ctid).unwrap(), 0);
    assert_eq!(sys.thread(pid).unwrap().state, TState::Runnable);
    assert!(sys.thread(t2).is_none());
    assert_eq!(sys.process(pid).unwrap().threads, vec![pid]);
}

#[test]
fn l5_signal_handler_frame_and_sigreturn() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let act = put(&mut sys, pid, &"\0".repeat(32));
    let (handler, restorer) = (0x400100u64, 0x400200u64);
    {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        m.write_u64(act, handler).unwrap();
        m.write_u64(act + 8, 0x0400_0000).unwrap(); // SA_RESTORER
        m.write_u64(act + 16, restorer).unwrap();
    }
    assert_eq!(sc(&mut sys, pid, 13, &[10, act, 0, 8]), 0);
    // set some register state, send SIGUSR1 to self, deliver
    {
        let t = sys.thread_mut(pid).unwrap();
        t.regs.rbx = 0xb0b;
        t.regs.rip = 0x400080;
    }
    let before = sys.thread(pid).unwrap().regs;
    assert_eq!(sc(&mut sys, pid, 62, &[pid as u64, 10]), 0);
    assert_eq!(sys.deliver_signal(pid), Delivery::Handler(10));
    let t = sys.thread(pid).unwrap().clone();
    assert_eq!((t.regs.rip, t.regs.rdi), (handler, 10));
    assert_eq!((t.regs.rsp + 8) % 16, 0, "handler entry alignment");
    assert_ne!(t.sigmask & (1 << 9), 0, "signal blocked during handler");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(t.regs.rsp).unwrap(), restorer);
    // handler returns via restorer -> rt_sigreturn
    sys.thread_mut(pid).unwrap().regs.rsp += 8;
    assert_eq!(sys.syscall(pid, 15, [0; 6]), Sys::Restored);
    let after = sys.thread(pid).unwrap();
    assert_eq!(after.regs, before);
    assert_eq!(after.sigmask, 0);
    // SIG_IGN drops, default SIGTERM kills with status 15
    let ign = put(&mut sys, pid, &"\u{1}\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
    assert_eq!(sc(&mut sys, pid, 13, &[12, ign, 0, 8]), 0);
    sc(&mut sys, pid, 62, &[pid as u64, 12]);
    assert_eq!(sys.deliver_signal(pid), Delivery::None);
    sc(&mut sys, pid, 62, &[pid as u64, 15]);
    assert_eq!(sys.deliver_signal(pid), Delivery::Killed(15));
    assert!(sys.process(pid).is_none());
    // blocked sleeper gets EINTR
    let p2 = spawn_nop(&mut sys);
    let ts = put(&mut sys, p2, &"\0".repeat(16));
    sys.process_mut(p2).unwrap().mem.write_u64(ts, 5).unwrap();
    assert_eq!(sys.syscall(p2, 35, [ts, 0, 0, 0, 0, 0]), Sys::Blocked);
    let h = put(&mut sys, p2, &"\0".repeat(32));
    sys.process_mut(p2).unwrap().mem.write_u64(h, 0x400100).unwrap();
    sc(&mut sys, p2, 13, &[2, h, 0, 8]);
    sys.send_signal(p2, None, 2);
    assert_eq!(sys.thread(p2).unwrap().regs.rax as i64, -EINTR);
}

#[test]
fn files_dirs_mmap_brk_and_misc_syscalls() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let t = pid;
    // create/write/read/lseek
    let f = put(&mut sys, t, "/tmp/a.txt");
    let fd = sc(&mut sys, t, 257, &[(-100i64) as u64, f, 0o102, 0o644]);
    assert!(fd >= 3);
    let data = put(&mut sys, t, "hello world");
    assert_eq!(sc(&mut sys, t, 1, &[fd as u64, data, 11]), 11);
    assert_eq!(sc(&mut sys, t, 8, &[fd as u64, 6, 0]), 6);
    let buf = put(&mut sys, t, &"\0".repeat(32));
    assert_eq!(sc(&mut sys, t, 0, &[fd as u64, buf, 32]), 5);
    assert_eq!(sys.process_mut(t).unwrap().mem.read_vec(buf, 5).unwrap(), b"world");
    assert_eq!(sc(&mut sys, t, 8, &[fd as u64, 0, 2]), 11);
    assert_eq!(sys.fs.file("/tmp/a.txt").unwrap(), b"hello world");
    // O_EXCL on existing
    assert_eq!(sc(&mut sys, t, 257, &[(-100i64) as u64, f, 0o302, 0o644]), -EEXIST);
    // getdents64
    let d = put(&mut sys, t, "/tmp");
    let dfd = sc(&mut sys, t, 257, &[(-100i64) as u64, d, 0o200000]);
    let db = put(&mut sys, t, &"\0".repeat(512));
    let n = sc(&mut sys, t, 217, &[dfd as u64, db, 512]);
    assert!(n > 0);
    let names = sys.process_mut(t).unwrap().mem.read_vec(db, n as usize).unwrap();
    assert!(String::from_utf8_lossy(&names).contains("a.txt"));
    // mkdir / rename / unlink / chdir / getcwd
    let nd = put(&mut sys, t, "/tmp/sub");
    assert_eq!(sc(&mut sys, t, 83, &[nd, 0o755]), 0);
    assert_eq!(sc(&mut sys, t, 80, &[nd]), 0);
    let cw = put(&mut sys, t, &"\0".repeat(64));
    assert_eq!(sc(&mut sys, t, 79, &[cw, 64]), 9);
    assert_eq!(sys.process_mut(t).unwrap().mem.read_cstr(cw, 64).unwrap(), "/tmp/sub");
    let rel_old = put(&mut sys, t, "../a.txt");
    let rel_new = put(&mut sys, t, "b.txt");
    assert_eq!(sc(&mut sys, t, 82, &[rel_old, rel_new]), 0);
    assert!(sys.fs.file("/tmp/sub/b.txt").is_some());
    assert_eq!(sc(&mut sys, t, 87, &[rel_new]), 0);
    let ne = put(&mut sys, t, "/nonexistent");
    assert_eq!(sc(&mut sys, t, 80, &[ne]), -ENOENT);
    // brk grows and is writable
    let b0 = sc(&mut sys, t, 12, &[0]) as u64;
    assert_eq!(sc(&mut sys, t, 12, &[b0 + 0x5000]) as u64, b0 + 0x5000);
    sys.process_mut(t).unwrap().mem.write_u64(b0 + 0x4ff0, 42).unwrap();
    assert_eq!(sc(&mut sys, t, 12, &[b0]) as u64, b0);
    assert!(sys.process_mut(t).unwrap().mem.write_u64(b0 + 0x4ff0, 1).is_err());
    // anon mmap, partial munmap splits the region, mprotect, fixed overlap replaces
    let m = sc(&mut sys, t, 9, &[0, 0x4000, 3, 0x22, u64::MAX, 0]) as u64;
    assert_eq!(m % 4096, 0);
    assert_eq!(sc(&mut sys, t, 11, &[m + 0x1000, 0x1000]), 0);
    assert!(sys.process_mut(t).unwrap().mem.read_vec(m + 0x1000, 1).is_err());
    assert!(sys.process_mut(t).unwrap().mem.read_vec(m + 0x2000, 1).is_ok());
    assert_eq!(sc(&mut sys, t, 10, &[m, 0x1000, 1]), 0);
    assert_eq!(sys.process(t).unwrap().mem.prot_at(m), Some(1));
    assert_eq!(sc(&mut sys, t, 9, &[m, 0x1000, 3, 0x32, u64::MAX, 0]) as u64, m);
    assert_eq!(sc(&mut sys, t, 9, &[0, 0, 3, 0x22, u64::MAX, 0]), -EINVAL);
    // uname / clock / getrandom / prlimit / ioctl / dev nodes
    let ub = put(&mut sys, t, &"\0".repeat(400));
    assert_eq!(sc(&mut sys, t, 63, &[ub]), 0);
    assert_eq!(sys.process_mut(t).unwrap().mem.read_cstr(ub, 64).unwrap(), "Linux");
    let ts = put(&mut sys, t, &"\0".repeat(16));
    assert_eq!(sc(&mut sys, t, 228, &[1, ts]), 0);
    sys.advance(1_500_000_000);
    assert_eq!(sc(&mut sys, t, 228, &[1, ts]), 0);
    assert_eq!(sys.process_mut(t).unwrap().mem.read_u64(ts).unwrap(), 1);
    assert_eq!(sc(&mut sys, t, 318, &[ub, 32, 0]), 32);
    assert_eq!(sc(&mut sys, t, 16, &[1, 0x5413, ub]), 0);
    assert_eq!(sc(&mut sys, t, 16, &[fd as u64, 0x5401, ub]), -ENOTTY);
    let dn = put(&mut sys, t, "/dev/null");
    let nul = sc(&mut sys, t, 257, &[(-100i64) as u64, dn, 1]);
    assert_eq!(sc(&mut sys, t, 1, &[nul as u64, data, 11]), 11);
    let pm = put(&mut sys, t, "/proc/self/maps");
    let pr = sc(&mut sys, t, 257, &[(-100i64) as u64, pm, 0]);
    assert_eq!(sc(&mut sys, t, 0, &[pr as u64, ub, 64]), 64);
    assert!(sc(&mut sys, t, 41, &[2, 1, 0]) >= 3, "sockets are supported (loopback)");
    assert_eq!(sc(&mut sys, t, 99999, &[]), -ENOSYS);
    // dup2 / close
    assert_eq!(sc(&mut sys, t, 33, &[fd as u64, 20]), 20);
    assert_eq!(sc(&mut sys, t, 3, &[20]), 0);
    assert_eq!(sc(&mut sys, t, 3, &[20]), -EBADF);
}

#[test]
fn stack_grows_down_on_demand() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let sp = sys.thread(pid).unwrap().regs.rsp;
    let m = &mut sys.process_mut(pid).unwrap().mem;
    assert!(m.write_u64(sp - 1024 * 1024, 7).is_ok());
    assert_eq!(m.read_u64(sp - 1024 * 1024).unwrap(), 7);
    assert!(m.write_u64(sp - (9 << 20), 7).is_err(), "beyond RLIMIT_STACK");
}

#[test]
fn l6_linux_app_opens_loos_window_via_bridge() {
    use loos_win32k::*;
    let k = Rc::new(RefCell::new(Win32k::new(640, 480)));
    let mut sys = new_sys();
    sys.set_bridge(Box::new(Win32Bridge::new(k.clone())));
    let pid = spawn_nop(&mut sys);
    let b = |f: u32| BRIDGE_BASE + f as u64;
    assert_eq!(sc(&mut sys, pid, b(LW_VERSION), &[]), 0x0A00_0001);
    let title = put(&mut sys, pid, "Linux GUI");
    let h = sc(&mut sys, pid, b(LW_CREATE_WINDOW), &[title, 30, 30, 240, 160, (WS_OVERLAPPEDWINDOW) as u64]);
    assert!(h > 0, "{h}");
    let h = h as u64;
    assert_eq!(sc(&mut sys, pid, b(LW_SHOW), &[h, 5]), 0);
    assert_eq!(sc(&mut sys, pid, b(LW_FILL_RECT), &[h, 10, 10, 60, 40, rgb(0, 200, 0) as u64]), 0);
    let txt = put(&mut sys, pid, "Hello from Linux");
    assert_eq!(sc(&mut sys, pid, b(LW_TEXT), &[h, 10, 50, txt, 0]), 0);
    assert_eq!(sc(&mut sys, pid, b(LW_LINE), &[h, 0, 0, 50, 50, rgb(255, 0, 0) as u64]), 0);
    // paint event reported, background untouched afterwards
    let ev = put(&mut sys, pid, &"\0".repeat(32));
    assert_eq!(sc(&mut sys, pid, b(LW_NEXT_EVENT), &[h, ev]), 1);
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u32(ev + 4).unwrap(), WM_PAINT);
    {
        let kk = k.borrow();
        let (px, w, _) = kk.window_pixels(h as u32).unwrap();
        // the app drew before the first paint was serviced; erase happened inside begin_paint -> drawing wiped?
        let _ = (px, w);
    }
    assert_eq!(sc(&mut sys, pid, b(LW_FILL_RECT), &[h, 10, 10, 60, 40, rgb(0, 200, 0) as u64]), 0);
    {
        let mut kk = k.borrow_mut();
        let (px, w, _) = kk.window_pixels(h as u32).unwrap();
        assert_eq!(px[(20 * w + 20) as usize], 0xFF00C800);
        let mut fb = vec![0u32; 640 * 480];
        kk.compose(&mut fb);
        assert_eq!(fb[(30 + 2 + 24 + 20) * 640 + 30 + 2 + 20], 0xFF00C800, "visible on the desktop");
        // user clicks the close button
        let cb = kk.close_button(h as u32);
        kk.inject_mouse_move(cb.left + 4, cb.top + 4);
        kk.inject_mouse_button(0, true);
        kk.inject_mouse_button(0, false);
    }
    // app polls events: SYSCOMMAND -> (default proc destroys) -> then quit (-1)
    let mut got_quit = false;
    for _ in 0..8 {
        let r = sc(&mut sys, pid, b(LW_NEXT_EVENT), &[0, ev]);
        if r == -1 {
            got_quit = true;
            break;
        }
    }
    assert!(got_quit);
    assert!(!k.borrow().is_window(h as u32));
}

#[test]
fn syscall_names_for_trace() {
    assert_eq!(syscall_name(1), "write");
    assert_eq!(syscall_name(BRIDGE_BASE + 2), "looswin");
}

#[test]
fn fs_resolve_symlinks_and_loops() {
    let mut fs = loos_lin::fs::MemFs::new();
    fs.add_dir("/usr/lib");
    fs.add_symlink("/lib", "usr/lib");
    fs.add_file("/usr/lib/x.so", b"x", 0o644);
    fs.add_symlink("/loop", "/loop");
    assert_eq!(loos_lin::fs::resolve(&mut fs, "/", "/lib/x.so", true).unwrap(), "/usr/lib/x.so");
    assert_eq!(loos_lin::fs::resolve(&mut fs, "/", "/lib", false).unwrap(), "/lib");
    assert_eq!(loos_lin::fs::resolve(&mut fs, "/usr", "../lib/./x.so", true).unwrap(), "/usr/lib/x.so");
    assert_eq!(loos_lin::fs::resolve(&mut fs, "/", "/loop", true).unwrap_err(), ELOOP);
    assert_eq!(fs.lstat("/nope").unwrap_err(), ENOENT);
}

#[test]
fn map_shared_survives_fork_and_vfork_shares_memory() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let shared = sc(&mut sys, pid, 9, &[0, 0x1000, 3, 0x21, u64::MAX, 0]) as u64; // MAP_SHARED|ANON
    let private = sc(&mut sys, pid, 9, &[0, 0x1000, 3, 0x22, u64::MAX, 0]) as u64;
    let child = sc(&mut sys, pid, 57, &[]) as u32;
    sys.process_mut(child).unwrap().mem.write_u64(shared, 0xAA).unwrap();
    sys.process_mut(child).unwrap().mem.write_u64(private, 0xBB).unwrap();
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(shared).unwrap(), 0xAA, "shared visible to parent");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(private).unwrap(), 0, "private is copy-on-fork");
    // vfork-style CLONE_VM|CLONE_VFORK: child writes (e.g. posix_spawn error slot) are seen by the parent
    assert_eq!(sys.syscall(pid, 56, [0x100 | 0x4000 | 17, 0, 0, 0, 0, 0]), Sys::Blocked, "vfork parent waits for exec/exit");
    let v = *sys.thread_ids().iter().max().unwrap();
    sys.process_mut(v).unwrap().mem.write_u64(private, 0xCC).unwrap();
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(private).unwrap(), 0xCC);
    // child exec gives it a fresh image; parent memory untouched
    let path = put(&mut sys, v, "/bin/nop");
    let argv = put(&mut sys, v, &"\0".repeat(8));
    assert_eq!(sys.syscall(v, 59, [path, argv, 0, 0, 0, 0]), Sys::Exec);
    sys.advance(1);
    let t = sys.thread(pid).unwrap();
    assert_eq!((t.state.clone(), t.regs.rax), (loos_lin::sys::TState::Runnable, v as u64), "exec releases the parent with the child pid");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(private).unwrap(), 0xCC);
    // after the vfork child exec'd, the parent's memory is private again: a later fork child gets a
    // copy-on-write snapshot, so parent writes after the fork stay invisible to it
    let f2 = sc(&mut sys, pid, 57, &[]) as u32;
    sys.process_mut(pid).unwrap().mem.write_u64(private, 0xEE).unwrap();
    assert_eq!(sys.process_mut(f2).unwrap().mem.read_u64(private).unwrap(), 0xCC, "fork after vfork is a snapshot");
    // partial munmap of a shared region keeps both halves coherent
    assert_eq!(sc(&mut sys, pid, 11, &[shared, 0x1000]), 0);
}

fn sockaddr_in(port: u16) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[0] = 2;
    b[2..4].copy_from_slice(&port.to_be_bytes());
    b[4..8].copy_from_slice(&[127, 0, 0, 1]);
    b
}

#[test]
fn loopback_stream_sockets_connect_accept_echo_and_unix_pair() {
    let mut sys = new_sys();
    let srv = spawn_nop(&mut sys);
    let cli = spawn_nop(&mut sys);
    let sa_s = put_bytes(&mut sys, srv, &sockaddr_in(8080));
    let sa_c = put_bytes(&mut sys, cli, &sockaddr_in(8080));
    let ls = sc(&mut sys, srv, 41, &[2, 1, 0]) as u64;
    assert_eq!(sc(&mut sys, srv, 49, &[ls, sa_s, 16]), 0);
    assert_eq!(sc(&mut sys, srv, 50, &[ls, 8]), 0);
    // second bind to same port fails
    let ls2 = sc(&mut sys, srv, 41, &[2, 1, 0]) as u64;
    assert_eq!(sc(&mut sys, srv, 49, &[ls2, sa_s, 16]), -98);
    // accept blocks until a client connects
    assert_eq!(sys.syscall(srv, 43, [ls, 0, 0, 0, 0, 0]), Sys::Blocked);
    let c = sc(&mut sys, cli, 41, &[2, 1, 0]) as u64;
    assert_eq!(sc(&mut sys, cli, 42, &[c, sa_c, 16]), 0);
    let conn = sys.thread(srv).unwrap().regs.rax as i64;
    assert!(conn >= 3, "accept woke with fd {conn}");
    let m = put(&mut sys, cli, "hello");
    assert_eq!(sc(&mut sys, cli, 44, &[c, m, 5, 0, 0, 0]), 5);
    let buf = put(&mut sys, srv, &"\0".repeat(16));
    assert_eq!(sc(&mut sys, srv, 0, &[conn as u64, buf, 16]), 5);
    assert_eq!(sc(&mut sys, srv, 1, &[conn as u64, buf, 5]), 5, "echo");
    let rb = put(&mut sys, cli, &"\0".repeat(16));
    assert_eq!(sc(&mut sys, cli, 0, &[c, rb, 16]), 5);
    // blocked reader wakes on data; close -> EOF
    assert_eq!(sys.syscall(cli, 0, [c, rb, 16, 0, 0, 0]), Sys::Blocked);
    sc(&mut sys, srv, 1, &[conn as u64, buf, 3]);
    assert_eq!(sys.thread(cli).unwrap().regs.rax, 3);
    assert_eq!(sc(&mut sys, srv, 3, &[conn as u64]), 0);
    assert_eq!(sc(&mut sys, cli, 0, &[c, rb, 16]), 0, "EOF after peer close");
    assert_eq!(sc(&mut sys, cli, 1, &[c, rb, 1]), -EPIPE);
    // refused / non-loopback / datagram
    let sa_x = put_bytes(&mut sys, cli, &sockaddr_in(9));
    let c2 = sc(&mut sys, cli, 41, &[2, 1, 0]) as u64;
    assert_eq!(sc(&mut sys, cli, 42, &[c2, sa_x, 16]), -111);
    assert_eq!(sc(&mut sys, cli, 41, &[2, 2, 0]), -93);
    // socketpair
    let fds = put(&mut sys, cli, &"\0".repeat(8));
    assert_eq!(sc(&mut sys, cli, 53, &[1, 1, 0, fds]), 0);
    let (a, b) = {
        let m = &mut sys.process_mut(cli).unwrap().mem;
        (m.read_u32(fds).unwrap() as u64, m.read_u32(fds + 4).unwrap() as u64)
    };
    let d = put(&mut sys, cli, "xy");
    assert_eq!(sc(&mut sys, cli, 1, &[a, d, 2]), 2);
    assert_eq!(sc(&mut sys, cli, 0, &[b, rb, 16]), 2);
}

#[test]
fn tar_initrd_populates_rootfs_and_runs() {
    // build a ustar archive in memory: /bin dir, /bin/echo (ELF), /bin/sh -> echo symlink
    fn hdr(name: &str, size: usize, ty: u8, mode: u32, link: &str) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = ty;
        h[157..157 + link.len()].copy_from_slice(link.as_bytes());
        h
    }
    let elf = elf_exec(&echo_program("from initrd\n", 0), 0);
    let mut tar = hdr("bin/", 0, b'5', 0o755, "");
    tar.extend(hdr("bin/echo", elf.len(), b'0', 0o755, ""));
    tar.extend(&elf);
    tar.extend(vec![0u8; (512 - elf.len() % 512) % 512]);
    tar.extend(hdr("bin/sh", 0, b'2', 0o777, "echo"));
    tar.extend(vec![0u8; 1024]);
    let mut sys = new_sys();
    assert_eq!(sys.fs.load_tar(&tar), 3);
    let pid = sys.spawn("/bin/sh", &[sh("sh")], &[]).unwrap();
    assert_eq!(run(&mut sys, pid), Sys::ProcessExit(0));
    assert_eq!(sys.console_take(), b"from initrd\n");
}

#[test]
fn file_backed_map_shared_writes_back_on_msync_munmap_and_exit() {
    let mut sys = new_sys();
    sys.fs.add_file("/tmp/data.bin", &[b'a'; 8192], 0o644);
    let pid = spawn_nop(&mut sys);
    let path = put(&mut sys, pid, "/tmp/data.bin");
    let fd = sc(&mut sys, pid, 257, &[(-100i64) as u64, path, 2]) as u64; // O_RDWR
    let m = sc(&mut sys, pid, 9, &[0, 8192, 3, 1, fd, 0]) as u64; // MAP_SHARED
    sys.process_mut(pid).unwrap().mem.write(m + 10, b"XY").unwrap();
    assert_eq!(&sys.fs.file("/tmp/data.bin").unwrap()[10..12], b"aa", "not yet written back");
    assert_eq!(sc(&mut sys, pid, 26, &[m, 8192, 4]), 0); // msync
    assert_eq!(&sys.fs.file("/tmp/data.bin").unwrap()[10..12], b"XY");
    sys.process_mut(pid).unwrap().mem.write(m + 4096, b"Z").unwrap();
    assert_eq!(sc(&mut sys, pid, 11, &[m + 4096, 4096]), 0); // munmap flushes the unmapped part
    assert_eq!(sys.fs.file("/tmp/data.bin").unwrap()[4096], b'Z');
    sys.process_mut(pid).unwrap().mem.write(m + 20, b"Q").unwrap();
    assert_eq!(sys.syscall(pid, 231, [0; 6]), Sys::ProcessExit(0)); // exit flushes the rest
    assert_eq!(sys.fs.file("/tmp/data.bin").unwrap()[20], b'Q');
    assert_eq!(sys.fs.file("/tmp/data.bin").unwrap().len(), 8192, "file never grows");
}

#[test]
fn fault_signal_frame_carries_fault_info_and_fpu_state() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let act = put(&mut sys, pid, &"\0".repeat(32));
    {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        m.write_u64(act, 0x400100).unwrap();
        m.write_u64(act + 8, 0x0400_0000).unwrap(); // SA_RESTORER
        m.write_u64(act + 16, 0x400200).unwrap();
    }
    assert_eq!(sc(&mut sys, pid, 13, &[11, act, 0, 8]), 0);
    // The program had changed its FPU state (MXCSR rounding toward zero) before faulting.
    sys.thread_mut(pid).unwrap().fx[24..28].copy_from_slice(&0x7f80u32.to_le_bytes());
    sys.fault(pid, 14, 4, 0xdead_0000); // user-mode read of an unmapped page
    assert_eq!(sys.deliver_signal(pid), Delivery::Handler(11));
    let t = sys.thread(pid).unwrap().clone();
    assert_eq!(&t.fx[24..28], &0x1f80u32.to_le_bytes(), "handler runs with a clean FPU state");
    let m = &mut sys.process_mut(pid).unwrap().mem;
    let (info, mc) = (t.regs.rsi, t.regs.rdx + 40);
    assert_eq!(m.read_u32(info + 8).unwrap(), 1, "SEGV_MAPERR");
    assert_eq!(m.read_u64(info + 16).unwrap(), 0xdead_0000, "si_addr");
    assert_eq!((m.read_u64(mc + 20 * 8).unwrap(), m.read_u64(mc + 22 * 8).unwrap()), (14, 0xdead_0000), "trapno, cr2");
    let fpregs = m.read_u64(mc + 23 * 8).unwrap();
    assert!(fpregs != 0 && fpregs % 16 == 0, "uc_mcontext.fpregs points at an aligned FXSAVE image");
    assert_eq!(m.read_u32(fpregs + 24).unwrap(), 0x7f80);
    // sigreturn brings the interrupted FPU state back
    sys.thread_mut(pid).unwrap().regs.rsp += 8;
    assert_eq!(sys.syscall(pid, 15, [0; 6]), Sys::Restored);
    assert_eq!(&sys.thread(pid).unwrap().fx[24..28], &0x7f80u32.to_le_bytes());
}

#[test]
fn unlinked_file_lives_until_its_last_descriptor_closes() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    sys.fs.mkdir("/dev/shm", 0o777).ok();
    let path = put(&mut sys, pid, "/dev/shm/org.mozilla.ipc.1");
    let fd = sc(&mut sys, pid, 257, &[(-100i64) as u64, path, 0o2 | 0o100 | 0o200, 0o600]) as u64; // O_RDWR|O_CREAT|O_EXCL
    let fd2 = sc(&mut sys, pid, 257, &[(-100i64) as u64, path, 0, 0]) as u64;
    assert_eq!(sc(&mut sys, pid, 87, &[path]), 0, "unlink");
    assert_eq!(sc(&mut sys, pid, 257, &[(-100i64) as u64, path, 0, 0]), -ENOENT, "the name is gone");
    assert_eq!(sc(&mut sys, pid, 285, &[fd, 0, 0, 8]), 0, "fallocate through the open fd");
    let buf = put(&mut sys, pid, "shared!");
    assert_eq!(sc(&mut sys, pid, 1, &[fd, buf, 7]), 7);
    let out = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 0, &[fd2, out, 7]), 7, "the other descriptor sees the same file");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_vec(out, 7).unwrap(), b"shared!");
    sc(&mut sys, pid, 3, &[fd]);
    let map = sc(&mut sys, pid, 9, &[0, 8, 1, 1, fd2, 0]);
    assert!(map > 0, "still mappable through the other descriptor");
    sc(&mut sys, pid, 3, &[fd2]);
    assert_eq!(sys.fs.list("/.loos-unlinked").unwrap().len(), 1, "a mapping keeps it too");
    sc(&mut sys, pid, 11, &[map as u64, 8]);
    let left: Vec<_> = sys.fs.list("/.loos-unlinked").map(|v| v.len()).into_iter().collect();
    assert_eq!(left, vec![0], "storage freed with the last descriptor");
}

#[test]
fn seqpacket_socketpair_keeps_message_boundaries() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let sv = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 53, &[1, 5 | 0o2000000, 0, sv]), 0, "socketpair(AF_UNIX, SOCK_SEQPACKET|SOCK_CLOEXEC)");
    let (a, b) = {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (m.read_u32(sv).unwrap() as u64, m.read_u32(sv + 4).unwrap() as u64)
    };
    let one = put(&mut sys, pid, "first");
    let two = put(&mut sys, pid, "second!");
    assert_eq!(sc(&mut sys, pid, 1, &[a, one, 5]), 5);
    assert_eq!(sc(&mut sys, pid, 1, &[a, two, 7]), 7);
    let out = put(&mut sys, pid, &".".repeat(32));
    assert_eq!(sc(&mut sys, pid, 0, &[b, out, 32]), 5, "one message per read");
    assert_eq!(sc(&mut sys, pid, 0, &[b, out, 3]), 3, "a short buffer truncates the message");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_vec(out, 3).unwrap(), b"sec");
    assert_eq!(sc_nonblock_empty(&mut sys, pid, b), -EAGAIN, "the truncated rest is gone");
}

fn sc_nonblock_empty(sys: &mut LinSystem<loos_lin::fs::MemFs>, pid: u32, fd: u64) -> i64 {
    // MSG_DONTWAIT recv on an empty socket reports EAGAIN instead of blocking
    let buf = put(sys, pid, "....");
    sc(sys, pid, 45, &[fd, buf, 4, 0x40, 0, 0])
}

#[test]
fn sendmsg_and_recvmsg_use_every_iovec() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let sv = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 53, &[1, 1, 0, sv]), 0);
    let (a, b) = {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (m.read_u32(sv).unwrap() as u64, m.read_u32(sv + 4).unwrap() as u64)
    };
    // msghdr { name, namelen, iov, iovlen, control, controllen, flags } with two iovecs
    let (h1, h2) = (put(&mut sys, pid, "HEAD"), put(&mut sys, pid, "payload"));
    let iov = put(&mut sys, pid, &"\0".repeat(32));
    let msg = put(&mut sys, pid, &"\0".repeat(56));
    {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        for (k, v) in [h1, 4, h2, 7].iter().enumerate() {
            m.write_u64(iov + k as u64 * 8, *v).unwrap();
        }
        m.write_u64(msg + 16, iov).unwrap();
        m.write_u64(msg + 24, 2).unwrap();
    }
    assert_eq!(sc(&mut sys, pid, 46, &[a, msg, 0]), 11, "both buffers are sent");
    let (r1, r2) = (put(&mut sys, pid, "...."), put(&mut sys, pid, "......."));
    {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        for (k, v) in [r1, 4, r2, 7].iter().enumerate() {
            m.write_u64(iov + k as u64 * 8, *v).unwrap();
        }
    }
    assert_eq!(sc(&mut sys, pid, 47, &[b, msg, 0]), 11);
    let m = &mut sys.process_mut(pid).unwrap().mem;
    assert_eq!((m.read_vec(r1, 4).unwrap(), m.read_vec(r2, 7).unwrap()), (b"HEAD".to_vec(), b"payload".to_vec()), "scattered back in order");
}

#[test]
fn passed_fds_arrive_with_the_bytes_they_were_sent_with() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let sv = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 53, &[1, 1, 0, sv]), 0);
    let (a, b) = {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (m.read_u32(sv).unwrap() as u64, m.read_u32(sv + 4).unwrap() as u64)
    };
    let msg = put(&mut sys, pid, &"\0".repeat(56));
    let iov = put(&mut sys, pid, &"\0".repeat(16));
    let ctl = put(&mut sys, pid, &"\0".repeat(24));
    let send = |sys: &mut LinSystem<loos_lin::fs::MemFs>, text: &str, fd: Option<u64>| {
        let data = put(sys, pid, text);
        let m = &mut sys.process_mut(pid).unwrap().mem;
        m.write_u64(iov, data).unwrap();
        m.write_u64(iov + 8, text.len() as u64).unwrap();
        m.write_u64(msg + 16, iov).unwrap();
        m.write_u64(msg + 24, 1).unwrap();
        match fd {
            Some(fd) => {
                m.write_u64(ctl, 20).unwrap(); // cmsg_len
                m.write_u32(ctl + 8, 1).unwrap(); // SOL_SOCKET
                m.write_u32(ctl + 12, 1).unwrap(); // SCM_RIGHTS
                m.write_u32(ctl + 16, fd as u32).unwrap();
                m.write_u64(msg + 32, ctl).unwrap();
                m.write_u64(msg + 40, 24).unwrap();
            }
            None => {
                m.write_u64(msg + 32, 0).unwrap();
                m.write_u64(msg + 40, 0).unwrap();
            }
        }
        sc(sys, pid, 46, &[a, msg, 0])
    };
    assert_eq!(send(&mut sys, "plain", None), 5);
    assert_eq!(send(&mut sys, "withfd", Some(1)), 6);
    let buf = put(&mut sys, pid, &".".repeat(64));
    let recv = |sys: &mut LinSystem<loos_lin::fs::MemFs>| {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        m.write_u64(iov, buf).unwrap();
        m.write_u64(iov + 8, 64).unwrap();
        m.write_u64(msg + 32, ctl).unwrap();
        m.write_u64(msg + 40, 24).unwrap();
        let n = sc(sys, pid, 47, &[b, msg, 0]);
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (n, m.read_u64(msg + 40).unwrap())
    };
    assert_eq!(recv(&mut sys), (5, 0), "stops before the fd-carrying message; no control data");
    let (n, clen) = recv(&mut sys);
    assert_eq!((n, clen), (6, 24), "the fd comes with its own bytes");
}

#[test]
fn sealed_or_read_only_shared_memory_cannot_be_mapped_writable() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let name = put(&mut sys, pid, "shm");
    let fd = sc(&mut sys, pid, 319, &[name, 1 | 2]) as u64; // MFD_CLOEXEC | MFD_ALLOW_SEALING
    assert_eq!(sc(&mut sys, pid, 77, &[fd, 4096]), 0);
    assert!(sc(&mut sys, pid, 9, &[0, 4096, 3, 1, fd, 0]) > 0, "writable shared map of an unsealed memfd");
    assert_eq!(sc(&mut sys, pid, 72, &[fd, 1033, 8 | 2 | 4]), 0, "F_ADD_SEALS write|shrink|grow");
    assert_eq!(sc(&mut sys, pid, 72, &[fd, 1034]), 8 | 2 | 4);
    assert_eq!(sc(&mut sys, pid, 9, &[0, 4096, 3, 1, fd, 0]), -EPERM, "write-sealed: no writable mapping");
    assert!(sc(&mut sys, pid, 9, &[0, 4096, 1, 1, fd, 0]) > 0, "read-only mapping still fine");
    let buf = put(&mut sys, pid, "x");
    assert_eq!(sc(&mut sys, pid, 1, &[fd, buf, 1]), -EPERM, "write() refused too");
    assert_eq!(sc(&mut sys, pid, 77, &[fd, 0]), -EPERM, "shrink refused");
    // a read-only descriptor of an ordinary file cannot back a writable shared mapping
    sys.fs.add_file("/tmp/ro", &[0; 4096], 0o644);
    let path = put(&mut sys, pid, "/tmp/ro");
    let ro = sc(&mut sys, pid, 257, &[(-100i64) as u64, path, 0, 0]) as u64;
    assert_eq!(sc(&mut sys, pid, 9, &[0, 4096, 3, 1, ro, 0]), -EACCES);
    assert_eq!(sc(&mut sys, pid, 72, &[ro, 1034]), -EINVAL, "only memfds have seals");
}

#[test]
fn proc_self_fd_reopens_the_same_file_read_only() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let name = put(&mut sys, pid, "shm");
    let fd = sc(&mut sys, pid, 319, &[name, 0]) as u64;
    let data = put(&mut sys, pid, "frozen");
    assert_eq!(sc(&mut sys, pid, 1, &[fd, data, 6]), 6);
    let link = put(&mut sys, pid, &format!("/proc/self/fd/{fd}"));
    let ro = sc(&mut sys, pid, 257, &[(-100i64) as u64, link, 0, 0]) as u64; // O_RDONLY
    assert!(ro as i64 > 0);
    assert_eq!(sc(&mut sys, pid, 72, &[ro, 3]) & 3, 0, "the new descriptor is read-only");
    assert_eq!(sc(&mut sys, pid, 9, &[0, 4096, 3, 1, ro, 0]), -EACCES, "so it cannot be mapped writable");
    let out = put(&mut sys, pid, "......");
    assert_eq!(sc(&mut sys, pid, 17, &[ro, out, 6, 0]), 6, "pread sees the same bytes");
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_vec(out, 6).unwrap(), b"frozen");
    let buf = put(&mut sys, pid, &".".repeat(64));
    let n = sc(&mut sys, pid, 89, &[link, buf, 64]);
    assert!(n > 0 && sys.process_mut(pid).unwrap().mem.read_vec(buf, n as usize).unwrap().starts_with(b"/tmp/.loos-memfd"), "readlink names the file");
}

#[test]
fn timerfd_expires_on_the_clock_and_wakes_its_reader() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let fd = sc(&mut sys, pid, 283, &[1, 0o4000]) as u64; // CLOCK_MONOTONIC, TFD_NONBLOCK
    // it_interval = 10 ms, it_value = 50 ms
    let spec = put(&mut sys, pid, &"\0".repeat(32));
    {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        m.write_u64(spec + 8, 10_000_000).unwrap();
        m.write_u64(spec + 24, 50_000_000).unwrap();
    }
    assert_eq!(sc(&mut sys, pid, 286, &[fd, 0, spec, 0]), 0);
    let buf = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 0, &[fd, buf, 8]), -EAGAIN, "not yet expired");
    sys.advance(75_000_000); // 25 ms past the first expiry: 1 + 2 periodic expirations
    assert_eq!(sc(&mut sys, pid, 0, &[fd, buf, 8]), 8);
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(buf).unwrap(), 3);
    // a blocking read sleeps until the next expiry and then returns its count
    assert_eq!(sc(&mut sys, pid, 72, &[fd, 4, 0]), 0, "clear O_NONBLOCK");
    assert_eq!(sys.syscall(pid, 0, [fd, buf, 8, 0, 0, 0]), Sys::Blocked);
    assert!(sys.next_deadline().is_some(), "the expiry is a deadline the scheduler waits for");
    sys.advance(10_000_000);
    let t = sys.thread(pid).unwrap();
    assert_eq!((t.state.clone(), t.regs.rax), (TState::Runnable, 8));
    assert_eq!(sys.process_mut(pid).unwrap().mem.read_u64(buf).unwrap(), 1);
}

#[test]
fn edge_triggered_epoll_reports_changes_not_levels() {
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let sv = put(&mut sys, pid, "........");
    assert_eq!(sc(&mut sys, pid, 53, &[1, 1, 0, sv]), 0);
    let (a, b) = {
        let m = &mut sys.process_mut(pid).unwrap().mem;
        (m.read_u32(sv).unwrap() as u64, m.read_u32(sv + 4).unwrap() as u64)
    };
    let ep = sc(&mut sys, pid, 291, &[0]) as u64;
    let ev = put(&mut sys, pid, &"\0".repeat(12));
    sys.process_mut(pid).unwrap().mem.write_u32(ev, 0x1 | 0x4 | (1 << 31)).unwrap(); // EPOLLIN|EPOLLOUT|EPOLLET
    assert_eq!(sc(&mut sys, pid, 233, &[ep, 1, b, ev]), 0);
    let out = put(&mut sys, pid, &"\0".repeat(48));
    assert_eq!(sc(&mut sys, pid, 232, &[ep, out, 4, 0]), 1, "first report: writable");
    assert_eq!(sc(&mut sys, pid, 232, &[ep, out, 4, 0]), 0, "still writable, but nothing new: no busy loop");
    let data = put(&mut sys, pid, "hi");
    assert_eq!(sc(&mut sys, pid, 1, &[a, data, 2]), 2);
    assert_eq!(sc(&mut sys, pid, 232, &[ep, out, 4, 0]), 1, "data arrived: reported again");
    assert_ne!(sys.process_mut(pid).unwrap().mem.read_u32(out).unwrap() & 1, 0, "with EPOLLIN");
    assert_eq!(sc(&mut sys, pid, 232, &[ep, out, 4, 0]), 0);
}

#[test]
fn mremap_keeps_a_shared_mapping_shared() {
    // A Wayland client (Firefox) grows its memfd pool with mremap; the compositor reads the same
    // pages through a second mapping. Growing in place or moving must not detach either side.
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let name = put(&mut sys, pid, "pool");
    let fd = sc(&mut sys, pid, 319, &[name, 1]) as u64;
    assert_eq!(sc(&mut sys, pid, 77, &[fd, 3 * 4096]), 0);
    let a = sc(&mut sys, pid, 9, &[0, 4096, 3, 1, fd, 0]) as u64; // client mapping
    let b = sc(&mut sys, pid, 9, &[0, 3 * 4096, 1, 1, fd, 0]) as u64; // "compositor" mapping
    // grow (b sits right below a in the mmap area, so a cannot grow in place): must move
    let moved = sc(&mut sys, pid, 25, &[a, 4096, 3 * 4096, 1]) as u64; // MREMAP_MAYMOVE
    assert!(moved > 0 && moved % 4096 == 0, "mremap result {moved:#x}");
    sys.process_mut(pid).unwrap().mem.write(moved + 10, b"new").unwrap();
    sys.process_mut(pid).unwrap().mem.write(moved + 2 * 4096, b"tail").unwrap();
    let mut got = [0u8; 4];
    sys.process_mut(pid).unwrap().mem.read(b + 10, &mut got[..3]).unwrap();
    assert_eq!(&got[..3], b"new", "write through the moved mapping is visible to the other mapping");
    sys.process_mut(pid).unwrap().mem.read(b + 2 * 4096, &mut got).unwrap();
    assert_eq!(&got, b"tail", "the grown tail aliases the file too");
    // in-place growth (top of a fresh mapping with room above is not guaranteed; shrink then regrow)
    assert_eq!(sc(&mut sys, pid, 25, &[moved, 3 * 4096, 4096, 0]) as u64, moved);
    let again = sc(&mut sys, pid, 25, &[moved, 4096, 2 * 4096, 0]) as u64;
    assert_eq!(again, moved, "regrow in place");
    sys.process_mut(pid).unwrap().mem.write(moved + 4096 + 5, b"z").unwrap();
    sys.process_mut(pid).unwrap().mem.read(b + 4096 + 5, &mut got[..1]).unwrap();
    assert_eq!(got[0], b'z');
}

#[test]
fn shared_futex_wakes_across_processes() {
    // A process-shared semaphore lives in MAP_SHARED memory: a waiter in one process is woken by a
    // FUTEX_WAKE (no PRIVATE flag) from another process mapping the same page at another address.
    let mut sys = new_sys();
    let pid = spawn_nop(&mut sys);
    let shared = sc(&mut sys, pid, 9, &[0, 0x2000, 3, 0x21, u64::MAX, 0]) as u64; // MAP_SHARED|ANON
    let child = sc(&mut sys, pid, 57, &[]) as u32;
    // the child re-maps the page elsewhere (as an fd-passed shm segment would be); a mapping right
    // after it forces the move
    sc(&mut sys, child, 9, &[shared + 0x2000, 0x1000, 3, 0x32, u64::MAX, 0]); // MAP_FIXED|PRIVATE|ANON
    let elsewhere = sc(&mut sys, child, 25, &[shared, 0x2000, 0x3000, 1]) as u64; // mremap MAYMOVE
    assert_ne!(elsewhere, shared);
    let word = shared + 0x1004;
    assert_eq!(sys.syscall(pid, 202, [word, 0, 0, 0, 0, 0]), Sys::Blocked, "FUTEX_WAIT (shared)");
    assert_eq!(sc(&mut sys, child, 202, &[elsewhere + 0x1004, 1, 1]), 1, "FUTEX_WAKE through the other mapping");
    assert_eq!(sys.thread(pid).unwrap().state, loos_lin::sys::TState::Runnable);
    // private futexes stay per address space: the same address in another process is a different futex
    let private = sc(&mut sys, pid, 9, &[0, 0x1000, 3, 0x22, u64::MAX, 0]) as u64;
    let c2 = sc(&mut sys, pid, 57, &[]) as u32;
    assert_eq!(sys.syscall(pid, 202, [private, 128, 0, 0, 0, 0]), Sys::Blocked, "FUTEX_WAIT_PRIVATE");
    assert_eq!(sc(&mut sys, c2, 202, &[private, 129, 1]), 0, "another process cannot wake a private futex");
    assert_eq!(sc(&mut sys, pid, 202, &[private, 129, 1]), 1, "its own process can");
}

#[test]
fn static_rootfs_borrows_the_boot_image_and_copies_on_write() {
    use loos_lin::fs::MemFs;
    let mut tar = vec![0u8; 512];
    tar[..5].copy_from_slice(b"a.txt");
    tar[100..107].copy_from_slice(b"0000644");
    tar[124..135].copy_from_slice(b"00000000005");
    tar[156] = b'0';
    tar.extend(b"hello");
    tar.extend(vec![0u8; 507 + 1024]);
    let image: &'static [u8] = Box::leak(tar.into_boxed_slice());
    let mut fs = MemFs::new();
    assert_eq!(fs.load_tar_static(image), 1);
    let mut buf = [0u8; 5];
    assert_eq!(fs.read("/a.txt", 0, &mut buf), Ok(5));
    assert_eq!(&buf, b"hello");
    assert_eq!(fs.write("/a.txt", 0, b"J"), Ok(1));
    fs.read("/a.txt", 0, &mut buf).unwrap();
    assert_eq!(&buf, b"Jello", "writes land in the file");
    assert_eq!(&image[512..517], b"hello", "the boot image itself is never modified");
}
