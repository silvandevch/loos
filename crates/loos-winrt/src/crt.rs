//! C runtime (`ucrtbase`/`msvcrt`) services: start-up and exit, heap, `mem*`/`str*`/`wcs*`, ctype, number conversion,
//! environment, and the data symbols CRT-linked programs import. Stdio and printf live in [`crate::cio`].
//!
//! Limits: `atexit` handlers are recorded but not run at exit, `qsort`/`bsearch` and the floating-point math functions
//! are not provided (callbacks and XMM return values need kernel support).

use crate::{Process, Ret};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// First syscall number of this table.
pub const CRT_BASE: u32 = 0x2000;

/// Data symbols programs import by name (the IAT slot must hold the variable's address) and their sizes.
pub const DATA: &[(&str, u64)] = &[
    ("_fmode", 4), ("_commode", 4), ("__initenv", 8), ("__winitenv", 8), ("_environ", 8), ("_wenviron", 8), ("_acmdln", 8),
    ("_wcmdln", 8), ("_pgmptr", 8), ("_wpgmptr", 8), ("_iob", 144), ("_timezone", 8), ("_daylight", 4), ("_tzname", 16),
    ("__argc", 4), ("__argv", 8), ("__wargv", 8), ("_osver", 4), ("_winver", 4), ("_winmajor", 4), ("_winminor", 4),
];

/// Size of an importable data symbol.
pub fn data_size(name: &str) -> Option<u64> {
    DATA.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Writable page(s) holding data symbols and runtime cells.
pub struct DataArena {
    base: u64,
    next: u64,
    end: u64,
    syms: BTreeMap<String, u64>,
}

impl DataArena {
    /// Arena over `[base, base+len)` (already mapped read/write, zeroed).
    pub fn new(base: u64, len: u64) -> Self {
        DataArena { base, next: base, end: base + len, syms: BTreeMap::new() }
    }

    /// Address of the named cell, allocating `size` zeroed bytes on first use.
    pub fn cell(&mut self, name: &str, size: u64) -> Option<u64> {
        if let Some(a) = self.syms.get(name) {
            return Some(*a);
        }
        let a = (self.next + 15) & !15;
        if self.base == 0 || a + size > self.end {
            return None;
        }
        self.next = a + size;
        self.syms.insert(name.to_string(), a);
        Some(a)
    }
}

/// Per-process C runtime state.
#[derive(Default)]
pub struct State {
    free: BTreeMap<u64, Vec<u64>>,
    pub(crate) atexit: Vec<u64>,
    pub(crate) args_built: bool,
    pub(crate) env: Option<BTreeMap<String, String>>,
    seed: u32,
    pub(crate) files: BTreeMap<u64, crate::cio::CFile>,
    pub(crate) next_fd: i32,
    tok: u64,
}

const NAMES: &[&str] = &[
    // start-up / exit
    "__set_app_type", "_set_app_type", "__getmainargs", "__wgetmainargs", "_configure_narrow_argv", "_configure_wide_argv",
    "__p___argc", "__p___argv", "__p___wargv", "_get_initial_narrow_environment", "_get_initial_wide_environment",
    "_initialize_narrow_environment", "_initialize_wide_environment", "_initterm", "_initterm_e", "_crt_atexit", "atexit",
    "_onexit", "_cexit", "_c_exit", "exit", "_exit", "_Exit", "abort", "_amsg_exit", "__setusermatherr", "_set_fmode",
    "_set_commode", "__p__fmode", "__p__commode", "_set_new_mode", "_configthreadlocale", "_set_invalid_parameter_handler",
    "_invalid_parameter_noinfo", "_invalid_parameter_noinfo_noreturn", "_controlfp_s", "_lock", "_unlock", "__lock", "__unlock",
    "signal", "raise", "_seh_filter_exe", "_crt_at_quick_exit", "_register_onexit_function", "_initialize_onexit_table",
    "_execute_onexit_table", "_errno", "__doserrno", "_get_errno", "_set_errno", "strerror", "setlocale", "localeconv",
    "___mb_cur_max_func", "___lc_codepage_func", "__mb_cur_max", "_get_osfhandle", "clock", "_set_abort_behavior",
    "__p__pgmptr", "__p__environ", "__p__wenviron", "_get_pgmptr", "_get_wpgmptr",
    // heap
    "malloc", "calloc", "realloc", "free", "_msize", "_aligned_malloc", "_aligned_free", "_recalloc", "_expand",
    // memory / strings
    "memcpy", "memmove", "memset", "memcmp", "memchr", "_memicmp", "memcpy_s", "memmove_s", "strlen", "strnlen", "strcmp",
    "strncmp", "_stricmp", "_strnicmp", "stricmp", "strcasecmp", "strncasecmp", "strchr", "strrchr", "strstr", "strcpy",
    "strncpy", "strcat", "strncat", "strcspn", "strspn", "strpbrk", "strdup", "_strdup", "strtok", "_strlwr", "_strupr",
    "strcpy_s", "strcat_s", "strncpy_s", "wcslen", "wcsnlen", "wcscmp", "wcsncmp", "_wcsicmp", "_wcsnicmp", "wcschr",
    "wcsrchr", "wcsstr", "wcscpy", "wcsncpy", "wcscat", "wcsncat", "wcscspn", "wcsspn", "wcspbrk", "_wcsdup", "wcsdup",
    "_wcsupr", "_wcslwr", "wcscpy_s", "wcscat_s", "towupper", "towlower", "toupper", "tolower", "_toupper", "_tolower",
    // ctype
    "isalpha", "isdigit", "isalnum", "isspace", "isupper", "islower", "isprint", "ispunct", "isxdigit", "iscntrl", "isgraph",
    "isascii", "iswalpha", "iswdigit", "iswalnum", "iswspace", "iswupper", "iswlower", "iswprint", "iswpunct", "iswxdigit",
    "iswcntrl", "iswgraph",
    // conversion
    "atoi", "atol", "atoll", "_atoi64", "_wtoi", "_wtol", "_wtoi64", "strtol", "strtoul", "strtoll", "strtoull", "_strtoi64",
    "_strtoui64", "wcstol", "wcstoul", "wcstoll", "wcstoull", "_itoa", "_ltoa", "_ultoa", "_i64toa", "_ui64toa", "_itow",
    "_ltow", "_ultow", "_i64tow", "_ui64tow", "abs", "labs", "llabs", "_abs64", "rand", "srand", "atof", "strtod", "_atof_l",
    // environment
    "getenv", "_wgetenv", "_putenv", "_wputenv", "_putenv_s", "getenv_s",
];

/// Syscall number of a C-runtime import (stdio names come from [`crate::cio`]).
pub fn lookup(name: &str) -> Option<u32> {
    if let Some(i) = NAMES.iter().position(|n| *n == name) {
        return Some(CRT_BASE + i as u32);
    }
    crate::cio::lookup(name)
}

/// Default environment (what a fresh session sees).
fn default_env() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for (k, v) in [
        ("SystemRoot", "C:\\Windows"), ("windir", "C:\\Windows"), ("SystemDrive", "C:"), ("TEMP", "C:\\Windows\\Temp"),
        ("TMP", "C:\\Windows\\Temp"), ("PATH", "C:\\Windows\\System32;C:\\Windows"), ("COMPUTERNAME", "LOOS"),
        ("OS", "Windows_NT"), ("NUMBER_OF_PROCESSORS", "1"), ("PROCESSOR_ARCHITECTURE", "AMD64"), ("USERNAME", "User"),
        ("USERPROFILE", "C:\\Users\\User"), ("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\User"), ("ComSpec", "C:\\Windows\\System32\\cmd.exe"),
        ("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
    ] {
        m.insert(k.to_ascii_uppercase(), alloc::format!("{k}={v}"));
    }
    m
}

fn is_space(c: u32) -> bool {
    matches!(c, 9..=13 | 32)
}

fn lower(c: u32) -> u32 {
    if (0x41..=0x5a).contains(&c) { c + 32 } else if c > 0x7f { char::from_u32(c).and_then(|x| x.to_lowercase().next()).map_or(c, |x| x as u32) } else { c }
}
fn upper(c: u32) -> u32 {
    if (0x61..=0x7a).contains(&c) { c - 32 } else if c > 0x7f { char::from_u32(c).and_then(|x| x.to_uppercase().next()).filter(|_| true).map_or(c, |x| x as u32) } else { c }
}

/// Parses the integer in `s` (C `strtol` rules); returns `(value as u64 two's complement, negative, consumed, overflow)`.
fn parse_int(s: &[u32], mut base: u32) -> (u64, bool, usize, bool) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == 0x2b || s[i] == 0x2d) {
        neg = s[i] == 0x2d;
        i += 1;
    }
    let has0x = s.get(i) == Some(&0x30) && matches!(s.get(i + 1), Some(0x78 | 0x58));
    if (base == 0 || base == 16) && has0x && s.get(i + 2).and_then(|c| char::from_u32(*c)).is_some_and(|c| c.is_ascii_hexdigit()) {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if s.get(i) == Some(&0x30) { 8 } else { 10 };
    }
    let (mut v, mut any, mut over) = (0u64, false, false);
    while let Some(d) = s.get(i).and_then(|c| char::from_u32(*c)).and_then(|c| c.to_digit(base.clamp(2, 36))) {
        match v.checked_mul(u64::from(base)).and_then(|x| x.checked_add(u64::from(d))) {
            Some(n) => v = n,
            None => over = true,
        }
        any = true;
        i += 1;
    }
    if !any {
        return (0, false, 0, false);
    }
    (v, neg, i, over)
}

