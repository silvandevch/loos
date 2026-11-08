//! advapi32 registry and service control, shlwapi path helpers, and ole32/oleaut32 basics (COM init, task memory,
//! GUID strings, BSTR). The registry is a per-process [`loos_registry::Hive`] seeded with the version keys programs
//! probe; services are registry entries plus an in-memory run state (no process is spawned by `StartService`).

use crate::{files, k32_wide, Process, Ret};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use loos_registry::{Hive, Value, ValueType};
use loos_security::sam::{Lsa, Sam, PRIVILEGES};
use loos_security::uac::{Integrity, Token, SID_ADMINS};

/// First syscall number of this table.
pub const ADV_BASE: u32 = 0xc00;

const ERROR_FILE_NOT_FOUND: u64 = 2;
const ERROR_INVALID_HANDLE: u64 = 6;
const ERROR_MORE_DATA: u64 = 234;
const ERROR_NO_MORE_ITEMS: u64 = 259;
const ERROR_INVALID_PARAMETER: u64 = 87;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;
const ERROR_SERVICE_EXISTS: u32 = 1073;
const REGDB_E_CLASSNOTREG: u64 = 0x8004_0154;
const E_INVALIDARG: u64 = 0x8007_0057;
const HANDLE_BASE: u64 = 0x5000;
const SCM_HANDLE: u64 = 0x4fff0;
const SVC_BASE: u64 = 0x6000;
const TOKEN_BASE: u64 = 0x7000;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NONE_MAPPED: u32 = 1332;
const SERVICES_KEY: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Services";

const NAMES: &[&str] = &[
    "RegOpenKeyExW", "RegCreateKeyExW", "RegCreateKeyW", "RegCloseKey", "RegQueryValueExW", "RegSetValueExW", "RegDeleteValueW",
    "RegDeleteKeyW", "RegEnumKeyExW", "RegEnumValueW", "RegGetValueW", "RegFlushKey", "RegQueryInfoKeyW",
    // shlwapi
    "PathFindFileNameW", "PathFindExtensionW", "PathCombineW", "PathAddBackslashW", "PathRemoveFileSpecW", "PathIsRelativeW",
    "PathFileExistsW",
    // ole32 / oleaut32
    "CoInitialize", "OleInitialize", "OleUninitialize", "CoCreateInstance", "CoTaskMemAlloc", "CoTaskMemFree", "StringFromGUID2",
    "CLSIDFromString", "IIDFromString", "CoCreateGuid", "SysAllocString", "SysAllocStringLen", "SysFreeString", "SysStringLen",
    // service control manager
    "OpenSCManagerW", "CreateServiceW", "OpenServiceW", "CloseServiceHandle", "StartServiceW", "ControlService",
    "QueryServiceStatus", "DeleteService",
    // accounts and tokens (SAM/LSA)
    "LookupAccountNameW", "LookupAccountSidW", "ConvertSidToStringSidW", "ConvertStringSidToSidW", "GetUserNameW",
    "GetComputerNameW", "OpenProcessToken", "OpenThreadToken", "GetTokenInformation", "LogonUserW", "LookupPrivilegeValueW",
    "LookupPrivilegeNameW", "IsValidSid", "GetLengthSid", "EqualSid", "IsUserAnAdmin", "CheckTokenMembership",
];

/// Syscall number of an advapi32/shlwapi/ole32 import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| ADV_BASE + i as u32)
}

/// Per-process state of this module.
#[derive(Default)]
pub struct State {
    hive: Option<Hive>,
    /// Open keys: full path (`HKLM\...`), `None` once closed.
    keys: Vec<Option<String>>,
    /// Service run states (`SERVICE_*`), by lower-case name.
    svc: BTreeMap<String, u32>,
    /// Open service handles: service name.
    svc_handles: Vec<Option<String>>,
    guids: u64,
    com_init: u32,
    accounts: Option<(Sam, Lsa)>,
    /// Token table: index 0 is the process token (a handle is `TOKEN_BASE + 4 * index`).
    tokens: Vec<Token>,
    /// RID of the user the process runs as.
    rid: u32,
}

fn seed() -> Hive {
    let mut h = Hive::new();
    let cv = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let wide = |s: &str| {
        let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        v.extend_from_slice(&[0, 0]);
        v
    };
    for (n, s) in [("ProductName", "Windows 10 Pro"), ("CurrentBuildNumber", "19045"), ("CurrentVersion", "6.3"), ("EditionID", "Professional")] {
        let _ = h.set_value(cv, n, ValueType::Sz, &wide(s));
    }
    let _ = h.set_value(cv, "CurrentMajorVersionNumber", ValueType::Dword, &10u32.to_le_bytes());
    let _ = h.create_key("HKCU\\Software");
    let _ = h.create_key("HKCR");
    let _ = h.create_key(SERVICES_KEY);
    h
}

fn root_name(h: u64) -> Option<&'static str> {
    Some(match h as u32 {
        0x8000_0000 => "HKCR",
        0x8000_0001 => "HKCU",
        0x8000_0002 => "HKLM",
        0x8000_0003 => "HKU",
        0x8000_0005 => "HKCC",
        _ => return None,
    })
    .filter(|_| h >> 32 == 0 || h >> 32 == 0xffff_ffff)
}

fn join(base: &str, sub: &str) -> String {
    let sub = sub.trim_matches('\\');
    if sub.is_empty() { base.to_string() } else { alloc::format!("{base}\\{sub}") }
}

