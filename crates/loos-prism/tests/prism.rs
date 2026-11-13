//! End-to-end tests over real Mach-O binaries built from `tests/prism/hello.c`
//! (skipped when clang / ld64.lld are not installed).

use loos_lin::fs::{LinFs, MemFs};
use loos_lin::sys::{LinSystem, Regs, TState};
use loos_prism::loader::{PRISM_NR, SHIM_BASE, STUB_SIZE};
use loos_prism::macho::{self, Fixup, MachO, CPU_ARM64, CPU_X86_64};
use loos_prism::Prism;
use std::path::PathBuf;
use std::sync::OnceLock;

fn binaries() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/prism");
        let out = std::env::temp_dir().join(format!("loos-prism-test-{}", std::process::id()));
        let ok = std::process::Command::new(root.join("build.sh")).arg(&out).output().map(|o| o.status.success()).unwrap_or(false);
        if !ok {
            eprintln!("prism tests: clang/ld64.lld unavailable, skipping binary tests");
        }
        ok.then_some(out)
    })
    .as_ref()
}

fn read(name: &str) -> Option<Vec<u8>> {
    binaries().map(|d| std::fs::read(d.join(name)).unwrap())
}

#[test]
fn parses_thin_chained_and_universal() {
    let Some(x86) = read("hello_x86_64") else { return };
    let m = MachO::parse(&x86).unwrap();
    assert_eq!(m.cpu, CPU_X86_64);
    assert_eq!(m.dylibs[0].0, "/usr/lib/libSystem.B.dylib");
    assert!(matches!(m.entry, Some(macho::Entry::Main(_))));
    let binds: Vec<&str> = m.fixups.iter().filter_map(|f| match f { Fixup::Bind { name, .. } => Some(name.as_str()), _ => None }).collect();
    assert!(binds.contains(&"_printf") && binds.contains(&"_malloc"), "{binds:?}");
    assert!(m.sections.iter().any(|s| s.name == "__mod_init_func" || s.name == "__init_offsets"));

    // Chained fixups describe the same imports.
    let ch = MachO::parse(&read("hello_chained").unwrap()).unwrap();
    let mut a: Vec<String> = ch.fixups.iter().filter_map(|f| match f { Fixup::Bind { name, .. } => Some(name.clone()), _ => None }).collect();
    let mut b: Vec<String> = binds.iter().filter(|s| **s != "dyld_stub_binder").map(|s| s.to_string()).collect();
    a.sort();
    a.dedup();
    b.sort();
    b.dedup();
    assert_eq!(a, b);
    assert!(ch.fixups.iter().any(|f| matches!(f, Fixup::Rebase { .. })), "the gp/msg pointers need rebasing");

    // Universal: preference order picks the slice.
    let fat = read("hello_universal").unwrap();
    let (cpu, r) = macho::select_arch(&fat, &[CPU_X86_64, CPU_ARM64]).unwrap();
    assert_eq!(cpu, CPU_X86_64);
    assert_eq!(&fat[r], &x86[..]);
    assert_eq!(macho::select_arch(&fat, &[CPU_ARM64]).unwrap().0, CPU_ARM64);
    assert!(macho::is_macho(&fat) && !macho::is_macho(b"\x7fELF\x02\x01\x01\0"));
}

fn system(files: &[(&str, &[u8])]) -> LinSystem<MemFs> {
    let mut fs = MemFs::new();
    for d in ["/bin", "/Applications", "/home", "/tmp", "/dev"] {
        fs.add_dir(d);
    }
    for (p, d) in files {
        fs.add_file(p, d, 0o755);
    }
    let mut sys = LinSystem::new(fs);
    sys.stdin_eof = true;
    sys
}

const EXPECTED: [&str; 7] = [
    "prism: hello from /bin/hello, argc=1, ctor=42\n",
    "heap: abcdefghijklmnopqrstuvwxyz len=26\n",
    "fmt: [ 3.14|7   |beef|str|Z] n=21\n",
    "math: fib(20)=6765 dot=5.000 div=-176\n",
    "env: HOME=/home cmp=1\n",
    "write: ok\n",
    "atexit handler ran\n",
];

#[test]
fn runs_arm64_program_on_the_interpreter() {
    let Some(bin) = read("hello_arm64") else { return };
    let mut sys = system(&[("/bin/hello", &bin)]);
    let mut prism = Prism::new();
    let argv = vec!["/bin/hello".to_string()];
    let pid = prism.spawn(&mut sys, "/bin/hello", &argv, &["HOME=/home".to_string()]).unwrap();
    assert!(prism.is_arm(pid));
    let mut rounds = 0;
    while sys.thread(pid).is_some() {
        rounds += 1;
        assert!(rounds < 10_000, "runaway; output so far: {}", String::from_utf8_lossy(&sys.console_out));
        if sys.thread(pid).unwrap().state == TState::Runnable {
            prism.run_arm(&mut sys, pid, 100_000);
            // A fatal signal is delivered the same way as for native threads.
            sys.deliver_signal(pid);
        } else {
            sys.advance(1_000_000);
        }
    }
    let out = String::from_utf8_lossy(&sys.console_out).into_owned();
    assert_eq!(out, EXPECTED.concat(), "diag: {}", sys.diag);
    assert!(sys.diag.contains("exits with status 7"), "diag: {}", sys.diag);
}