/// Parses a decimal floating-point number; returns `(value, consumed)`.
fn parse_float(s: &[u32]) -> (f64, usize) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let start = i;
    let neg = s.get(i) == Some(&0x2d);
    if matches!(s.get(i), Some(0x2b | 0x2d)) {
        i += 1;
    }
    let dig = |c: Option<&u32>| c.and_then(|c| char::from_u32(*c)).and_then(|c| c.to_digit(10));
    let (mut m, mut scale, mut any) = (0f64, 0i32, false);
    while let Some(d) = dig(s.get(i)) {
        m = m * 10.0 + f64::from(d);
        any = true;
        i += 1;
    }
    if s.get(i) == Some(&0x2e) {
        i += 1;
        while let Some(d) = dig(s.get(i)) {
            m = m * 10.0 + f64::from(d);
            scale -= 1;
            any = true;
            i += 1;
        }
    }
    if !any {
        return (0.0, 0);
    }
    if matches!(s.get(i), Some(0x65 | 0x45)) {
        let mut j = i + 1;
        let eneg = s.get(j) == Some(&0x2d);
        if matches!(s.get(j), Some(0x2b | 0x2d)) {
            j += 1;
        }
        if dig(s.get(j)).is_some() {
            let mut e = 0i32;
            while let Some(d) = dig(s.get(j)) {
                e = (e * 10 + d as i32).min(10_000);
                j += 1;
            }
            scale += if eneg { -e } else { e };
            i = j;
        }
    }
    let mut v = m;
    let mut sc = scale;
    while sc > 0 && v.is_finite() {
        v *= 10.0;
        sc -= 1;
    }
    while sc < 0 && v != 0.0 {
        v /= 10.0;
        sc += 1;
    }
    let _ = start;
    (if neg { -v } else { v }, i)
}

