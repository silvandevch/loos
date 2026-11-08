//! advapi32 registry, shlwapi paths, COM/BSTR basics and the service control manager.
use loos_winrt::adv::lookup;
use loos_winrt::{Process, Ret};

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");

struct T {
    p: Process,
    buf: u64,
    at: u64,
}

fn w(s: &str) -> Vec<u8> {
    let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    b.extend_from_slice(&[0, 0]);
    b
}

impl T {
    fn new() -> T {
        let mut p = Process::create(1, 1, "t.exe", "", SVC).unwrap();
        let buf = p.mem.map_anywhere(0x8000, 3, &[], "buf").unwrap();
        p.cur_rsp = buf + 0x7000;
        T { p, buf, at: 0x100 }
    }
    /// Writes a wide string into scratch memory and returns its address.
    fn ws(&mut self, s: &str) -> u64 {
        let a = self.buf + self.at;
        self.at += 2 * (s.len() as u64 + 2);
        self.p.mem.write(a, &w(s)).unwrap();
        a
    }
    fn mem(&mut self, n: u64) -> u64 {
        let a = self.buf + self.at;
        self.at += n.max(8);
        a
    }
    fn call(&mut self, name: &str, a: [u64; 4], stack: &[u64]) -> u64 {
        for (k, v) in stack.iter().enumerate() {
            self.p.mem.write_u64(self.p.cur_rsp + 0x28 + 8 * k as u64, *v).unwrap();
        }
        match self.p.syscall(lookup(name).unwrap_or_else(|| panic!("{name}")), a, 77) {
            Ret::Value(v) => v,
            r => panic!("{name}: {r:?}"),
        }
    }
}

const HKLM: u64 = 0xffff_ffff_8000_0002;
const HKCU: u64 = 0x8000_0001;

#[test]
fn registry_create_set_query_enum_delete() {
    let mut t = T::new();
    let (sub, out, disp) = (t.ws("Software\\LoOS\\Test"), t.mem(8), t.mem(4));
    // RegCreateKeyExW(hKey, sub, 0, class, options | sam | sec | &out | &disp)
    assert_eq!(t.call("RegCreateKeyExW", [HKCU, sub, 0, 0], &[0, 0, 0, out, disp]), 0);
    assert_eq!(t.p.mem.read_u32(disp).unwrap(), 1, "REG_CREATED_NEW_KEY");
    let key = t.p.mem.read_u64(out).unwrap();
    let (n, data) = (t.ws("Greeting"), t.ws("hello"));
    assert_eq!(t.call("RegSetValueExW", [key, n, 0, 1], &[data, 12]), 0);
    let (dn, dv) = (t.ws("Count"), t.mem(4));
    t.p.mem.write_u32(dv, 42).unwrap();
    assert_eq!(t.call("RegSetValueExW", [key, dn, 0, 4], &[dv, 4]), 0);

    // Size negotiation: NULL data reports the size, a small buffer reports ERROR_MORE_DATA.
    let (ty, cb, buf) = (t.mem(4), t.mem(4), t.mem(32));
    assert_eq!(t.call("RegQueryValueExW", [key, n, 0, ty], &[0, cb]), 0);
    assert_eq!(t.p.mem.read_u32(cb).unwrap(), 12);
    t.p.mem.write_u32(cb, 4).unwrap();
    assert_eq!(t.call("RegQueryValueExW", [key, n, 0, ty], &[buf, cb]), 234);
    t.p.mem.write_u32(cb, 32).unwrap();
    assert_eq!(t.call("RegQueryValueExW", [key, n, 0, ty], &[buf, cb]), 0);
    assert_eq!(t.p.mem.read_u32(ty).unwrap(), 1);
    assert_eq!(t.p.mem.read_vec(buf, 12).unwrap(), w("hello"));
    assert_eq!(t.call("RegQueryValueExW", [key, t.buf + 0x7f00, 0, ty], &[0, 0]), 2, "missing value");

    // Reopen read-only through the parent by relative path; enumerate values.
    let (parent, rel, out2) = (t.ws("Software\\LoOS"), t.ws("Test"), t.mem(8));
    assert_eq!(t.call("RegOpenKeyExW", [HKCU, parent, 0, 0], &[out2]), 0);
    let pk = t.p.mem.read_u64(out2).unwrap();
    assert_eq!(t.call("RegOpenKeyExW", [pk, rel, 0, 0], &[out2]), 0);
    let k2 = t.p.mem.read_u64(out2).unwrap();
    let (name, cch) = (t.mem(64), t.mem(4));
    t.p.mem.write_u32(cch, 32).unwrap();
    assert_eq!(t.call("RegEnumValueW", [k2, 1, name, cch], &[0, ty, 0, 0]), 0);
    assert_eq!(t.p.mem.read_cstr(name, 0).unwrap_or_default().len(), 0); // wide: first byte 'C'
    assert_eq!(t.p.mem.read_vec(name, 10).unwrap(), w("Count")[..10]);
    assert_eq!(t.p.mem.read_u32(ty).unwrap(), 4);
    assert_eq!(t.call("RegEnumValueW", [k2, 2, name, cch], &[0, ty, 0, 0]), 259, "ERROR_NO_MORE_ITEMS");
    t.p.mem.write_u32(cch, 32).unwrap();
    assert_eq!(t.call("RegEnumKeyExW", [pk, 0, name, cch], &[0]), 0);
    assert_eq!(t.p.mem.read_vec(name, 8).unwrap(), w("Test")[..8]);

    // Delete value, delete key (parent with subkeys is refused), close.
    assert_eq!(t.call("RegDeleteValueW", [key, n, 0, 0], &[]), 0);
    assert_eq!(t.call("RegDeleteValueW", [key, n, 0, 0], &[]), 2);
    let lo = t.ws("Software\\LoOS");
    assert_eq!(t.call("RegDeleteKeyW", [HKCU, lo, 0, 0], &[]), 5);
    let tst = t.ws("Software\\LoOS\\Test");
    assert_eq!(t.call("RegDeleteKeyW", [HKCU, tst, 0, 0], &[]), 0);
    let again = t.ws("Software\\LoOS\\Test");
    assert_eq!(t.call("RegOpenKeyExW", [HKCU, again, 0, 0], &[out2]), 2);
    assert_eq!(t.call("RegCloseKey", [key, 0, 0, 0], &[]), 0);
    assert_eq!(t.call("RegCloseKey", [key, 0, 0, 0], &[]), 6, "double close");
}

