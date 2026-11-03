use loos_lin::mem::PROT_EXEC;
use loos_winrt::pe::{load, STUB};
use loos_winrt::{PeError, Process, Ret};

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");

fn proc(cmd: &str) -> Process {
    Process::create(260, 1, "csrss.exe", cmd, SVC).unwrap()
}

#[test]
fn loads_mingw_exe_and_binds_imports() {
    let mut p = proc("SessionId=1");
    assert_eq!(p.base, 0x1_4000_0000);
    assert!(p.unresolved.is_empty(), "{:?}", p.unresolved);
    assert_eq!(p.mem.prot_at(p.entry).unwrap() & PROT_EXEC, PROT_EXEC, "entry is in executable memory");
    assert!(p.entry > p.base && p.entry < p.base + 0x2000);
    // .idata is read-only in the file, but the loader patched it: every IAT slot points at a syscall stub.
    let idata = p.base + 0x5000;
    let mut found = 0;
    for slot in 0..16 {
        let a = p.mem.read_u64(idata + 0x70 + slot * 8).unwrap_or(0); // scan the area for patched pointers
        if a >= p.base + 0x6000 && a < p.base + 0x7000 {
            let s = p.mem.read_vec(a, 4).unwrap();
            assert_eq!(s, [0x4c, 0x8b, 0xd1, 0xb8]);
            found += 1;
        }
    }
    assert!(found >= 4, "found {found} patched IAT slots");
    // The entry returns into the exit stub.
    let ret = p.mem.read_u64(p.rsp).unwrap();
    let st = p.mem.read_vec(ret, STUB).unwrap();
    assert_eq!(u32::from_le_bytes(st[4..8].try_into().unwrap()), 1, "ExitProcess");
    assert_eq!(p.rsp % 16, 8, "ABI stack alignment at entry");
}

#[test]
fn syscalls_behave_like_kernel32() {
    let mut p = proc("autochk *");
    let cl = match p.syscall(5, [0; 4], 0) {
        Ret::Value(v) => v,
        r => panic!("{r:?}"),
    };
    assert_eq!(p.mem.read_vec(cl, 17).unwrap(), b"csrss.exe autochk");
    assert_eq!(p.syscall(6, [0; 4], 0), Ret::Value(260));
    let out = p.mem.map_anywhere(4096, 3, &[], "buf").unwrap();
    assert_eq!(p.syscall(7, [260, out, 0, 0], 0), Ret::Value(1));
    assert_eq!(p.mem.read_u32(out).unwrap(), 1, "session id");
    p.mem.write(out + 16, b"hello").unwrap();
    assert_eq!(p.syscall(2, [(-11i32) as u32 as u64, 0, 0, 0], 0), Ret::Value(0x14));
    assert_eq!(p.syscall(3, [0x14, out + 16, 5, out + 32], 0), Ret::Value(1));
    assert_eq!(p.mem.read_u32(out + 32).unwrap(), 5);
    assert_eq!(p.take_console(), b"hello");
    assert_eq!(p.syscall(3, [0x99, out, 1, 0], 0), Ret::Value(0), "bad handle");
    assert_eq!(p.syscall(12, [0; 4], 0), Ret::Value(6), "ERROR_INVALID_HANDLE");
    assert_eq!(p.syscall(3, [0x14, 0xdead_0000, 4, 0], 0), Ret::Value(0), "bad pointer is an error, not a crash");
    let v = match p.syscall(10, [0, 8192, 0x3000, 4], 0) {
        Ret::Value(v) => v,
        r => panic!("{r:?}"),
    };
    assert!(v != 0);
    p.mem.write(v + 4096, b"z").unwrap();
    assert_eq!(p.syscall(10, [0, 0, 0, 0], 0), Ret::Value(0));
    assert_eq!(p.syscall(8, [100, 0, 0, 0], 0), Ret::Value(0));
    assert_eq!(p.syscall(8, [0xffff_ffff, 0, 0, 0], 0), Ret::Park);
    assert_eq!(p.syscall(9, [0; 4], 1234), Ret::Value(1234));
    assert_eq!(p.syscall(14, [0; 4], 0), Ret::Value(p.base));
    assert_eq!(p.syscall(21, [u64::MAX, 7, 0, 0], 0), Ret::Exit(7));
    assert_eq!(p.syscall(21, [0x1234, 7, 0, 0], 0), Ret::Value(0), "cannot terminate other processes");
    assert_eq!(p.syscall(1, [42, 0, 0, 0], 0), Ret::Exit(42));
    assert_eq!(p.syscall(0x1000, [0; 4], 0), Ret::Exit(0xc000_0002), "unresolved import index out of range still terminates");
}

#[test]
fn rejects_bad_images() {
    let mut m = loos_lin::mem::UserMem::new();
    assert!(matches!(load(&mut m, b"not a pe"), Err(PeError::Malformed(_))));
    assert!(matches!(load(&mut m, &SVC[..0x100]), Err(PeError::Malformed(_))));
    let mut x86 = SVC.to_vec();
    let nt = u32::from_le_bytes(x86[0x3c..0x40].try_into().unwrap()) as usize;
    x86[nt + 4] = 0x4c;
    x86[nt + 5] = 0x01;
    assert!(matches!(load(&mut m, &x86), Err(PeError::Unsupported(_))));
    let mut low = SVC.to_vec();
    low[nt + 24 + 24..nt + 24 + 32].copy_from_slice(&0x400u64.to_le_bytes());
    assert!(matches!(load(&mut m, &low), Err(PeError::NoMemory)));
    let mut bad_sec = SVC.to_vec();
    let sh = nt + 24 + u16::from_le_bytes([SVC[nt + 20], SVC[nt + 21]]) as usize;
    bad_sec[sh + 12..sh + 16].copy_from_slice(&0x7fff_f000u32.to_le_bytes());
    assert!(load(&mut m, &bad_sec).is_err());
}

#[test]
fn unresolved_imports_are_reported_not_fatal_at_load() {
    // Rename an imported function in the name table so lookup fails.
    let mut img = SVC.to_vec();
    let pos = img.windows(12).position(|w| w == b"GetStdHandle").unwrap();
    img[pos..pos + 12].copy_from_slice(b"GetStdHandlX");
    let mut p = Process::create(1, 0, "x.exe", "", &img).unwrap();
    assert_eq!(p.unresolved, ["KERNEL32.dll!GetStdHandlX"]);
    let r = p.syscall(0x1000, [0; 4], 0);
    assert_eq!(r, Ret::Exit(0xc000_0002));
    assert!(String::from_utf8(p.take_console()).unwrap().contains("unimplemented import KERNEL32.dll!GetStdHandlX"));
}