fn to_raw(k: ValueType) -> u32 {
    match k {
        ValueType::Sz => 1,
        ValueType::ExpandSz => 2,
        ValueType::Binary => 3,
        ValueType::Dword => 4,
        ValueType::MultiSz => 7,
        ValueType::Qword => 11,
        ValueType::Other(v) => v,
    }
}

fn from_raw(v: u32) -> ValueType {
    match v {
        1 => ValueType::Sz,
        2 => ValueType::ExpandSz,
        3 => ValueType::Binary,
        4 => ValueType::Dword,
        7 => ValueType::MultiSz,
        11 => ValueType::Qword,
        v => ValueType::Other(v),
    }
}

impl Process {
    fn hive(&mut self) -> &mut Hive {
        self.adv.hive.get_or_insert_with(seed)
    }

    /// Full registry path of an `HKEY` (predefined root or an open key).
    fn key_path(&self, h: u64) -> Option<String> {
        if let Some(r) = root_name(h) {
            return Some(r.to_string());
        }
        let i = h.checked_sub(HANDLE_BASE)? as usize / 4;
        self.adv.keys.get(i)?.clone()
    }

    fn new_key(&mut self, path: String) -> u64 {
        self.adv.keys.push(Some(path));
        HANDLE_BASE + (self.adv.keys.len() as u64 - 1) * 4
    }

    fn wstr(&mut self, p: u64) -> Option<String> {
        if p == 0 { Some(String::new()) } else { k32_wide(&mut self.mem, p) }
    }

    fn put_out(&mut self, p: u64, v: u64) {
        if p != 0 {
            let _ = self.mem.write(p, &v.to_le_bytes());
        }
    }

    /// Copies a value to the caller's `(type*, data*, size*)` triple with the registry's size negotiation.
    fn give_value(&mut self, val: &Value, ty: u64, data: u64, cb: u64) -> u64 {
        if ty != 0 {
            let _ = self.mem.write(ty, &to_raw(val.kind).to_le_bytes());
        }
        let cap = if cb != 0 { self.mem.read_u32(cb).unwrap_or(0) as usize } else { 0 };
        if cb != 0 {
            let _ = self.mem.write(cb, &(val.data.len() as u32).to_le_bytes());
        }
        if data == 0 {
            return 0;
        }
        if cap < val.data.len() {
            return ERROR_MORE_DATA;
        }
        let _ = self.mem.write(data, &val.data);
        0
    }

    fn svc_key(name: &str) -> String {
        alloc::format!("{SERVICES_KEY}\\{name}")
    }

    fn svc_name(&self, h: u64) -> Option<String> {
        let i = h.checked_sub(SVC_BASE)? as usize / 4;
        self.adv.svc_handles.get(i)?.clone()
    }