#[test]
fn seeded_version_keys_are_readable() {
    let mut t = T::new();
    let (sub, v, cb, buf) = (t.ws("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"), t.ws("ProductName"), t.mem(4), t.mem(128));
    t.p.mem.write_u32(cb, 128).unwrap();
    // RegGetValueW(hKey, sub, value, flags, &type, data, &cb)
    assert_eq!(t.call("RegGetValueW", [HKLM, sub, v, 0], &[0, buf, cb]), 0);
    let n = t.p.mem.read_u32(cb).unwrap() as usize;
    assert_eq!(t.p.mem.read_vec(buf, n).unwrap(), w("Windows 10 Pro"));
}

#[test]
fn shlwapi_path_helpers() {
    let mut t = T::new();
    let p = t.ws("C:\\dir\\sub\\file.tar.gz");
    let fname = t.call("PathFindFileNameW", [p, 0, 0, 0], &[]);
    assert_eq!(fname, p + 2 * "C:\\dir\\sub\\".len() as u64);
    let ext = t.call("PathFindExtensionW", [p, 0, 0, 0], &[]);
    assert_eq!(t.p.mem.read_vec(ext, 6).unwrap(), w(".gz")[..6]);
    let (out, d, f) = (t.mem(520), t.ws("C:\\dir"), t.ws("a.txt"));
    assert_eq!(t.call("PathCombineW", [out, d, f, 0], &[]), out);
    assert_eq!(t.p.mem.read_vec(out, 2 * 12 + 2).unwrap(), w("C:\\dir\\a.txt"));
    let abs = t.ws("D:\\x.txt");
    t.call("PathCombineW", [out, d, abs, 0], &[]);
    assert_eq!(t.p.mem.read_vec(out, 2 * 8 + 2).unwrap(), w("D:\\x.txt"));
    let spec = t.mem(520);
    t.p.mem.write(spec, &w("C:\\dir\\file.txt")).unwrap();
    assert_eq!(t.call("PathRemoveFileSpecW", [spec, 0, 0, 0], &[]), 1);
    assert_eq!(t.p.mem.read_vec(spec, 2 * 6 + 2).unwrap(), w("C:\\dir"));
    let rel = t.ws("a\\b");
    assert_eq!(t.call("PathIsRelativeW", [rel, 0, 0, 0], &[]), 1);
    t.p.add_file("C:\\data\\x.bin", b"1");
    let (f1, f2, f3) = (t.ws("C:\\data\\x.bin"), t.ws("C:\\data"), t.ws("C:\\nope"));
    assert_eq!(t.call("PathFileExistsW", [f1, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("PathFileExistsW", [f2, 0, 0, 0], &[]), 1, "directory implied by a file");
    assert_eq!(t.call("PathFileExistsW", [f3, 0, 0, 0], &[]), 0);
}

#[test]
fn com_guid_and_bstr_basics() {
    let mut t = T::new();
    assert_eq!(t.call("CoInitialize", [0; 4], &[]), 0);
    assert_eq!(t.call("CoInitialize", [0; 4], &[]), 1, "S_FALSE");
    let (s, g, out) = (t.ws("{6B29FC40-CA47-1067-B31D-00DD010662DA}"), t.mem(16), t.mem(100));
    assert_eq!(t.call("CLSIDFromString", [s, g, 0, 0], &[]), 0);
    assert_eq!(t.p.mem.read_vec(g, 4).unwrap(), [0x40, 0xFC, 0x29, 0x6B]);
    assert_eq!(t.call("StringFromGUID2", [g, out, 39, 0], &[]), 39);
    assert_eq!(t.p.mem.read_vec(out, 78).unwrap(), w("{6B29FC40-CA47-1067-B31D-00DD010662DA}")[..78]);
    assert_eq!(t.call("StringFromGUID2", [g, out, 10, 0], &[]), 0);
    let bad = t.ws("not-a-guid");
    assert_eq!(t.call("CLSIDFromString", [bad, g, 0, 0], &[]), 0x8007_0057);
    let (g1, g2) = (t.mem(16), t.mem(16));
    t.call("CoCreateGuid", [g1, 0, 0, 0], &[]);
    t.call("CoCreateGuid", [g2, 0, 0, 0], &[]);
    assert_ne!(t.p.mem.read_vec(g1, 16).unwrap(), t.p.mem.read_vec(g2, 16).unwrap());
    let ptr = t.mem(8);
    assert_eq!(t.call("CoCreateInstance", [g, 0, 1, g], &[ptr]), 0x8004_0154, "REGDB_E_CLASSNOTREG");
    let src = t.ws("bstr!");
    let b = t.call("SysAllocString", [src, 0, 0, 0], &[]);
    assert_ne!(b, 0);
    assert_eq!(t.call("SysStringLen", [b, 0, 0, 0], &[]), 5);
    assert_eq!(t.p.mem.read_vec(b, 10).unwrap(), w("bstr!")[..10]);
    assert_eq!(t.call("SysStringLen", [0, 0, 0, 0], &[]), 0);
}

#[test]
fn service_control_lifecycle() {
    let mut t = T::new();
    let scm = t.call("OpenSCManagerW", [0; 4], &[]);
    let (name, disp, path) = (t.ws("LoosDemo"), t.ws("Demo Service"), t.ws("C:\\svc\\demo.exe"));
    // CreateServiceW(scm, name, display, access | type, start, errctl, path)
    let h = t.call("CreateServiceW", [scm, name, disp, 0xf01ff], &[0x10, 3, 1, path]);
    assert_ne!(h, 0);
    assert_eq!(t.call("CreateServiceW", [scm, name, disp, 0xf01ff], &[0x10, 3, 1, path]), 0, "duplicate");
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(1073));
    let st = t.mem(28);
    assert_eq!(t.call("QueryServiceStatus", [h, st, 0, 0], &[]), 1);
    assert_eq!(t.p.mem.read_u32(st + 4).unwrap(), 1, "STOPPED");
    assert_eq!(t.call("StartServiceW", [h, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("StartServiceW", [h, 0, 0, 0], &[]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(1056));
    t.call("QueryServiceStatus", [h, st, 0, 0], &[]);
    assert_eq!(t.p.mem.read_u32(st + 4).unwrap(), 4, "RUNNING");
    assert_eq!(t.call("ControlService", [h, 1, st, 0], &[]), 1);
    assert_eq!(t.p.mem.read_u32(st + 4).unwrap(), 1);
    assert_eq!(t.call("ControlService", [h, 1, st, 0], &[]), 0, "already stopped");
    // The service is a registry entry: a second handle finds it, and it is gone after DeleteService.
    let h2 = t.call("OpenServiceW", [scm, name, 0, 0], &[]);
    assert_ne!(h2, 0);
    assert_eq!(t.call("DeleteService", [h2, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("OpenServiceW", [scm, name, 0, 0], &[]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(1060));
    assert_eq!(t.call("CloseServiceHandle", [h, 0, 0, 0], &[]), 1);
}

use loos_security::sam::{Lsa, Sam};
use loos_security::uac::SID_ADMINS;

fn machine(t: &mut T) -> (u32, u32) {
    let mut sam = Sam::new("LOOS", "S-1-5-21-1-2-3", 2);
    let bob = sam.create_user("Bob", b"hunter2hunter2", [1; 16], 0).unwrap();
    let ann = sam.create_user("Ann", b"correct-horse", [2; 16], 0).unwrap();
    let sid = sam.user(ann).unwrap().account.sid.clone();
    sam.add_member(SID_ADMINS, &sid).unwrap();
    t.p.set_accounts(sam, Lsa::new(&[9; 32]), bob);
    (bob, ann)
}

fn rd_wide(t: &mut T, at: u64, n: usize) -> String {
    let b = t.p.mem.read_vec(at, n * 2).unwrap();
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
    String::from_utf16_lossy(&u)
}

#[test]
fn user_and_computer_name_two_call_protocol() {
    let mut t = T::new();
    machine(&mut t);
    let (buf, cch) = (t.mem(64), t.mem(4));
    t.p.mem.write_u32(cch, 2).unwrap();
    assert_eq!(t.call("GetUserNameW", [buf, cch, 0, 0], &[]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(122));
    assert_eq!(t.p.mem.read_u32(cch).unwrap(), 4, "needs 3 chars + NUL");
    assert_eq!(t.call("GetUserNameW", [buf, cch, 0, 0], &[]), 1);
    assert_eq!(rd_wide(&mut t, buf, 8), "Bob");
    t.p.mem.write_u32(cch, 32).unwrap();
    assert_eq!(t.call("GetComputerNameW", [buf, cch, 0, 0], &[]), 1);
    assert_eq!(rd_wide(&mut t, buf, 8), "LOOS");
    assert_eq!(t.p.mem.read_u32(cch).unwrap(), 4);
}

#[test]
fn default_identity_without_installed_accounts() {
    let mut t = T::new();
    let (buf, cch) = (t.mem(64), t.mem(4));
    t.p.mem.write_u32(cch, 32).unwrap();
    assert_eq!(t.call("GetUserNameW", [buf, cch, 0, 0], &[]), 1);
    assert_eq!(rd_wide(&mut t, buf, 8), "User");
}

#[test]
fn lookup_account_name_sid_and_string_sids() {
    let mut t = T::new();
    machine(&mut t);
    let (name, sid, cb, dom, cchd, ty) = (t.ws("Bob"), t.mem(32), t.mem(4), t.mem(64), t.mem(4), t.mem(4));
    t.p.mem.write_u32(cb, 4).unwrap();
    t.p.mem.write_u32(cchd, 1).unwrap();
    assert_eq!(t.call("LookupAccountNameW", [0, name, sid, cb], &[dom, cchd, ty]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(122));
    assert_eq!((t.p.mem.read_u32(cb).unwrap(), t.p.mem.read_u32(cchd).unwrap()), (28, 5), "5 sub-authorities, domain LOOS+NUL");
    t.p.mem.write_u32(cb, 28).unwrap();
    t.p.mem.write_u32(cchd, 5).unwrap();
    assert_eq!(t.call("LookupAccountNameW", [0, name, sid, cb], &[dom, cchd, ty]), 1);
    assert_eq!(rd_wide(&mut t, dom, 8), "LOOS");
    assert_eq!(t.p.mem.read_u32(ty).unwrap(), 1, "SidTypeUser");
    // Binary layout: revision 1, 5 sub-authorities, authority 5, then 21,1,2,3,1000.
    let raw = t.p.mem.read_vec(sid, 28).unwrap();
    assert_eq!(&raw[..8], &[1, 5, 0, 0, 0, 0, 0, 5]);
    assert_eq!(u32::from_le_bytes(raw[24..28].try_into().unwrap()), 1000);
    assert_eq!(t.call("IsValidSid", [sid, 0, 0, 0], &[]), 1);
    assert_eq!(t.call("GetLengthSid", [sid, 0, 0, 0], &[]), 28);
    // SID <-> string.
    let out = t.mem(8);
    assert_eq!(t.call("ConvertSidToStringSidW", [sid, out, 0, 0], &[]), 1);
    let p = t.p.mem.read_u64(out).unwrap();
    assert_eq!(rd_wide(&mut t, p, 40), "S-1-5-21-1-2-3-1000");
    let out2 = t.mem(8);
    assert_eq!(t.call("ConvertStringSidToSidW", [p, out2, 0, 0], &[]), 1);
    let p2 = t.p.mem.read_u64(out2).unwrap();
    assert_eq!(t.call("EqualSid", [sid, p2, 0, 0], &[]), 1);
    // Reverse lookup.
    let (n2, cn, d2, cd2, ty2) = (t.mem(64), t.mem(4), t.mem(64), t.mem(4), t.mem(4));
    t.p.mem.write_u32(cn, 32).unwrap();
    t.p.mem.write_u32(cd2, 32).unwrap();
    assert_eq!(t.call("LookupAccountSidW", [0, sid, n2, cn], &[d2, cd2, ty2]), 1);
    assert_eq!((rd_wide(&mut t, n2, 16), rd_wide(&mut t, d2, 16)), ("Bob".into(), "LOOS".into()));
    // Well-known and unknown names.
    let adm = t.ws("BUILTIN\\Administrators");
    t.p.mem.write_u32(cb, 28).unwrap();
    t.p.mem.write_u32(cchd, 32).unwrap();
    assert_eq!(t.call("LookupAccountNameW", [0, adm, sid, cb], &[dom, cchd, ty]), 1);
    assert_eq!(t.p.mem.read_vec(sid, 16).unwrap(), [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0]);
    assert_eq!(t.p.mem.read_u32(ty).unwrap(), 4, "SidTypeAlias");
    let nobody = t.ws("Nobody");
    assert_eq!(t.call("LookupAccountNameW", [0, nobody, sid, cb], &[dom, cchd, ty]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(1332));
    let bad = t.ws("S-2-5");
    assert_eq!(t.call("ConvertStringSidToSidW", [bad, out2, 0, 0], &[]), 0);
}

#[test]
fn process_token_information_and_membership() {
    let mut t = T::new();
    machine(&mut t);
    let h = t.mem(8);
    assert_eq!(t.call("OpenProcessToken", [u64::MAX, 8, h, 0], &[]), 1);
    let tok = t.p.mem.read_u64(h).unwrap();
    let (buf, rl) = (t.mem(1024), t.mem(4));
    // TokenUser: a SID_AND_ATTRIBUTES whose pointer targets the SID inside the buffer.
    assert_eq!(t.call("GetTokenInformation", [tok, 1, 0, 0], &[rl]), 0, "size query");
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(122));
    let need = t.p.mem.read_u32(rl).unwrap() as u64;
    assert_eq!(t.call("GetTokenInformation", [tok, 1, buf, need - 1], &[rl]), 0, "one byte short");
    assert_eq!(t.call("GetTokenInformation", [tok, 1, buf, need], &[rl]), 1);
    let sidp = t.p.mem.read_u64(buf).unwrap();
    assert!(sidp >= buf && sidp < buf + need);
    let s = t.mem(8);
    assert_eq!(t.call("ConvertSidToStringSidW", [sidp, s, 0, 0], &[]), 1);
    let sp = t.p.mem.read_u64(s).unwrap();
    assert_eq!(rd_wide(&mut t, sp, 40), "S-1-5-21-1-2-3-1000");
    // Groups: Everyone, Authenticated Users, Users; none administrative for Bob.
    assert_eq!(t.call("GetTokenInformation", [tok, 2, buf, 1024], &[rl]), 1);
    assert_eq!(t.p.mem.read_u32(buf).unwrap(), 3);
    // Standard user: no debug privilege, not elevated, medium integrity label.
    assert_eq!(t.call("GetTokenInformation", [tok, 20, buf, 4], &[rl]), 1);
    assert_eq!(t.p.mem.read_u32(buf).unwrap(), 0, "TokenElevation");
    assert_eq!(t.call("GetTokenInformation", [tok, 25, buf, 1024], &[rl]), 1);
    let ip = t.p.mem.read_u64(buf).unwrap();
    assert_eq!(t.call("ConvertSidToStringSidW", [ip, s, 0, 0], &[]), 1);
    let sp = t.p.mem.read_u64(s).unwrap();
    assert_eq!(rd_wide(&mut t, sp, 40), "S-1-16-8192");
    assert_eq!(t.call("GetTokenInformation", [tok, 999, buf, 1024], &[rl]), 0, "unknown class");
    // Privileges: look up by name and round-trip the LUID.
    let (pn, luid) = (t.ws("SeShutdownPrivilege"), t.mem(8));
    assert_eq!(t.call("LookupPrivilegeValueW", [0, pn, luid, 0], &[]), 1);
    let (nb, ncch) = (t.mem(64), t.mem(4));
    t.p.mem.write_u32(ncch, 64).unwrap();
    assert_eq!(t.call("LookupPrivilegeNameW", [0, luid, nb, ncch], &[]), 1);
    assert_eq!(rd_wide(&mut t, nb, 32), "SeShutdownPrivilege");
    let dbg = t.ws("SeDebugPrivilege");
    t.call("LookupPrivilegeValueW", [0, dbg, luid, 0], &[]);
    assert_eq!(t.call("GetTokenInformation", [tok, 3, buf, 1024], &[rl]), 1);
    let count = t.p.mem.read_u32(buf).unwrap() as u64;
    let want = t.p.mem.read_u32(luid).unwrap();
    assert!((0..count).all(|i| t.p.mem.read_u32(buf + 4 + 12 * i).unwrap() != want), "no SeDebugPrivilege for Bob");
    // CheckTokenMembership + IsUserAnAdmin.
    let (users, admins, res) = (t.ws("S-1-5-32-545"), t.ws("S-1-5-32-544"), t.mem(4));
    for (str_sid, member) in [(users, 1), (admins, 0)] {
        let out = t.mem(8);
        assert_eq!(t.call("ConvertStringSidToSidW", [str_sid, out, 0, 0], &[]), 1);
        let sid = t.p.mem.read_u64(out).unwrap();
        assert_eq!(t.call("CheckTokenMembership", [0, sid, res, 0], &[]), 1);
        assert_eq!(t.p.mem.read_u32(res).unwrap(), member);
    }
    assert_eq!(t.call("IsUserAnAdmin", [0; 4], &[]), 0);
}

#[test]
fn logon_user_authenticates_with_lockout() {
    let mut t = T::new();
    let (_, _ann) = machine(&mut t);
    let (user, good, bad, out, buf) = (t.ws("Ann"), t.ws("correct-horse"), t.ws("wrong"), t.mem(8), t.mem(64));
    // LogonUserW(user, domain, password, type=INTERACTIVE, provider=0, &token)
    assert_eq!(t.call("LogonUserW", [user, 0, bad, 2], &[0, out]), 0);
    assert_eq!(t.p.syscall(12, [0; 4], 0), Ret::Value(1326));
    assert_eq!(t.call("LogonUserW", [user, 0, good, 2], &[0, out]), 1);
    let tok = t.p.mem.read_u64(out).unwrap();
    // Administrator logon yields the full (elevated) token; the process token itself stays Bob's.
    assert_eq!(t.call("GetTokenInformation", [tok, 20, buf, 4], &[0]), 1);
    assert_eq!(t.p.mem.read_u32(buf).unwrap(), 1, "elevated");
    assert_eq!(t.call("GetTokenInformation", [tok, 18, buf, 4], &[0]), 1);
    assert_eq!(t.p.mem.read_u32(buf).unwrap(), 2, "TokenElevationTypeFull");
    let (dbg, luid) = (t.ws("SeDebugPrivilege"), t.mem(8));
    t.call("LookupPrivilegeValueW", [0, dbg, luid, 0], &[]);
    assert_eq!(t.call("GetTokenInformation", [tok, 3, buf, 64], &[0]), 0, "privilege list does not fit 64 bytes");
    assert_eq!(t.call("GetTokenInformation", [tok, 3, buf, 64 + 12 * 10], &[0]), 1);
    let want = t.p.mem.read_u32(luid).unwrap();
    let count = t.p.mem.read_u32(buf).unwrap() as u64;
    assert!((0..count).any(|i| t.p.mem.read_u32(buf + 4 + 12 * i).unwrap() == want), "administrators hold SeDebugPrivilege");
    // Repeated failures lock the account: even the right password then fails.
    for _ in 0..5 {
        t.call("LogonUserW", [user, 0, bad, 2], &[0, out]);
    }
    assert_eq!(t.call("LogonUserW", [user, 0, good, 2], &[0, out]), 0, "locked out");
    // Unsupported logon type, unknown user.
    let nobody = t.ws("Nobody");
    assert_eq!(t.call("LogonUserW", [nobody, 0, good, 2], &[0, out]), 0);
}
