use loos_winrt::Process;

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");
const DLL: &[u8] = include_bytes!("../../../tests/win/testdll.dll");

fn proc() -> Process {
    let mut p = Process::create(300, 1, "t.exe", "", SVC).unwrap();
    p.add_file("C:\\Windows\\System32\\TestDll.dll", DLL);
    p
}

#[test]
fn load_library_relocates_resolves_exports_and_queues_dllmain() {
    let mut p = proc();
    let h = p.load_library("testdll.dll").expect("loads");
    assert_eq!(p.load_library("TESTDLL").unwrap(), h, "second load returns the same module");
    let add = p.proc_address(h, Some("add"), 0);
    assert!(add > h && add < h + 0x10000, "export inside the image: {add:#x}");
    assert_eq!(p.proc_address(h, Some("nope"), 0), 0);
    assert_eq!(p.next_init().map(|(_, b)| b), Some(h), "DllMain queued once");
    assert_eq!(p.next_init(), None);
}

#[test]
fn builtin_modules_hand_out_stub_pointers() {
    let mut p = proc();
    let k = p.load_library("KERNEL32.dll").unwrap();
    assert_eq!(Some(k), p.module_handle("kernel32"));
    let f = p.proc_address(k, Some("GetTickCount"), 0);
    assert_eq!(p.mem.read_vec(f, 4).unwrap(), [0x4c, 0x8b, 0xd1, 0xb8], "syscall stub");
    assert!(p.load_library("missing.dll").is_none());
}

#[test]
fn api_set_contracts_resolve_to_their_host_module() {
    let mut p = proc();
    let k = p.module_handle("kernelbase.dll").unwrap();
    assert_eq!(p.load_library("api-ms-win-core-synch-l1-2-0.dll"), Some(k));
    assert_eq!(p.module_handle("ext-ms-win-kernel32-package-l1-1-0"), Some(k));
    assert_eq!(p.load_library("api-ms-win-crt-stdio-l1-1-0.dll"), p.module_handle("ucrtbase.dll"));
    for dll in ["shlwapi.dll", "oleaut32.dll", "rpcrt4.dll", "bcrypt.dll"] {
        assert!(p.load_library(dll).is_some(), "{dll}");
    }
    let f = p.proc_address(k, Some("GetTickCount"), 0);
    assert_eq!(p.mem.read_vec(f, 4).unwrap(), [0x4c, 0x8b, 0xd1, 0xb8]);
    assert_eq!(p.syscall(12, [0; 4], 0), loos_winrt::Ret::Value(0), "built-in contracts do not touch the last error");
}

#[test]
fn volume_paths_are_normalised() {
    let mut p = proc();
    p.add_file("/Users/a.txt", b"x");
    assert_eq!(p.vfs.get("/users/a.txt").map(Vec::as_slice), Some(&b"x"[..]));
    assert_eq!(loos_winrt::files::norm("C:\\Users\\A.TXT"), "/users/a.txt");
}