    /// advapi32/shlwapi/ole32 calls. `None` for numbers outside this table.
    pub(crate) fn adv_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now: u64) -> Option<Ret> {
        let name = *NAMES.get(nr.checked_sub(ADV_BASE)? as usize)?;
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "RegOpenKeyExW" | "RegCreateKeyExW" | "RegCreateKeyW" => {
                let create = name != "RegOpenKeyExW";
                let Some(base) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let sub = self.wstr(a[1])?;
                let path = join(&base, &sub);
                let (out, disp) = match name {
                    "RegOpenKeyExW" => (self.stack_arg(rsp, 5), 0),
                    "RegCreateKeyW" => (a[2], 0),
                    _ => (self.stack_arg(rsp, 8), self.stack_arg(rsp, 9)),
                };
                let existed = self.hive().key(&path).is_ok();
                if !existed {
                    if !create {
                        return v(ERROR_FILE_NOT_FOUND);
                    }
                    if self.hive().create_key(&path).is_err() {
                        return v(ERROR_INVALID_PARAMETER);
                    }
                }
                let h = self.new_key(path);
                self.put_out(out, h);
                if disp != 0 {
                    let _ = self.mem.write(disp, &(if existed { 2u32 } else { 1 }).to_le_bytes());
                }
                v(0)
            }
            "RegCloseKey" => {
                if root_name(a[0]).is_some() {
                    return v(0);
                }
                let i = (a[0].wrapping_sub(HANDLE_BASE) / 4) as usize;
                match self.adv.keys.get_mut(i) {
                    Some(k @ Some(_)) => {
                        *k = None;
                        v(0)
                    }
                    _ => v(ERROR_INVALID_HANDLE),
                }
            }
            "RegQueryValueExW" => {
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let n = self.wstr(a[1])?;
                let (data, cb) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6));
                match self.hive().value(&path, &n).cloned() {
                    Ok(val) => {
                        let r = self.give_value(&val, a[3], data, cb);
                        v(r)
                    }
                    Err(_) => v(ERROR_FILE_NOT_FOUND),
                }
            }
            "RegGetValueW" => {
                // (hKey, subkey, value, flags, &type[5], data[6], &cb[7]); no type restriction filtering beyond existence.
                let Some(base) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let (sub, n) = (self.wstr(a[1])?, self.wstr(a[2])?);
                let (ty, data, cb) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6), self.stack_arg(rsp, 7));
                let path = join(&base, &sub);
                match self.hive().value(&path, &n).cloned() {
                    Ok(val) => {
                        let r = self.give_value(&val, ty, data, cb);
                        v(r)
                    }
                    Err(_) => v(ERROR_FILE_NOT_FOUND),
                }
            }
            "RegSetValueExW" => {
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let n = self.wstr(a[1])?;
                let (data, cb) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6) as usize);
                let bytes = if cb == 0 { Vec::new() } else { self.mem.read_vec(data, cb.min(1 << 20)).ok()? };
                match self.hive().set_value(&path, &n, from_raw(a[3] as u32), &bytes) {
                    Ok(()) => v(0),
                    Err(_) => v(ERROR_INVALID_PARAMETER),
                }
            }
            "RegDeleteValueW" => {
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let n = self.wstr(a[1])?;
                v(if self.hive().delete_value(&path, &n).is_ok() { 0 } else { ERROR_FILE_NOT_FOUND })
            }
            "RegDeleteKeyW" => {
                let Some(base) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let path = join(&base, &self.wstr(a[1])?);
                let Some((parent, leaf)) = path.rsplit_once('\\') else { return v(ERROR_INVALID_PARAMETER) };
                let (parent, leaf) = (parent.to_string(), leaf.to_string());
                match self.hive().key(&path) {
                    Ok(k) if !k.children.is_empty() => return v(5), // ERROR_ACCESS_DENIED: has subkeys
                    Err(_) => return v(ERROR_FILE_NOT_FOUND),
                    _ => {}
                }
                match self.hive().key_mut(&parent) {
                    Ok(p) => {
                        p.children.retain(|c| !c.name.eq_ignore_ascii_case(&leaf));
                        v(0)
                    }
                    Err(_) => v(ERROR_FILE_NOT_FOUND),
                }
            }
            "RegEnumKeyExW" => {
                // (hKey, index, name, &cch, reserved[5], ...)
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let nm = match self.hive().key(&path) {
                    Ok(k) => k.children.get(a[1] as usize).map(|c| c.name.clone()),
                    Err(_) => return v(ERROR_FILE_NOT_FOUND),
                };
                let Some(nm) = nm else { return v(ERROR_NO_MORE_ITEMS) };
                v(self.give_name(&nm, a[2], a[3]))
            }
            "RegEnumValueW" => {
                // (hKey, index, name, &cch, reserved[5], &type[6], data[7], &cb[8])
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let val = match self.hive().key(&path) {
                    Ok(k) => k.values.get(a[1] as usize).cloned(),
                    Err(_) => return v(ERROR_FILE_NOT_FOUND),
                };
                let Some(val) = val else { return v(ERROR_NO_MORE_ITEMS) };
                let r = self.give_name(&val.name, a[2], a[3]);
                if r != 0 {
                    return v(r);
                }
                let (ty, data, cb) = (self.stack_arg(rsp, 6), self.stack_arg(rsp, 7), self.stack_arg(rsp, 8));
                let r = self.give_value(&val, ty, data, cb);
                v(r)
            }
            "RegQueryInfoKeyW" => {
                // (hKey, class, &cchClass, reserved, &subKeys[5], &maxSubKeyLen[6], &maxClassLen[7], &values[8], &maxValueName[9]...)
                let Some(path) = self.key_path(a[0]) else { return v(ERROR_INVALID_HANDLE) };
                let (nk, nv, mk, mv): (u32, u32, u32, u32) = match self.hive().key(&path) {
                    Ok(k) => (
                        k.children.len() as u32,
                        k.values.len() as u32,
                        k.children.iter().map(|c| c.name.chars().count() as u32).max().unwrap_or(0),
                        k.values.iter().map(|x| x.name.chars().count() as u32).max().unwrap_or(0),
                    ),
                    Err(_) => return v(ERROR_FILE_NOT_FOUND),
                };
                for (slot, val) in [(5, nk), (6, mk), (8, nv), (9, mv)] {
                    let p = self.stack_arg(rsp, slot);
                    if p != 0 {
                        let _ = self.mem.write(p, &val.to_le_bytes());
                    }
                }
                v(0)
            }
            "RegFlushKey" => v(0),

            // ---- shlwapi ----
            "PathFindFileNameW" | "PathFindExtensionW" => {
                let s = self.wstr(a[0])?;
                let chars: Vec<char> = s.chars().collect();
                let units = |upto: usize| chars[..upto].iter().map(|c| c.len_utf16() as u64).sum::<u64>();
                let off = if name == "PathFindFileNameW" {
                    chars.iter().rposition(|c| *c == '\\' || *c == '/').map_or(0, |i| i + 1)
                } else {
                    let leaf = chars.iter().rposition(|c| *c == '\\' || *c == '/').map_or(0, |i| i + 1);
                    chars[leaf..].iter().rposition(|c| *c == '.').map_or(chars.len(), |i| leaf + i)
                };
                v(a[0] + 2 * units(off))
            }
            "PathCombineW" => {
                let (d, f) = (self.wstr(a[1])?, self.wstr(a[2])?);
                let abs = f.starts_with(['\\', '/']) || f.as_bytes().get(1) == Some(&b':');
                let out = if f.is_empty() {
                    d
                } else if abs || d.is_empty() {
                    f
                } else if d.ends_with(['\\', '/']) {
                    alloc::format!("{d}{f}")
                } else {
                    alloc::format!("{d}\\{f}")
                };
                if out.encode_utf16().count() >= 260 {
                    return v(0);
                }
                self.put_wide(a[0], &out);
                v(a[0])
            }
            "PathAddBackslashW" => {
                let s = self.wstr(a[0])?;
                let n = s.encode_utf16().count() as u64;
                if s.is_empty() || s.ends_with('\\') {
                    return v(a[0] + 2 * n);
                }
                if n >= 259 {
                    return v(0);
                }
                self.put_wide(a[0], &alloc::format!("{s}\\"));
                v(a[0] + 2 * (n + 1))
            }
            "PathRemoveFileSpecW" => {
                let s = self.wstr(a[0])?;
                let Some(i) = s.rfind(['\\', '/']) else {
                    let had = !s.is_empty();
                    self.put_wide(a[0], "");
                    return v(u64::from(had));
                };
                let keep = if i == 0 || s.as_bytes().get(i - 1) == Some(&b':') { i + 1 } else { i };
                let out = s[..keep].to_string();
                let changed = out.len() != s.len();
                self.put_wide(a[0], &out);
                v(u64::from(changed))
            }
            "PathIsRelativeW" => {
                let s = self.wstr(a[0])?;
                v(u64::from(!(s.starts_with(['\\', '/']) || s.as_bytes().get(1) == Some(&b':'))))
            }
            "PathFileExistsW" => {
                let n = files::norm(&self.wstr(a[0])?);
                let dir = alloc::format!("{n}/");
                v(u64::from(self.vfs.contains_key(&n) || self.vfs.keys().any(|k| k.starts_with(&dir))))
            }

            // ---- ole32 / oleaut32 ----
            "CoInitialize" | "OleInitialize" => {
                self.adv.com_init += 1;
                v(if self.adv.com_init > 1 { 1 } else { 0 }) // S_FALSE when already initialised
            }
            "OleUninitialize" => {
                self.adv.com_init = self.adv.com_init.saturating_sub(1);
                v(0)
            }
            "CoCreateInstance" => {
                let out = self.stack_arg(rsp, 5);
                self.put_out(out, 0);
                v(REGDB_E_CLASSNOTREG)
            }
            "CoTaskMemAlloc" => v(self.heap_alloc(a[0])),
            "CoTaskMemFree" => v(0),
            "StringFromGUID2" => {
                // (rguid, lpsz, cchMax) -> chars written including NUL (39), 0 when the buffer is too small.
                if a[2] < 39 {
                    return v(0);
                }
                let g = self.mem.read_vec(a[0], 16).ok()?;
                let s = guid_string(&g);
                self.put_wide(a[1], &s);
                v(39)
            }
            "CLSIDFromString" | "IIDFromString" => {
                let s = self.wstr(a[0])?;
                match parse_guid(&s) {
                    Some(g) => {
                        let _ = self.mem.write(a[1], &g);
                        v(0)
                    }
                    None => v(E_INVALIDARG),
                }
            }
            "CoCreateGuid" => {
                // No entropy source in the runtime: a counter mixed with the clock gives process-unique v4-shaped ids.
                self.adv.guids += 1;
                let mut x = now ^ (self.adv.guids << 32) ^ self.pid.rotate_left(17);
                let mut g = [0u8; 16];
                for c in g.chunks_mut(8) {
                    x = x.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(0x1234_5678_9abc_def1);
                    c.copy_from_slice(&(x ^ (x >> 29)).to_le_bytes());
                }
                g[7] = g[7] & 0x0f | 0x40;
                g[8] = g[8] & 0x3f | 0x80;
                let _ = self.mem.write(a[0], &g);
                v(0)
            }
            "SysAllocString" | "SysAllocStringLen" => {
                let n = if name == "SysAllocString" {
                    if a[0] == 0 {
                        return v(0);
                    }
                    self.wstr(a[0])?.encode_utf16().count()
                } else {
                    a[1] as usize
                };
                let blk = self.heap_alloc(4 + 2 * n as u64 + 2);
                if blk == 0 {
                    return v(0);
                }
                let mut b = Vec::from(((2 * n) as u32).to_le_bytes());
                if a[0] != 0 {
                    b.extend_from_slice(&self.mem.read_vec(a[0], 2 * n).ok()?);
                } else {
                    b.resize(4 + 2 * n, 0);
                }
                b.extend_from_slice(&[0, 0]);
                let _ = self.mem.write(blk, &b);
                v(blk + 4)
            }
            "SysFreeString" => v(0),
            "SysStringLen" => v(if a[0] == 0 { 0 } else { u64::from(self.mem.read_u32(a[0] - 4).unwrap_or(0) / 2) }),

            // ---- service control manager ----
            "OpenSCManagerW" => v(SCM_HANDLE),
            "CloseServiceHandle" => {
                if let Some(i) = a[0].checked_sub(SVC_BASE).map(|x| x as usize / 4) {
                    if let Some(s) = self.adv.svc_handles.get_mut(i) {
                        *s = None;
                    }
                }
                v(1)
            }
            "CreateServiceW" => {
                // (hSCM, name, display, access, type[5], start[6], errctl[7], path[8])
                if a[0] != SCM_HANDLE {
                    self.set_last_error(ERROR_INVALID_HANDLE as u32);
                    return v(0);
                }
                let n = self.wstr(a[1])?;
                let key = Self::svc_key(&n);
                if n.is_empty() || n.contains(['\\', '/']) {
                    self.set_last_error(ERROR_INVALID_PARAMETER as u32);
                    return v(0);
                }
                if self.hive().key(&key).is_ok() {
                    self.set_last_error(ERROR_SERVICE_EXISTS);
                    return v(0);
                }
                let (ty, start, path_p) = (self.stack_arg(rsp, 5) as u32, self.stack_arg(rsp, 6) as u32, self.stack_arg(rsp, 8));
                let path = self.wstr(path_p)?;
                let disp = self.wstr(a[2])?;
                let wide = |s: &str| {
                    let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
                    b.extend_from_slice(&[0, 0]);
                    b
                };
                let h = self.hive();
                let _ = h.set_value(&key, "Type", ValueType::Dword, &ty.to_le_bytes());
                let _ = h.set_value(&key, "Start", ValueType::Dword, &start.to_le_bytes());
                let _ = h.set_value(&key, "ImagePath", ValueType::ExpandSz, &wide(&path));
                let _ = h.set_value(&key, "DisplayName", ValueType::Sz, &wide(&if disp.is_empty() { n.clone() } else { disp }));
                self.adv.svc.insert(n.to_lowercase(), 1);
                self.adv.svc_handles.push(Some(n));
                v(SVC_BASE + (self.adv.svc_handles.len() as u64 - 1) * 4)
            }
            "OpenServiceW" => {
                let n = self.wstr(a[1])?;
                if self.hive().key(&Self::svc_key(&n)).is_err() {
                    self.set_last_error(ERROR_SERVICE_DOES_NOT_EXIST);
                    return v(0);
                }
                self.adv.svc_handles.push(Some(n));
                v(SVC_BASE + (self.adv.svc_handles.len() as u64 - 1) * 4)
            }
            "StartServiceW" | "ControlService" | "QueryServiceStatus" | "DeleteService" => {
                let Some(n) = self.svc_name(a[0]) else {
                    self.set_last_error(ERROR_INVALID_HANDLE as u32);
                    return v(0);
                };
                let id = n.to_lowercase();
                let cur = self.adv.svc.get(&id).copied().unwrap_or(1);
                match name {
                    "StartServiceW" => {
                        if cur == 4 {
                            self.set_last_error(ERROR_SERVICE_ALREADY_RUNNING);
                            return v(0);
                        }
                        self.adv.svc.insert(id, 4);
                    }
                    "ControlService" => {
                        // (h, code, SERVICE_STATUS*): 1 = STOP; 2/3 = PAUSE/CONTINUE; 4 = INTERROGATE.
                        if cur == 1 && a[1] != 4 {
                            self.set_last_error(ERROR_SERVICE_NOT_ACTIVE);
                            return v(0);
                        }
                        let new = match a[1] {
                            1 => 1,
                            2 => 7,
                            3 => 4,
                            _ => cur,
                        };
                        self.adv.svc.insert(id, new);
                        self.write_status(a[2], new);
                    }
                    "QueryServiceStatus" => self.write_status(a[1], cur),
                    _ => {
                        let key = Self::svc_key(&n);
                        if let Ok(p) = self.hive().key_mut(SERVICES_KEY) {
                            p.children.retain(|c| !c.name.eq_ignore_ascii_case(&n));
                        }
                        let _ = key;
                        self.adv.svc.remove(&id);
                    }
                }
                v(1)
            }
            _ => self.acct_syscall(name, a, rsp, now),
        }
    }

    fn give_name(&mut self, nm: &str, buf: u64, cch: u64) -> u64 {
        let cap = self.mem.read_u32(cch).unwrap_or(0) as usize;
        let n = nm.encode_utf16().count();
        if cap < n + 1 {
            return ERROR_MORE_DATA;
        }
        self.put_wide(buf, nm);
        let _ = self.mem.write(cch, &(n as u32).to_le_bytes());
        0
    }

    fn put_wide(&mut self, at: u64, s: &str) {
        let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend_from_slice(&[0, 0]);
        let _ = self.mem.write(at, &b);
    }

    fn write_status(&mut self, at: u64, state: u32) {
        if at == 0 {
            return;
        }
        let accepted = if state == 4 { 0x1 | 0x4 } else { 0 }; // STOP | SHUTDOWN
        let mut b = [0u8; 28];
        b[0..4].copy_from_slice(&0x10u32.to_le_bytes()); // SERVICE_WIN32_OWN_PROCESS
        b[4..8].copy_from_slice(&state.to_le_bytes());
        b[8..12].copy_from_slice(&(accepted as u32).to_le_bytes());
        let _ = self.mem.write(at, &b);
    }
}