impl Process {
    /// Reads a NUL-terminated string of 8- or 16-bit units (at most `max`).
    pub(crate) fn rd_z(&mut self, mut p: u64, wide: bool, max: usize) -> Vec<u32> {
        let mut out = Vec::new();
        let unit = if wide { 2 } else { 1 };
        while out.len() < max {
            let n = (4096 - (p & 4095)) as usize;
            let Ok(b) = self.mem.read_vec(p, n) else { break };
            for c in b.chunks_exact(unit) {
                let v = if wide { u32::from(u16::from_le_bytes([c[0], c[1]])) } else { u32::from(c[0]) };
                if v == 0 || out.len() >= max {
                    return out;
                }
                out.push(v);
            }
            p += n as u64;
        }
        out
    }

    pub(crate) fn wr_z(&mut self, at: u64, s: &[u32], wide: bool) {
        let mut b = Vec::new();
        for c in s {
            if wide {
                b.extend_from_slice(&(*c as u16).to_le_bytes());
            } else {
                b.push(*c as u8);
            }
        }
        b.extend_from_slice(if wide { &[0, 0] } else { &[0] });
        let _ = self.mem.write(at, &b);
    }

    fn crt_cell(&mut self, name: &str, size: u64) -> u64 {
        self.data.cell(name, size).unwrap_or(0)
    }

    fn set_errno(&mut self, e: u32) {
        let c = self.crt_cell("<errno>", 4);
        if c != 0 {
            let _ = self.mem.write(c, &e.to_le_bytes());
        }
    }

    /// `malloc`: size-class free lists over the bump heap.
    pub(crate) fn crt_alloc(&mut self, n: u64) -> u64 {
        if n > (1 << 30) {
            return 0;
        }
        let class = (n.max(1) + 15) & !15;
        if let Some(p) = self.crt.free.get_mut(&class).and_then(Vec::pop) {
            let _ = self.mem.write(p - 16, &n.to_le_bytes());
            return p;
        }
        let p = self.heap_alloc(n);
        if p == 0 {
            self.set_errno(12);
        }
        p
    }

    fn crt_free(&mut self, p: u64) {
        if p < 16 {
            return;
        }
        let n = self.mem.read_u64(p - 16).unwrap_or(0);
        let class = (n.max(1) + 15) & !15;
        let l = self.crt.free.entry(class).or_default();
        if !l.contains(&p) {
            l.push(p);
        }
    }

    fn crt_size(&mut self, p: u64) -> u64 {
        if p < 16 { 0 } else { self.mem.read_u64(p - 16).unwrap_or(0) }
    }

    fn fill(&mut self, at: u64, byte: u8, n: u64) -> bool {
        let mut done = 0u64;
        let chunk = alloc::vec![byte; 65536.min(n as usize).max(1)];
        while done < n {
            let k = (n - done).min(chunk.len() as u64) as usize;
            if self.mem.write(at + done, &chunk[..k]).is_err() {
                return false;
            }
            done += k as u64;
        }
        true
    }

    fn copy_mem(&mut self, dst: u64, src: u64, n: u64) -> bool {
        if n == 0 {
            return true;
        }
        if n > (64 << 20) {
            return false;
        }
        match self.mem.read_vec(src, n as usize) {
            Ok(b) => self.mem.write(dst, &b).is_ok(),
            Err(_) => false,
        }
    }