#[test]
fn loads_x86_64_program_and_enters_main_through_start_stub() {
    let Some(bin) = read("hello_universal") else { return };
    let mut sys = system(&[("/Applications/Hello.app/Contents/MacOS/Hello", &bin)]);
    let plist = b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\n<key>CFBundleExecutable</key>\n<string>Hello</string><key>CFBundleIdentifier</key><string>ch.loos.hello</string></dict></plist>";
    sys.fs.create("/Applications/Hello.app/Contents/Info.plist", 0o644).unwrap();
    sys.fs.write("/Applications/Hello.app/Contents/Info.plist", 0, plist).unwrap();
    assert!(loos_prism::recognizes(&mut sys.fs, "/Applications/Hello.app"));
    let b = loos_prism::bundle::open(&mut sys.fs, "/Applications/Hello.app").unwrap();
    assert_eq!(b.identifier.as_deref(), Some("ch.loos.hello"));

    let mut prism = Prism::new();
    let pid = prism.spawn(&mut sys, "/Applications/Hello.app", &["Hello".to_string()], &[]).unwrap();
    assert!(!prism.is_arm(pid), "the universal binary runs its Intel slice natively");
    let regs: Regs = sys.thread(pid).unwrap().regs;
    assert_eq!(regs.rip, SHIM_BASE);
    let p = sys.process_mut(pid).unwrap();
    // The stubs are real x86-64 code: mov r10, rcx; mov eax, PRISM_NR+i; syscall; ret.
    let stub0 = p.mem.read_vec(SHIM_BASE, 11).unwrap();
    assert_eq!(&stub0[..4], &[0x49, 0x89, 0xca, 0xb8]);
    assert_eq!(u32::from_le_bytes(stub0[4..8].try_into().unwrap()) as u64, PRISM_NR);
    assert_eq!(&stub0[8..], &[0x0f, 0x05, 0xc3]);

    // Execute the start stub's syscall: the constructor runs first, then main.
    let sp0 = regs.rsp;
    prism.syscall(&mut sys, pid, PRISM_NR, [0; 6]).unwrap();
    let r = sys.thread(pid).unwrap().regs;
    let p = sys.process_mut(pid).unwrap();
    let ctor = p.mem.read_u64(r.rsp).unwrap();
    let back = p.mem.read_u64(r.rsp + 8).unwrap();
    assert_eq!(back, SHIM_BASE, "the constructor returns into the start stub");
    assert_eq!((r.rsp + 8) % 16, 8, "callee sees rsp = 8 mod 16");
    assert!(ctor != 0, "the first initializer (Prism's C runtime, then the program's) is called");
    // Each further pass runs the next initializer; the last one enters main.
    loop {
        sys.thread_mut(pid).unwrap().regs.rsp = sp0;
        prism.syscall(&mut sys, pid, PRISM_NR, [0; 6]).unwrap();
        let r = sys.thread(pid).unwrap().regs;
        if sys.process_mut(pid).unwrap().mem.read_u64(r.rsp + 8).unwrap() != SHIM_BASE {
            break;
        }
    }
    let r = sys.thread(pid).unwrap().regs;
    let p = sys.process_mut(pid).unwrap();
    assert_eq!(p.mem.read_u64(r.rsp + 8).unwrap(), SHIM_BASE + STUB_SIZE);
    assert_eq!(r.rdi, 1, "argc");
    let a0 = p.mem.read_u64(r.rsi).unwrap();
    assert_eq!(p.mem.read_cstr(a0, 64).unwrap(), "Hello");

    // A Rust libSystem call made natively: strlen("from x86").
    let s = 0x7000_0020_0000 + 0x100; // scratch area, mapped by the loader
    p.mem.write(s, b"from x86\0").unwrap();
    sys.thread_mut(pid).unwrap().regs.rdi = s;
    let strlen = prism_stub_index(&mut prism, &mut sys, pid, "_strlen");
    prism.syscall(&mut sys, pid, PRISM_NR + strlen, [0; 6]).unwrap();
    assert_eq!(sys.thread(pid).unwrap().regs.rax, 8);
    // puts now comes from Prism's C runtime (buffered stdio), not a stub.
    assert!(prism.stub_index(pid, "_puts").is_none());

    // A raw Darwin syscall: getpid (BSD 20) with carry clear.
    sys.thread_mut(pid).unwrap().regs.rflags = 0x203;
    prism.syscall(&mut sys, pid, 0x200_0014, [0; 6]).unwrap();
    let r = sys.thread(pid).unwrap().regs;
    assert_eq!((r.rax, r.rflags & 1), (pid as u64, 0));
    // An unknown one: ENOSYS (Darwin 78) with carry set.
    prism.syscall(&mut sys, pid, 0x200_0fff, [0; 6]).unwrap();
    let r = sys.thread(pid).unwrap().regs;
    assert_eq!((r.rax, r.rflags & 1), (78, 1));
}