/// `S-1-5-21-...` to the binary `SID` layout (revision, count, 6-byte big-endian authority, 32-bit sub-authorities).
fn sid_to_bytes(s: &str) -> Option<Vec<u8>> {
    let mut it = s.strip_prefix("S-")?.split('-');
    if it.next()? != "1" {
        return None;
    }
    let auth: u64 = it.next()?.parse().ok()?;
    let subs: Vec<u32> = it.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if subs.is_empty() || subs.len() > 15 || auth >> 48 != 0 {
        return None;
    }
    let mut b = alloc::vec![1u8, subs.len() as u8];
    b.extend_from_slice(&auth.to_be_bytes()[2..]);
    for x in subs {
        b.extend_from_slice(&x.to_le_bytes());
    }
    Some(b)
}

fn sid_from_bytes(b: &[u8]) -> Option<String> {
    if b.len() < 8 || b[0] != 1 || b[1] == 0 || b[1] > 15 || b.len() < 8 + 4 * usize::from(b[1]) {
        return None;
    }
    let auth = b[2..8].iter().fold(0u64, |a, x| a << 8 | u64::from(*x));
    let mut s = alloc::format!("S-1-{auth}");
    for i in 0..usize::from(b[1]) {
        s += &alloc::format!("-{}", u32::from_le_bytes(b[8 + 4 * i..12 + 4 * i].try_into().ok()?));
    }
    Some(s)
}