    /// Splits the command line into arguments (Windows rules) and builds `argv`/`wargv` once.
    fn ensure_args(&mut self) {
        if self.crt.args_built {
            return;
        }
        self.crt.args_built = true;
        let cmd = self.mem.read_cstr(self.cmdline_addr(), 8192).unwrap_or_default();
        let mut args: Vec<String> = Vec::new();
        let (mut cur, mut in_q, mut started) = (String::new(), false, false);
        let chars: Vec<char> = cmd.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let mut n = 0;
                while chars.get(i + n) == Some(&'\\') {
                    n += 1;
                }
                if chars.get(i + n) == Some(&'"') {
                    cur.extend(core::iter::repeat('\\').take(n / 2));
                    if n % 2 == 1 {
                        cur.push('"');
                        i += n + 1;
                    } else {
                        i += n;
                    }
                } else {
                    cur.extend(core::iter::repeat('\\').take(n));
                    i += n;
                }
                started = true;
                continue;
            }
            if c == '"' {
                in_q = !in_q;
                started = true;
            } else if c.is_whitespace() && !in_q {
                if started {
                    args.push(core::mem::take(&mut cur));
                    started = false;
                }
            } else {
                cur.push(c);
                started = true;
            }
            i += 1;
        }
        if started {
            args.push(cur);
        }
        let n = args.len() as u64;
        let (argv, wargv) = (self.heap_alloc(8 * (n + 1)), self.heap_alloc(8 * (n + 1)));
        for (k, a) in args.iter().enumerate() {
            let k = k as u64;
            let (np, wp) = (self.heap_alloc(a.len() as u64 + 1), self.heap_alloc(2 * (a.encode_utf16().count() as u64 + 1)));
            let narrow: Vec<u32> = a.bytes().map(u32::from).collect();
            self.wr_z(np, &narrow, false);
            let w: Vec<u32> = a.encode_utf16().map(u32::from).collect();
            self.wr_z(wp, &w, true);
            let _ = self.mem.write(argv + 8 * k, &np.to_le_bytes());
            let _ = self.mem.write(wargv + 8 * k, &wp.to_le_bytes());
        }
        let (c_argc, c_argv, c_wargv) = (self.crt_cell("__argc", 4), self.crt_cell("__argv", 8), self.crt_cell("__wargv", 8));
        let _ = self.mem.write(c_argc, &(n as u32).to_le_bytes());
        let _ = self.mem.write(c_argv, &argv.to_le_bytes());
        let _ = self.mem.write(c_wargv, &wargv.to_le_bytes());
        for (cell, wide) in [("_acmdln", false), ("_wcmdln", true)] {
            let c = self.crt_cell(cell, 8);
            let text = self.mem.read_cstr(self.cmdline_addr(), 8192).unwrap_or_default();
            let (units, b): (Vec<u32>, u64) = if wide { (text.encode_utf16().map(u32::from).collect(), 2) } else { (text.bytes().map(u32::from).collect(), 1) };
            let p = self.heap_alloc((units.len() as u64 + 1) * b);
            self.wr_z(p, &units, wide);
            let _ = self.mem.write(c, &p.to_le_bytes());
        }
        let name = args.first().cloned().unwrap_or_default();
        for (cell, wide) in [("_pgmptr", false), ("_wpgmptr", true)] {
            let c = self.crt_cell(cell, 8);
            let (units, b): (Vec<u32>, u64) = if wide { (name.encode_utf16().map(u32::from).collect(), 2) } else { (name.bytes().map(u32::from).collect(), 1) };
            let p = self.heap_alloc((units.len() as u64 + 1) * b);
            self.wr_z(p, &units, wide);
            let _ = self.mem.write(c, &p.to_le_bytes());
        }
        self.build_environ();
    }

    pub(crate) fn env_map(&mut self) -> &mut BTreeMap<String, String> {
        self.crt.env.get_or_insert_with(default_env)
    }

    fn build_environ(&mut self) {
        let entries: Vec<String> = self.env_map().values().cloned().collect();
        let n = entries.len() as u64;
        let (narrow, wide) = (self.heap_alloc(8 * (n + 1)), self.heap_alloc(8 * (n + 1)));
        for (k, e) in entries.iter().enumerate() {
            let k = k as u64;
            let (np, wp) = (self.heap_alloc(e.len() as u64 + 1), self.heap_alloc(2 * (e.encode_utf16().count() as u64 + 1)));
            let nb: Vec<u32> = e.bytes().map(u32::from).collect();
            self.wr_z(np, &nb, false);
            let wb: Vec<u32> = e.encode_utf16().map(u32::from).collect();
            self.wr_z(wp, &wb, true);
            let _ = self.mem.write(narrow + 8 * k, &np.to_le_bytes());
            let _ = self.mem.write(wide + 8 * k, &wp.to_le_bytes());
        }
        for (cells, v) in [(["_environ", "__initenv"], narrow), (["_wenviron", "__winitenv"], wide)] {
            for c in cells {
                let a = self.crt_cell(c, 8);
                let _ = self.mem.write(a, &v.to_le_bytes());
            }
        }
    }

    /// C-runtime calls. `None` for numbers outside this table.
    pub(crate) fn crt_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now: u64) -> Option<Ret> {
        let idx = nr.checked_sub(CRT_BASE)? as usize;
        let Some(&name) = NAMES.get(idx) else { return self.cio_syscall(nr, a, rsp, now) };
        let v = |x: u64| Some(Ret::Value(x));
        let wide = name.starts_with("_w") || name.starts_with("wcs") || name.starts_with("isw") || name.starts_with("tow");
        match name {
            // ---- start-up and exit ----
            "__set_app_type" | "_set_app_type" | "_set_fmode" | "_set_commode" | "_set_new_mode" | "_configthreadlocale"
            | "__setusermatherr" | "_set_invalid_parameter_handler" | "_controlfp_s" | "_lock" | "_unlock" | "__lock" | "__unlock"
            | "_set_abort_behavior" | "_seh_filter_exe" | "_configure_narrow_argv" | "_configure_wide_argv" => {
                if matches!(name, "_configure_narrow_argv" | "_configure_wide_argv") {
                    self.ensure_args();
                }
                v(0)
            }
            "__getmainargs" | "__wgetmainargs" => {
                self.ensure_args();
                let (c, av) = (self.crt_cell("__argc", 4), self.crt_cell(if name == "__getmainargs" { "__argv" } else { "__wargv" }, 8));
                let (argc, argv) = (self.mem.read_u32(c).unwrap_or(0), self.mem.read_u64(av).unwrap_or(0));
                let env = self.crt_cell(if name == "__getmainargs" { "_environ" } else { "_wenviron" }, 8);
                let envp = self.mem.read_u64(env).unwrap_or(0);
                let _ = self.mem.write(a[0], &argc.to_le_bytes());
                let _ = self.mem.write(a[1], &argv.to_le_bytes());
                let _ = self.mem.write(a[2], &envp.to_le_bytes());
                v(0)
            }
            "__p___argc" => {
                self.ensure_args();
                v(self.crt_cell("__argc", 4))
            }
            "__p___argv" | "__p___wargv" => {
                self.ensure_args();
                v(self.crt_cell(if name == "__p___argv" { "__argv" } else { "__wargv" }, 8))
            }
            "_get_initial_narrow_environment" | "_get_initial_wide_environment" | "__p__environ" | "__p__wenviron" => {
                self.ensure_args();
                let narrow = matches!(name, "_get_initial_narrow_environment" | "__p__environ");
                let cell = self.crt_cell(if narrow { "_environ" } else { "_wenviron" }, 8);
                v(if name.starts_with("_get_initial") { self.mem.read_u64(cell).unwrap_or(0) } else { cell })
            }
            "_initialize_narrow_environment" | "_initialize_wide_environment" => {
                self.ensure_args();
                v(0)
            }
            "__p__fmode" => v(self.crt_cell("_fmode", 4)),
            "__p__commode" => v(self.crt_cell("_commode", 4)),
            "__p__pgmptr" | "_get_pgmptr" | "_get_wpgmptr" => {
                self.ensure_args();
                let c = self.crt_cell(if name == "_get_wpgmptr" { "_wpgmptr" } else { "_pgmptr" }, 8);
                if name == "__p__pgmptr" {
                    return v(c);
                }
                let val = self.mem.read_u64(c).unwrap_or(0);
                let _ = self.mem.write(a[0], &val.to_le_bytes());
                v(0)
            }
            "_initterm" | "_initterm_e" => {
                // Table of void(void) (or int(void)) function pointers in [a0, a1): run them in the guest, in order.
                let mut p = a[0];
                let mut n = 0;
                while p < a[1] && n < 1024 {
                    let f = self.mem.read_u64(p).unwrap_or(0);
                    if f != 0 {
                        self.pending_init.push((f, 0));
                    }
                    p += 8;
                    n += 1;
                }
                Some(Ret::Init(0))
            }
            "_crt_atexit" | "atexit" | "_crt_at_quick_exit" | "_onexit" => {
                self.crt.atexit.push(a[0]);
                v(if name == "_onexit" { a[0] } else { 0 })
            }
            "_initialize_onexit_table" => v(0),
            "_register_onexit_function" => {
                self.crt.atexit.push(a[1]);
                v(0)
            }
            "_execute_onexit_table" | "_cexit" | "_c_exit" => v(0),
            "exit" | "_exit" | "_Exit" => Some(Ret::Exit(a[0] as u32)),
            "abort" => Some(Ret::Exit(3)),
            "_amsg_exit" => Some(Ret::Exit(255)),
            "_invalid_parameter_noinfo" => v(0),
            "_invalid_parameter_noinfo_noreturn" => Some(Ret::Exit(0xc000_000d)),
            "signal" | "raise" => v(0),
            "_errno" => v(self.crt_cell("<errno>", 4)),
            "__doserrno" => v(self.crt_cell("<doserrno>", 4)),
            "_get_errno" => {
                let c = self.crt_cell("<errno>", 4);
                let e = self.mem.read_u32(c).unwrap_or(0);
                let _ = self.mem.write(a[0], &e.to_le_bytes());
                v(0)
            }
            "_set_errno" => {
                self.set_errno(a[0] as u32);
                v(0)
            }
            "strerror" => {
                let s = match a[0] as u32 {
                    0 => "No error",
                    2 => "No such file or directory",
                    9 => "Bad file descriptor",
                    12 => "Not enough space",
                    13 => "Permission denied",
                    17 => "File exists",
                    22 => "Invalid argument",
                    34 => "Result too large",
                    _ => "Unknown error",
                };
                let p = self.crt_cell("<strerror>", 64);
                let u: Vec<u32> = s.bytes().map(u32::from).collect();
                self.wr_z(p, &u, false);
                v(p)
            }
            "setlocale" => {
                let p = self.crt_cell("<locale>", 4);
                self.wr_z(p, &[0x43], false);
                v(p)
            }
            "localeconv" => {
                let st = self.crt_cell("<lconv>", 96);
                let dot = self.crt_cell("<dot>", 4);
                self.wr_z(dot, &[0x2e], false);
                let empty = self.crt_cell("<empty>", 4);
                let mut b = [0u8; 96];
                b[0..8].copy_from_slice(&dot.to_le_bytes());
                for k in 1..8 {
                    b[8 * k..8 * k + 8].copy_from_slice(&empty.to_le_bytes());
                }
                let _ = self.mem.write(st, &b);
                v(st)
            }
            "___mb_cur_max_func" | "__mb_cur_max" => v(1),
            "___lc_codepage_func" => v(1252),
            "_get_osfhandle" => v(match a[0] as i32 {
                0 => 0x10,
                1 => 0x14,
                2 => 0x18,
                _ => u64::MAX,
            }),
            "clock" => v(now),

            // ---- heap ----
            "malloc" => v(self.crt_alloc(a[0])),
            "calloc" | "_recalloc" => {
                let (cnt, sz, old) = if name == "calloc" { (a[0], a[1], 0) } else { (a[1], a[2], a[0]) };
                let Some(n) = cnt.checked_mul(sz) else { return v(0) };
                let p = self.crt_alloc(n);
                if p != 0 {
                    self.fill(p, 0, n);
                    if old != 0 {
                        let k = self.crt_size(old).min(n);
                        self.copy_mem(p, old, k);
                        self.crt_free(old);
                    }
                }
                v(p)
            }
            "realloc" | "_expand" => {
                if a[0] == 0 {
                    return v(self.crt_alloc(a[1]));
                }
                if a[1] == 0 && name == "realloc" {
                    self.crt_free(a[0]);
                    return v(0);
                }
                let old = self.crt_size(a[0]);
                if (a[1].max(1) + 15 & !15) == (old.max(1) + 15 & !15) {
                    let _ = self.mem.write(a[0] - 16, &a[1].to_le_bytes());
                    return v(a[0]);
                }
                if name == "_expand" {
                    return v(0);
                }
                let p = self.crt_alloc(a[1]);
                if p != 0 {
                    self.copy_mem(p, a[0], old.min(a[1]));
                    self.crt_free(a[0]);
                }
                v(p)
            }
            "free" | "_aligned_free" => {
                if name == "_aligned_free" && a[0] != 0 {
                    let orig = self.mem.read_u64(a[0] - 8).unwrap_or(0);
                    self.crt_free(orig);
                } else {
                    self.crt_free(a[0]);
                }
                v(0)
            }
            "_msize" => v(self.crt_size(a[0])),
            "_aligned_malloc" => {
                // (size, alignment): over-allocate, align, and stash the original pointer just below the result.
                let al = a[1].max(16);
                if !al.is_power_of_two() {
                    return v(0);
                }
                let raw = self.crt_alloc(a[0] + al + 8);
                if raw == 0 {
                    return v(0);
                }
                let p = (raw + 8 + al - 1) & !(al - 1);
                let _ = self.mem.write(p - 8, &raw.to_le_bytes());
                v(p)
            }

            // ---- memory ----
            "memcpy" | "memmove" => {
                if a[2] > (64 << 20) || !self.copy_mem(a[0], a[1], a[2]) {
                    return v(0);
                }
                v(a[0])
            }
            "memcpy_s" | "memmove_s" => {
                // (dst, dstsz, src, count)
                if a[3] > a[1] {
                    return v(34);
                }
                v(if self.copy_mem(a[0], a[2], a[3]) { 0 } else { 22 })
            }
            "memset" => {
                self.fill(a[0], a[1] as u8, a[2]);
                v(a[0])
            }
            "memcmp" | "_memicmp" => {
                let n = a[2].min(64 << 20) as usize;
                let (x, y) = (self.mem.read_vec(a[0], n).unwrap_or_default(), self.mem.read_vec(a[1], n).unwrap_or_default());
                for (p, q) in x.iter().zip(&y) {
                    let (p, q) = if name == "_memicmp" { (lower(u32::from(*p)) as u8, lower(u32::from(*q)) as u8) } else { (*p, *q) };
                    if p != q {
                        return v((i32::from(p) - i32::from(q)) as i64 as u64);
                    }
                }
                v(0)
            }
            "memchr" => {
                let n = a[2].min(64 << 20) as usize;
                let b = self.mem.read_vec(a[0], n).unwrap_or_default();
                v(b.iter().position(|c| *c == a[1] as u8).map_or(0, |i| a[0] + i as u64))
            }

            // ---- strings (narrow and wide share one implementation) ----
            "strlen" | "wcslen" => v(self.rd_z(a[0], wide, usize::MAX).len() as u64),
            "strnlen" | "wcsnlen" => v(self.rd_z(a[0], wide, a[1] as usize).len() as u64),
            "strcmp" | "strncmp" | "_stricmp" | "_strnicmp" | "stricmp" | "strcasecmp" | "strncasecmp" | "wcscmp" | "wcsncmp" | "_wcsicmp" | "_wcsnicmp" => {
                let limited = name.contains("ncmp") || name.contains("nicmp") || name.contains("ncasecmp");
                let ci = name.contains("icmp") || name.contains("casecmp");
                let max = if limited { a[2] as usize } else { usize::MAX };
                let (x, y) = (self.rd_z(a[0], wide, max), self.rd_z(a[1], wide, max));
                let n = x.len().max(y.len()).min(max);
                for k in 0..n {
                    let (p, q) = (x.get(k).copied().unwrap_or(0), y.get(k).copied().unwrap_or(0));
                    let (p, q) = if ci { (lower(p), lower(q)) } else { (p, q) };
                    if p != q {
                        return v((p as i32 - q as i32).signum() as i64 as u64);
                    }
                }
                v(0)
            }
            "strchr" | "wcschr" | "strrchr" | "wcsrchr" => {
                let s = self.rd_z(a[0], wide, usize::MAX);
                let unit = if wide { 2 } else { 1 };
                let c = if wide { a[1] as u32 & 0xffff } else { a[1] as u32 & 0xff };
                let rev = name.contains("rchr");
                let pos = if c == 0 { Some(s.len()) } else if rev { s.iter().rposition(|x| *x == c) } else { s.iter().position(|x| *x == c) };
                v(pos.map_or(0, |i| a[0] + (i * unit) as u64))
            }
            "strstr" | "wcsstr" => {
                let (h, n) = (self.rd_z(a[0], wide, usize::MAX), self.rd_z(a[1], wide, usize::MAX));
                let unit = if wide { 2 } else { 1 };
                if n.is_empty() {
                    return v(a[0]);
                }
                v(h.windows(n.len()).position(|w| w == &n[..]).map_or(0, |i| a[0] + (i * unit) as u64))
            }
            "strcpy" | "wcscpy" | "strncpy" | "wcsncpy" | "strcat" | "wcscat" | "strncat" | "wcsncat" => {
                let cat = name.contains("cat");
                let lim = name.contains("ncpy") || name.contains("ncat");
                let n = if lim { a[2] as usize } else { usize::MAX };
                let unit = if wide { 2u64 } else { 1 };
                let src = self.rd_z(a[1], wide, n);
                let dst_at = if cat { a[0] + self.rd_z(a[0], wide, usize::MAX).len() as u64 * unit } else { a[0] };
                let mut out = src.clone();
                let mut zero_pad = 0;
                if name.contains("ncpy") && src.len() < n {
                    zero_pad = n - src.len(); // strncpy pads with NULs up to n
                }
                if name.contains("ncpy") && src.len() >= n {
                    // no terminator written when the source is at least n long
                    let mut b = Vec::new();
                    for c in &out[..n] {
                        if wide { b.extend_from_slice(&(*c as u16).to_le_bytes()) } else { b.push(*c as u8) }
                    }
                    let _ = self.mem.write(dst_at, &b);
                    return v(a[0]);
                }
                out.extend(core::iter::repeat(0).take(zero_pad));
                self.wr_z(dst_at, &out, wide);
                v(a[0])
            }
            "strcpy_s" | "wcscpy_s" | "strcat_s" | "wcscat_s" | "strncpy_s" => {
                // (dst, dstsz, src[, count])
                let unit = if wide { 2u64 } else { 1 };
                let cat = name.contains("cat");
                let src = self.rd_z(a[2], wide, usize::MAX);
                let used = if cat { self.rd_z(a[0], wide, usize::MAX).len() } else { 0 };
                if a[0] == 0 || a[1] == 0 {
                    return v(22);
                }
                if used + src.len() + 1 > a[1] as usize {
                    self.wr_z(a[0], &[], wide);
                    return v(34);
                }
                self.wr_z(a[0] + used as u64 * unit, &src, wide);
                v(0)
            }
            "strcspn" | "wcscspn" | "strspn" | "wcsspn" | "strpbrk" | "wcspbrk" => {
                let (s, set) = (self.rd_z(a[0], wide, usize::MAX), self.rd_z(a[1], wide, usize::MAX));
                let unit = if wide { 2 } else { 1 };
                match name {
                    "strcspn" | "wcscspn" => v(s.iter().position(|c| set.contains(c)).unwrap_or(s.len()) as u64),
                    "strspn" | "wcsspn" => v(s.iter().position(|c| !set.contains(c)).unwrap_or(s.len()) as u64),
                    _ => v(s.iter().position(|c| set.contains(c)).map_or(0, |i| a[0] + (i * unit) as u64)),
                }
            }
            "strdup" | "_strdup" | "_wcsdup" | "wcsdup" => {
                let s = self.rd_z(a[0], wide, usize::MAX);
                let unit = if wide { 2 } else { 1 };
                let p = self.crt_alloc((s.len() as u64 + 1) * unit);
                if p != 0 {
                    self.wr_z(p, &s, wide);
                }
                v(p)
            }
            "strtok" => {
                // Not re-entrant (as in C): the scan position persists between calls.
                let start = if a[0] != 0 { a[0] } else { self.crt.tok };
                if start == 0 {
                    return v(0);
                }
                let s = self.rd_z(start, false, usize::MAX);
                let delims = self.rd_z(a[1], false, usize::MAX);
                let Some(b) = s.iter().position(|c| !delims.contains(c)) else {
                    self.crt.tok = 0;
                    return v(0);
                };
                let rest = &s[b..];
                match rest.iter().position(|c| delims.contains(c)) {
                    Some(e) => {
                        let _ = self.mem.write(start + (b + e) as u64, &[0]);
                        self.crt.tok = start + (b + e) as u64 + 1;
                    }
                    None => self.crt.tok = 0,
                }
                v(start + b as u64)
            }
            "_strlwr" | "_strupr" | "_wcslwr" | "_wcsupr" => {
                let s = self.rd_z(a[0], wide, usize::MAX);
                let t: Vec<u32> = s.iter().map(|c| if name.contains("lwr") { lower(*c) } else { upper(*c) }).collect();
                self.wr_z(a[0], &t, wide);
                v(a[0])
            }
            "toupper" | "tolower" | "_toupper" | "_tolower" | "towupper" | "towlower" => {
                let c = if wide { a[0] as u32 & 0xffff } else { a[0] as u32 & 0xff };
                v(u64::from(if name.contains("upper") { upper(c) } else { lower(c) }))
            }

            // ---- ctype (C locale; the wide variants also classify Unicode letters) ----
            n if n.starts_with("is") => {
                let c = if wide { a[0] as u32 & 0xffff } else { a[0] as u32 & 0xff };
                let ch = char::from_u32(c).unwrap_or('\0');
                let r = match n.trim_start_matches("isw").trim_start_matches("is") {
                    "alpha" => ch.is_ascii_alphabetic() || (wide && ch.is_alphabetic()),
                    "digit" => ch.is_ascii_digit(),
                    "alnum" => ch.is_ascii_alphanumeric() || (wide && ch.is_alphanumeric()),
                    "space" => is_space(c),
                    "upper" => ch.is_ascii_uppercase() || (wide && ch.is_uppercase()),
                    "lower" => ch.is_ascii_lowercase() || (wide && ch.is_lowercase()),
                    "print" => (0x20..0x7f).contains(&c) || (wide && c > 0xa0),
                    "graph" => (0x21..0x7f).contains(&c) || (wide && c > 0xa0),
                    "punct" => ch.is_ascii_punctuation(),
                    "xdigit" => ch.is_ascii_hexdigit(),
                    "cntrl" => c < 0x20 || c == 0x7f,
                    "ascii" => c < 0x80,
                    _ => false,
                };
                v(u64::from(r))
            }

            // ---- conversion ----
            "atoi" | "atol" | "atoll" | "_atoi64" | "_wtoi" | "_wtol" | "_wtoi64" => {
                let s = self.rd_z(a[0], wide || name.starts_with("_w"), 64);
                let (val, neg, _, _) = parse_int(&s, 10);
                let r = if neg { (val as i64).wrapping_neg() } else { val as i64 };
                v(if matches!(name, "atoll" | "_atoi64" | "_wtoi64") { r as u64 } else { r as i32 as i64 as u64 })
            }
            "strtol" | "strtoul" | "strtoll" | "strtoull" | "_strtoi64" | "_strtoui64" | "wcstol" | "wcstoul" | "wcstoll" | "wcstoull" => {
                let is_wide = name.starts_with("wcs");
                let s = self.rd_z(a[0], is_wide, 256);
                let unsigned = name.ends_with("ul") || name.ends_with("ull") || name == "_strtoui64";
                let long = name.ends_with("ll") || name.starts_with("_strto");
                let (val, neg, used, over) = parse_int(&s, a[2] as u32);
                if a[1] != 0 {
                    let unit = if is_wide { 2 } else { 1 };
                    let _ = self.mem.write(a[1], &(a[0] + (used * unit) as u64).to_le_bytes());
                }
                let (max_pos, max_neg): (u64, u64) = match (unsigned, long) {
                    (true, true) => (u64::MAX, u64::MAX),
                    (true, false) => (u64::from(u32::MAX), u64::from(u32::MAX)),
                    (false, true) => (i64::MAX as u64, 1 << 63),
                    (false, false) => (i32::MAX as u64, 1 << 31),
                };
                let (res, clamp) = if unsigned {
                    if over || val > max_pos { (max_pos, true) } else { (if neg { val.wrapping_neg() } else { val }, false) }
                } else if neg {
                    if over || val > max_neg { ((max_neg as i64).wrapping_neg() as u64, true) } else { ((val as i64).wrapping_neg() as u64, false) }
                } else if over || val > max_pos {
                    (max_pos, true)
                } else {
                    (val, false)
                };
                if clamp {
                    self.set_errno(34);
                }
                v(if long || unsigned && long { res } else if unsigned { res & 0xffff_ffff } else { res as i32 as i64 as u64 })
            }
            "_itoa" | "_ltoa" | "_ultoa" | "_i64toa" | "_ui64toa" | "_itow" | "_ltow" | "_ultow" | "_i64tow" | "_ui64tow" => {
                // (value, buffer, radix)
                let is_wide = name.ends_with('w');
                let signed = matches!(name, "_itoa" | "_ltoa" | "_i64toa" | "_itow" | "_ltow" | "_i64tow");
                let wide64 = name.contains("64");
                let radix = a[2].clamp(2, 36);
                let (mut val, neg) = if signed && radix == 10 {
                    let x = if wide64 { a[0] as i64 } else { a[0] as i32 as i64 };
                    (x.unsigned_abs(), x < 0)
                } else {
                    (if wide64 { a[0] } else { a[0] & 0xffff_ffff }, false)
                };
                let mut d = Vec::new();
                loop {
                    let r = (val % radix) as u32;
                    d.push(if r < 10 { 0x30 + r } else { 0x61 + r - 10 });
                    val /= radix;
                    if val == 0 {
                        break;
                    }
                }
                if neg {
                    d.push(0x2d);
                }
                d.reverse();
                self.wr_z(a[1], &d, is_wide);
                v(a[1])
            }
            "abs" => v((a[0] as i32).unsigned_abs() as u64),
            "labs" => v((a[0] as i32).unsigned_abs() as u64),
            "llabs" | "_abs64" => v((a[0] as i64).unsigned_abs()),
            "rand" => {
                self.crt.seed = self.crt.seed.wrapping_mul(214_013).wrapping_add(2_531_011);
                v(u64::from(self.crt.seed >> 16 & 0x7fff))
            }
            "srand" => {
                self.crt.seed = a[0] as u32;
                v(0)
            }
            "atof" | "strtod" | "_atof_l" => {
                // The double result belongs in XMM0, which the syscall path cannot set: return the bits in RAX and
                // let callers that need it use the integer conversions. Parsing still validates and sets `endptr`.
                let s = self.rd_z(a[0], false, 256);
                let (val, used) = parse_float(&s);
                if name == "strtod" && a[1] != 0 {
                    let _ = self.mem.write(a[1], &(a[0] + used as u64).to_le_bytes());
                }
                v(val.to_bits())
            }

            // ---- environment ----
            "getenv" | "_wgetenv" => {
                let key = String::from_utf16_lossy(&self.rd_z(a[0], wide, 256).iter().map(|c| *c as u16).collect::<Vec<_>>()).to_ascii_uppercase();
                let Some(entry) = self.env_map().get(&key).cloned() else { return v(0) };
                let val = entry.split_once('=').map(|x| x.1.to_string()).unwrap_or_default();
                let u: Vec<u32> = if wide { val.encode_utf16().map(u32::from).collect() } else { val.bytes().map(u32::from).collect() };
                let p = self.crt_alloc((u.len() as u64 + 1) * if wide { 2 } else { 1 });
                self.wr_z(p, &u, wide);
                v(p)
            }
            "getenv_s" => {
                // (&required, buffer, size, name)
                let key = String::from_utf8_lossy(&self.rd_z(a[3], false, 256).iter().map(|c| *c as u8).collect::<Vec<_>>()).to_ascii_uppercase();
                let Some(entry) = self.env_map().get(&key).cloned() else {
                    let _ = self.mem.write(a[0], &0u64.to_le_bytes());
                    return v(0);
                };
                let val = entry.split_once('=').map(|x| x.1.to_string()).unwrap_or_default();
                let _ = self.mem.write(a[0], &(val.len() as u64 + 1).to_le_bytes());
                let size = self.stack_arg(rsp, 5);
                let _ = size;
                if a[1] != 0 && a[2] > val.len() as u64 {
                    let u: Vec<u32> = val.bytes().map(u32::from).collect();
                    self.wr_z(a[1], &u, false);
                    return v(0);
                }
                v(if a[1] == 0 { 0 } else { 34 })
            }
            "_putenv" | "_wputenv" | "_putenv_s" => {
                let (key, val) = if name == "_putenv_s" {
                    (self.rd_z(a[0], false, 256), self.rd_z(a[1], false, 4096))
                } else {
                    let s = self.rd_z(a[0], wide, 4096);
                    match s.iter().position(|c| *c == 0x3d) {
                        Some(i) => (s[..i].to_vec(), s[i + 1..].to_vec()),
                        None => return v(u64::MAX),
                    }
                };
                let k: String = key.iter().filter_map(|c| char::from_u32(*c)).collect();
                let val: String = val.iter().filter_map(|c| char::from_u32(*c)).collect();
                if k.is_empty() {
                    return v(if name == "_putenv_s" { 22 } else { u64::MAX });
                }
                if val.is_empty() {
                    self.env_map().remove(&k.to_ascii_uppercase());
                } else {
                    self.env_map().insert(k.to_ascii_uppercase(), alloc::format!("{k}={val}"));
                }
                if self.crt.args_built {
                    self.build_environ();
                }
                v(0)
            }
            _ => self.cio_syscall(nr, a, rsp, now),
        }
    }
}
