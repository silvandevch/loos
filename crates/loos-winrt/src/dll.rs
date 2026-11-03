//! Run-time DLL loading: `LoadLibrary`/`GetProcAddress`/`FreeLibrary` over the guest file set, with relocation,
//! import resolution (built-in kernel32/user32/... stubs or other loaded DLLs) and `DllMain` attach calls.

use crate::{api, pe, Process};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use loos_lin::mem::{ceil, PROT_EXEC, PROT_READ};

/// DLLs provided by the runtime itself (their functions are syscall stubs).
const BUILTIN: &[&str] = &[
    "kernel32.dll", "kernelbase.dll", "ntdll.dll", "user32.dll", "gdi32.dll", "advapi32.dll", "shell32.dll", "comctl32.dll",
    "comdlg32.dll", "ole32.dll", "uxtheme.dll", "winspool.drv", "msvcrt.dll", "ucrtbase.dll", "win32u.dll", "ws2_32.dll",
];
const BUILTIN_BASE: u64 = 0x7ff0_0000;
const DLL_AREA: u64 = 0x1_8000_0000;
const STUB_AREA: u64 = 0x10000;

/// A DLL mapped into the process.
pub struct Module {
    /// Lower-case file name.
    pub name: String,
    /// Load address (the module handle).
    pub base: u64,
    /// Exports by name.
    pub names: BTreeMap<String, u64>,
    /// Exports by ordinal.
    pub ords: BTreeMap<u32, u64>,
}

fn base_name(name: &str) -> String {
    let n = name.rsplit(['\\', '/']).next().unwrap_or(name).to_lowercase();
    if n.contains('.') { n } else { alloc::format!("{n}.dll") }
}

impl Process {
    /// Adds a file to the guest file set (path as the guest will see it, any separator).
    pub fn add_file(&mut self, path: &str, data: &[u8]) {
        self.vfs.insert(crate::files::norm(path), data.to_vec());
    }

    /// Address of a runtime stub that performs syscall `nr` (a function pointer for `GetProcAddress`/DLL imports).
    pub(crate) fn dyn_stub(&mut self, nr: u32) -> u64 {
        if let Some(&a) = self.stub_cache.get(&nr) {
            return a;
        }
        if self.stub_next == 0 {
            self.stub_next = self.mem.map_anywhere(STUB_AREA, PROT_READ | PROT_EXEC, &[], "stubs2").unwrap_or(0);
            self.stub_end = self.stub_next + STUB_AREA;
        }
        if self.stub_next == 0 || self.stub_next + pe::STUB as u64 > self.stub_end {
            return 0;
        }
        let a = self.stub_next;
        self.stub_next += pe::STUB as u64;
        let _ = self.mem.write(a, &pe::stub(nr));
        self.stub_cache.insert(nr, a);
        a
    }

    fn builtin_proc(&mut self, dll: &str, name: Option<&str>, ord: u32) -> u64 {
        let nr = match name.and_then(api::lookup) {
            Some(nr) => nr,
            None => {
                self.unresolved.push(alloc::format!("{dll}!{}", name.map_or(alloc::format!("#{ord}"), |n| n.to_string())));
                api::UNRESOLVED_BASE + self.unresolved.len() as u32 - 1
            }
        };
        self.dyn_stub(nr)
    }

    /// Module handle for `name` if already available (built-in pseudo handle or mapped DLL).
    pub fn module_handle(&self, name: &str) -> Option<u64> {
        let n = base_name(name);
        if let Some(i) = BUILTIN.iter().position(|b| *b == n) {
            return Some(BUILTIN_BASE + i as u64 * 0x1000);
        }
        self.modules.iter().find(|m| m.name == n).map(|m| m.base)
    }

    /// `LoadLibrary`: returns the module handle, or `None` (last error set). Newly mapped modules queue their
    /// `DllMain` in [`Process::pending_init`] (dependencies first).
    pub fn load_library(&mut self, name: &str) -> Option<u64> {
        if let Some(h) = self.module_handle(name) {
            return Some(h);
        }
        let n = base_name(name);
        let suffix = alloc::format!("/{n}");
        let data = self.vfs.iter().find(|(k, _)| k.ends_with(&suffix) || **k == suffix[1..]).map(|(_, v)| v.clone());
        let Some(data) = data else {
            self.set_last_error(126);
            return None;
        };
        let st = match pe::stage(&data) {
            Ok(s) => s,
            Err(_) => {
                self.set_last_error(193);
                return None;
            }
        };
        let base = self.next_dll_base;
        self.next_dll_base += ceil(st.size as u64) + 0x1_0000;
        let imports = st.imports().ok()?;
        let mut patches = Vec::new();
        for imp in &imports {
            let dll = base_name(&imp.dll);
            let addr = if BUILTIN.contains(&dll.as_str()) {
                self.builtin_proc(&dll, imp.name.as_deref(), imp.ord)
            } else if let Some(h) = self.load_library(&dll) {
                self.module_proc(h, imp.name.as_deref(), imp.ord)
            } else {
                0
            };
            let addr = if addr == 0 { self.builtin_proc(&dll, imp.name.as_deref(), imp.ord) } else { addr };
            patches.push((imp.slot, addr));
        }
        let exports = st.exports().ok()?;
        let entry = st.entry_rva;
        let (pd, pdsize, imgsize) = (st.pdata(), st.size, st.size as u64);
        if st.map(&mut self.mem, base, &patches).is_err() {
            self.set_last_error(8);
            return None;
        }
        self.register_image(base, imgsize, pd.0, pd.1);
        let _ = pdsize;
        let mut m = Module { name: n, base, names: BTreeMap::new(), ords: BTreeMap::new() };
        for e in exports {
            m.ords.insert(e.ord, base + u64::from(e.rva));
            if let Some(nm) = e.name {
                m.names.insert(nm, base + u64::from(e.rva));
            }
        }
        self.modules.push(m);
        if entry != 0 {
            self.pending_init.push((base + entry, base));
        }
        Some(base)
    }

    fn module_proc(&self, h: u64, name: Option<&str>, ord: u32) -> u64 {
        let Some(m) = self.modules.iter().find(|m| m.base == h) else { return 0 };
        match name {
            Some(n) => m.names.get(n).copied().unwrap_or(0),
            None => m.ords.get(&ord).copied().unwrap_or(0),
        }
    }

    /// `GetProcAddress`.
    pub fn proc_address(&mut self, h: u64, name: Option<&str>, ord: u32) -> u64 {
        let r = if (BUILTIN_BASE..BUILTIN_BASE + BUILTIN.len() as u64 * 0x1000).contains(&h) {
            match name.and_then(api::lookup) {
                Some(nr) => self.dyn_stub(nr),
                None => 0,
            }
        } else {
            self.module_proc(h, name, ord)
        };
        if r == 0 {
            self.set_last_error(127);
        }
        r
    }

    /// `FreeLibrary` (modules stay mapped: there is no unmap yet).
    pub fn free_library(&mut self, h: u64) -> bool {
        h != 0
    }

    /// Next queued `DllMain` as `(entry address, module base)`.
    pub fn next_init(&mut self) -> Option<(u64, u64)> {
        if self.pending_init.is_empty() { None } else { Some(self.pending_init.remove(0)) }
    }

    /// First free DLL base (exposed for tests).
    pub fn dll_area() -> u64 {
        DLL_AREA
    }
}