fn integrity_sid(i: Integrity) -> String {
    alloc::format!("S-1-16-{}", match i { Integrity::Low => 4096, Integrity::Medium => 8192, Integrity::High => 12288, Integrity::System => 16384 })
}

impl Process {
    /// Installs the machine's account database and the user this process runs as (the kernel does this at start-up).
    pub fn set_accounts(&mut self, sam: Sam, lsa: Lsa, rid: u32) {
        let token = lsa.token_for(&sam, rid, self.session).map(|t| if t.is_admin() { t.filtered() } else { t });
        self.adv.tokens = token.into_iter().collect();
        self.adv.rid = rid;
        self.adv.accounts = Some((sam, lsa));
    }

    /// Account database; without one installed a throw-away default ("User" in Administrators, blank password, boot
    /// key of zeros) is created so programs that query their identity still work.
    fn accounts(&mut self) -> &mut (Sam, Lsa) {
        if self.adv.accounts.is_none() {
            let mut sam = Sam::new("LOOS", "S-1-5-21-1000-2000-3000", 1);
            sam.policy.min_len = 0;
            let rid = sam.create_user("User", b"", [0; 16], 0).unwrap_or(1000);
            if let Some(u) = sam.user(rid).map(|u| u.account.sid.clone()) {
                let _ = sam.add_member(SID_ADMINS, &u);
            }
            sam.policy.min_len = 8;
            let lsa = Lsa::new(&[0; 32]);
            self.set_accounts(sam, lsa, rid);
        }
        self.adv.accounts.as_mut().unwrap()
    }