fn prism_stub_index(prism: &mut Prism, sys: &mut LinSystem<MemFs>, pid: u32, name: &str) -> u64 {
    prism.stub_index(pid, name).unwrap_or_else(|| panic!("no stub {name}; diag {}", sys.diag)) as u64
}

#[test]
fn loads_bundled_dylib_through_rpath() {
    let Some(dir) = binaries() else { return };
    let app = dir.join("Greet_arm64.app/Contents");
    let exe = std::fs::read(app.join("MacOS/Greet_arm64")).unwrap();
    let lib = std::fs::read(app.join("Frameworks/libgreet.dylib")).unwrap();
    let mut sys = system(&[]);
    for d in ["/Applications/Greet_arm64.app", "/Applications/Greet_arm64.app/Contents", "/Applications/Greet_arm64.app/Contents/MacOS", "/Applications/Greet_arm64.app/Contents/Frameworks"] {
        sys.fs.add_dir(d);
    }
    sys.fs.add_file("/Applications/Greet_arm64.app/Contents/MacOS/Greet_arm64", &exe, 0o755);
    sys.fs.add_file("/Applications/Greet_arm64.app/Contents/Frameworks/libgreet.dylib", &lib, 0o755);
    let mut prism = Prism::new();
    // No Info.plist: the executable is named after the bundle.
    let pid = prism.spawn(&mut sys, "/Applications/Greet_arm64.app", &["Greet".to_string()], &[]).unwrap();
    let mut rounds = 0;
    while sys.thread(pid).is_some() {
        rounds += 1;
        assert!(rounds < 1000, "runaway: {}", sys.diag);
        prism.run_arm(&mut sys, pid, 100_000);
        sys.deliver_signal(pid);
    }
    assert_eq!(String::from_utf8_lossy(&sys.console_out), "greet: hello prism from a bundled dylib\napp: greet_add=1005\n", "diag: {}", sys.diag);
    assert!(sys.diag.contains("exits with status 0") || !sys.diag.contains("exits with status"), "diag: {}", sys.diag);
}

/// Apple's own binaries (arm64e slices) on the interpreter. They are not in the repo:
/// `PRISM_MAC_BIN` names a directory holding pwd, printenv, dirname, cksum, sum, xxd and getopt
/// copied from macOS (boot/prism_macos_test.sh fetches the same set).
#[test]
fn runs_apple_arm64e_tools_on_the_interpreter() {
    let Ok(dir) = std::env::var("PRISM_MAC_BIN") else { return };
    let cases: &[(&str, &[&str], &str)] = &[
        ("/bin/pwd", &[], "/\n"),
        ("/usr/bin/printenv", &["HOME"], "/home\n"),
        ("/usr/bin/dirname", &["/a/b/c"], "/a/b\n"),
        ("/usr/bin/cksum", &["/tmp/f"], "3015617425 6 /tmp/f\n"),
        ("/usr/bin/sum", &["/tmp/f"], "36979 1 /tmp/f\n"),
        ("/usr/bin/xxd", &["/tmp/f"], "00000000: 6865 6c6c 6f0a                           hello.\n"),
        ("/usr/bin/getopt", &["ab:", "-a", "-b", "x", "y"], " -a -b x -- y\n"),
    ];
    for (path, args, want) in cases {
        let name = path.rsplit('/').next().unwrap();
        let bin = std::fs::read(format!("{dir}/{name}")).unwrap();
        let mut sys = system(&[(path, &bin), ("/tmp/f", b"hello\n")]);
        sys.fs.add_dir("/usr");
        sys.fs.add_dir("/usr/bin");
        sys.fs.add_file(path, &bin, 0o755);
        let mut prism = Prism::new();
        prism.prefer_arm = true;
        let mut argv = vec![path.to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let pid = prism.spawn(&mut sys, path, &argv, &["HOME=/home".to_string()]).unwrap_or_else(|e| panic!("{path}: {e} {}", sys.diag));
        assert!(prism.is_arm(pid));
        let mut rounds = 0;
        while sys.thread(pid).is_some() {
            rounds += 1;
            assert!(rounds < 100_000, "{path} runaway: {}", sys.diag);
            if sys.thread(pid).unwrap().state == TState::Runnable {
                prism.run_arm(&mut sys, pid, 100_000);
                sys.deliver_signal(pid);
            } else {
                sys.advance(1_000_000);
            }
        }
        assert_eq!(String::from_utf8_lossy(&sys.console_out), *want, "{path} diag: {}", sys.diag);
    }
}