    /// Token for a handle (`0`/pseudo handles mean the process token).
    fn token_of(&mut self, h: u64) -> Option<Token> {
        self.accounts();
        let i = if h == 0 || h == u64::MAX || h == u64::MAX - 1 { 0 } else { (h.checked_sub(TOKEN_BASE)? / 4) as usize };
        self.adv.tokens.get(i).cloned()
    }

    /// Two-call string output: writes `s` plus NUL when `cap` allows, else reports the needed size (in characters,
    /// including NUL) and fails with ERROR_INSUFFICIENT_BUFFER.
    fn out_wide(&mut self, s: &str, buf: u64, cch: u64) -> bool {
        let n = s.encode_utf16().count() as u32 + 1;
        let cap = self.mem.read_u32(cch).unwrap_or(0);
        if cap < n || buf == 0 {
            let _ = self.mem.write(cch, &n.to_le_bytes());
            self.set_last_error(ERROR_INSUFFICIENT_BUFFER);
            return false;
        }
        self.put_wide(buf, s);
        let _ = self.mem.write(cch, &(n - 1).to_le_bytes());
        true
    }

    fn token_info(&mut self, t: &Token, class: u32) -> Option<Vec<u8>> {
        // Every SID lives after the fixed part; pointers inside the structure are patched to the caller's buffer later
        // by `place`, so build with offsets relative to the start.
        let sid = |s: &str| sid_to_bytes(s).unwrap_or_default();
        match class {
            1 => Some(Self::sid_attr_list(&[(&t.user, 0)], false)),
            2 => {
                let mut l: Vec<(&str, u32)> = t.groups.iter().map(|g| (g.as_str(), 0x7)).collect();
                l.extend(t.deny_only.iter().map(|g| (g.as_str(), 0x10)));
                Some(Self::sid_attr_list(&l, true))
            }
            3 => {
                let mut o = Vec::from((PRIVILEGES.iter().filter(|p| t.privileges & p.1 != 0).count() as u32).to_le_bytes());
                for (i, (_, bit)) in PRIVILEGES.iter().enumerate() {
                    if t.privileges & bit != 0 {
                        o.extend_from_slice(&(i as u32 + 2).to_le_bytes()); // LUID low
                        o.extend_from_slice(&0u32.to_le_bytes()); // LUID high
                        o.extend_from_slice(&(if *bit == loos_security::uac::privs::CHANGE_NOTIFY { 3u32 } else { 0 }).to_le_bytes());
                    }
                }
                Some(o)
            }
            8 => Some(1u32.to_le_bytes().to_vec()), // TokenPrimary
            12 => Some(t.session.to_le_bytes().to_vec()),
            18 => Some((if t.elevated { 2u32 } else if t.deny_only.iter().any(|g| g == SID_ADMINS) { 3 } else { 1 }).to_le_bytes().to_vec()),
            20 => Some(u32::from(t.elevated).to_le_bytes().to_vec()),
            25 => {
                let _ = sid;
                Some(Self::sid_attr_list(&[(&integrity_sid(t.integrity), 0x20)], false))
            }
            _ => None,
        }
    }

    /// `TOKEN_USER` / `TOKEN_GROUPS` / `TOKEN_MANDATORY_LABEL` layout with self-relative SID pointers (offsets stored in
    /// the pointer slots; [`Process::place`] turns them into addresses).
    fn sid_attr_list(l: &[(&str, u32)], counted: bool) -> Vec<u8> {
        let head = if counted { 8 } else { 0 };
        let mut o = Vec::new();
        if counted {
            o.extend_from_slice(&(l.len() as u32).to_le_bytes());
            o.extend_from_slice(&[0; 4]);
        }
        let mut sids = Vec::new();
        let tail = head + 16 * l.len();
        for (s, attr) in l {
            let off = tail + sids.len();
            let b = sid_to_bytes(s).unwrap_or_default();
            sids.extend_from_slice(&b);
            while sids.len() % 4 != 0 {
                sids.push(0);
            }
            o.extend_from_slice(&(off as u64).to_le_bytes());
            o.extend_from_slice(&attr.to_le_bytes());
            o.extend_from_slice(&[0; 4]);
        }
        o.extend_from_slice(&sids);
        o
    }

    /// Turns the offsets in `SID_AND_ATTRIBUTES` pointer slots into addresses for a buffer placed at `addr`.
    fn place(b: &mut [u8], class: u32, addr: u64) {
        let (start, n) = match class {
            1 | 25 => (0usize, 1usize),
            2 => (8, u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize),
            _ => return,
        };
        for i in 0..n {
            let at = start + 16 * i;
            let off = u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
            b[at..at + 8].copy_from_slice(&(addr + off).to_le_bytes());
        }
    }

    /// Account and token calls; `None` for names outside this group.
    fn acct_syscall(&mut self, name: &str, a: [u64; 4], rsp: u64, now: u64) -> Option<Ret> {
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "LookupAccountNameW" => {
                // (system, name, Sid, &cbSid, domain[5], &cchDomain[6], &use[7])
                let n = self.wstr(a[1])?;
                let (dom, cchdom, usep) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6), self.stack_arg(rsp, 7));
                let (sam, _) = self.accounts();
                let Some((sid, ty)) = sam.lookup_name(&n) else {
                    self.set_last_error(ERROR_NONE_MAPPED);
                    return v(0);
                };
                let domain = sam.lookup_sid(&sid).map(|x| x.1).unwrap_or_default();
                let bytes = sid_to_bytes(&sid)?;
                let cap = self.mem.read_u32(a[3]).unwrap_or(0) as usize;
                let dn = domain.encode_utf16().count() as u32 + 1;
                let dcap = if cchdom != 0 { self.mem.read_u32(cchdom).unwrap_or(0) } else { 0 };
                let _ = self.mem.write(a[3], &(bytes.len() as u32).to_le_bytes());
                if cap < bytes.len() || dcap < dn {
                    if cchdom != 0 {
                        let _ = self.mem.write(cchdom, &dn.to_le_bytes());
                    }
                    self.set_last_error(ERROR_INSUFFICIENT_BUFFER);
                    return v(0);
                }
                let _ = self.mem.write(a[2], &bytes);
                self.put_wide(dom, &domain);
                let _ = self.mem.write(cchdom, &(dn - 1).to_le_bytes());
                if usep != 0 {
                    let _ = self.mem.write(usep, &(ty as u32).to_le_bytes());
                }
                v(1)
            }
            "LookupAccountSidW" => {
                // (system, Sid, name, &cchName, domain[5], &cchDomain[6], &use[7])
                let (dom, cchdom, usep) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6), self.stack_arg(rsp, 7));
                let raw = self.mem.read_vec(a[1], 8).ok()?;
                let len = 8 + 4 * usize::from(raw.get(1).copied().unwrap_or(0));
                let sid = sid_from_bytes(&self.mem.read_vec(a[1], len).ok()?);
                let found = sid.and_then(|s| self.accounts().0.lookup_sid(&s));
                let Some((nm, domain, ty)) = found else {
                    self.set_last_error(ERROR_NONE_MAPPED);
                    return v(0);
                };
                let ok1 = self.out_wide(&nm, a[2], a[3]);
                let ok2 = self.out_wide(&domain, dom, cchdom);
                if usep != 0 && ok1 && ok2 {
                    let _ = self.mem.write(usep, &(ty as u32).to_le_bytes());
                }
                v(u64::from(ok1 && ok2))
            }
            "ConvertSidToStringSidW" => {
                let raw = self.mem.read_vec(a[0], 8).ok()?;
                let len = 8 + 4 * usize::from(raw.get(1).copied().unwrap_or(0));
                let Some(s) = sid_from_bytes(&self.mem.read_vec(a[0], len).ok()?) else {
                    self.set_last_error(1337); // ERROR_INVALID_SID
                    return v(0);
                };
                let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
                b.extend_from_slice(&[0, 0]);
                let p = self.heap_alloc(b.len() as u64);
                let _ = self.mem.write(p, &b);
                self.put_out(a[1], p);
                v(1)
            }
            "ConvertStringSidToSidW" => {
                let s = self.wstr(a[0])?;
                let Some(b) = sid_to_bytes(&s) else {
                    self.set_last_error(1337);
                    return v(0);
                };
                let p = self.heap_alloc(b.len() as u64);
                let _ = self.mem.write(p, &b);
                self.put_out(a[1], p);
                v(1)
            }
            "GetUserNameW" => {
                let rid = {
                    self.accounts();
                    self.adv.rid
                };
                let nm = self.accounts().0.user(rid).map(|u| u.account.name.clone()).unwrap_or_default();
                v(u64::from(self.out_wide(&nm, a[0], a[1])))
            }
            "GetComputerNameW" => {
                let nm = String::from("LOOS");
                let ok = self.out_wide(&nm, a[0], a[1]);
                v(u64::from(ok))
            }
            "OpenProcessToken" => {
                self.accounts();
                if self.adv.tokens.is_empty() {
                    self.set_last_error(5);
                    return v(0);
                }
                self.put_out(a[2], TOKEN_BASE);
                v(1)
            }
            "OpenThreadToken" => {
                self.set_last_error(1008); // ERROR_NO_TOKEN: threads do not impersonate
                v(0)
            }
            "GetTokenInformation" => {
                // (token, class, buf, len, &retlen[5])
                let Some(t) = self.token_of(a[0]) else {
                    self.set_last_error(ERROR_INVALID_HANDLE as u32);
                    return v(0);
                };
                let rl = self.stack_arg(rsp, 5);
                let Some(mut info) = self.token_info(&t, a[1] as u32) else {
                    self.set_last_error(ERROR_INVALID_PARAMETER as u32);
                    return v(0);
                };
                if rl != 0 {
                    let _ = self.mem.write(rl, &(info.len() as u32).to_le_bytes());
                }
                if a[2] == 0 || (a[3] as usize) < info.len() {
                    self.set_last_error(ERROR_INSUFFICIENT_BUFFER);
                    return v(0);
                }
                Self::place(&mut info, a[1] as u32, a[2]);
                v(u64::from(self.mem.write(a[2], &info).is_ok()))
            }
            "LogonUserW" => {
                // (user, domain, password, type[4], provider[5], &token[6]); interactive (2) and network (3) types.
                let (user, pw) = (self.wstr(a[0])?, self.wstr(a[2])?);
                let (ty, out) = (a[3] as u32, self.stack_arg(rsp, 6));
                let sec = now / 1000;
                let session = self.session;
                let auth = self.accounts().0.authenticate(&user, pw.as_bytes(), sec);
                let Ok(rid) = auth else {
                    self.set_last_error(1326); // ERROR_LOGON_FAILURE
                    return v(0);
                };
                let (sam, lsa) = self.accounts();
                let tok = match ty {
                    2 | 3 | 7 | 8 | 9 => lsa.token_for(sam, rid, session),
                    _ => Err(loos_security::Error::Unsupported),
                };
                match tok {
                    Ok(t) => {
                        self.adv.tokens.push(t);
                        let h = TOKEN_BASE + 4 * (self.adv.tokens.len() as u64 - 1);
                        self.put_out(out, h);
                        v(1)
                    }
                    Err(_) => {
                        self.set_last_error(1385); // ERROR_LOGON_TYPE_NOT_GRANTED
                        v(0)
                    }
                }
            }
            "LookupPrivilegeValueW" => {
                let n = self.wstr(a[1])?;
                match PRIVILEGES.iter().position(|p| p.0.eq_ignore_ascii_case(&n)) {
                    Some(i) => {
                        let _ = self.mem.write(a[2], &(i as u64 + 2).to_le_bytes());
                        v(1)
                    }
                    None => {
                        self.set_last_error(1313); // ERROR_NO_SUCH_PRIVILEGE
                        v(0)
                    }
                }
            }
            "LookupPrivilegeNameW" => {
                // (system, &LUID, name, &cch)
                let luid = self.mem.read_u64(a[1]).ok()?;
                match luid.checked_sub(2).and_then(|i| PRIVILEGES.get(i as usize)) {
                    Some(p) => v(u64::from(self.out_wide(p.0, a[2], a[3]))),
                    None => {
                        self.set_last_error(1313);
                        v(0)
                    }
                }
            }
            "IsValidSid" => {
                let ok = self.mem.read_vec(a[0], 8).ok().is_some_and(|r| {
                    let n = usize::from(r[1]);
                    r[0] == 1 && (1..=15).contains(&n) && self.mem.read_vec(a[0], 8 + 4 * n).is_ok()
                });
                v(u64::from(ok))
            }
            "GetLengthSid" => v(self.mem.read_vec(a[0], 2).map_or(0, |r| 8 + 4 * u64::from(r[1]))),
            "EqualSid" => {
                let rd = |p: &mut Process, at: u64| {
                    let r = p.mem.read_vec(at, 2).ok()?;
                    p.mem.read_vec(at, 8 + 4 * usize::from(r[1])).ok()
                };
                v(u64::from(matches!((rd(self, a[0]), rd(self, a[1])), (Some(x), Some(y)) if x == y)))
            }
            "IsUserAnAdmin" => {
                let t = self.token_of(0);
                v(u64::from(t.is_some_and(|t| t.is_admin() && t.elevated)))
            }
            "CheckTokenMembership" => {
                let raw = self.mem.read_vec(a[1], 8).ok()?;
                let sid = sid_from_bytes(&self.mem.read_vec(a[1], 8 + 4 * usize::from(raw[1])).ok()?)?;
                let Some(t) = self.token_of(a[0]) else {
                    self.set_last_error(ERROR_INVALID_HANDLE as u32);
                    return v(0);
                };
                let member = t.user == sid || t.groups.iter().any(|g| *g == sid);
                let _ = self.mem.write(a[2], &u32::from(member).to_le_bytes());
                v(1)
            }
            _ => None,
        }
    }
}

fn guid_string(g: &[u8]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let (d2, d3) = (u16::from_le_bytes([g[4], g[5]]), u16::from_le_bytes([g[6], g[7]]));
    alloc::format!(
        "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let t = s.strip_prefix('{')?.strip_suffix('}')?;
    let parts: Vec<&str> = t.split('-').collect();
    if parts.len() != 5 || [8, 4, 4, 4, 12].iter().zip(&parts).any(|(l, p)| p.len() != *l) {
        return None;
    }
    let mut g = [0u8; 16];
    g[0..4].copy_from_slice(&u32::from_str_radix(parts[0], 16).ok()?.to_le_bytes());
    g[4..6].copy_from_slice(&u16::from_str_radix(parts[1], 16).ok()?.to_le_bytes());
    g[6..8].copy_from_slice(&u16::from_str_radix(parts[2], 16).ok()?.to_le_bytes());
    let tail = alloc::format!("{}{}", parts[3], parts[4]);
    for i in 0..8 {
        g[8 + i] = u8::from_str_radix(&tail[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(g)
}

