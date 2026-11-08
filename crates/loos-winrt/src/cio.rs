//! C stdio over the guest volume and the console: `FILE` objects, `fopen`/`fread`/`fwrite`..., and the printf family
//! (including the UCRT `__stdio_common_*` entry points). Output is written through, so there is no buffering to flush.

use crate::crt::CRT_BASE;
use crate::fmt::{self, Args};
use crate::{files, Process, Ret};
use alloc::string::String;
use alloc::vec::Vec;

/// First syscall number of the stdio table.
pub const CIO_BASE: u32 = CRT_BASE + 0x200;
/// Size of a guest `FILE` (the CRT's `FILE` is 48 bytes; `__iob_func` arrays are indexed with this stride).
const FILE_SIZE: u64 = 48;
const EOF: u64 = u64::MAX;

pub(crate) enum Kind {
    Stdin,
    Out,
    Err,
    Vfs { path: String, pos: usize, read: bool, write: bool, append: bool },
}

/// An open `FILE`.
pub struct CFile {
    kind: Kind,
    eof: bool,
    err: bool,
    unget: Option<u8>,
    fd: i32,
}

const NAMES: &[&str] = &[
    "__acrt_iob_func", "__iob_func", "fopen", "_wfopen", "fopen_s", "_wfopen_s", "fclose", "fread", "fwrite", "fputs", "fputc",
    "putc", "putchar", "puts", "fgets", "fgetc", "getc", "getchar", "ungetc", "fflush", "feof", "ferror", "clearerr", "fseek",
    "ftell", "rewind", "_fileno", "fileno", "setvbuf", "setbuf", "fprintf", "printf", "sprintf", "snprintf", "_snprintf",
    "vfprintf", "vprintf", "vsprintf", "vsnprintf", "_vsnprintf", "_vsnwprintf", "_snwprintf", "fwprintf", "wprintf", "vfwprintf",
    "__stdio_common_vfprintf", "__stdio_common_vsprintf", "__stdio_common_vswprintf", "__stdio_common_vfwprintf",
    "__stdio_common_vsnprintf_s", "__local_stdio_printf_options", "__local_stdio_scanf_options", "_write", "_close",
    "remove", "_unlink", "unlink", "_access", "sprintf_s", "_fseeki64", "_ftelli64", "fputws", "_fflush_nolock", "_set_printf_count_output",
];

/// Syscall number of a stdio import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| CIO_BASE + i as u32)
}

/// Where the next vararg comes from.
enum Va {
    /// Register/stack slots of the current call, starting at slot `idx` (slots 0..4 are registers).
    Slots { regs: [u64; 4], rsp: u64, idx: u64 },
    /// A `va_list` (a pointer to consecutive 8-byte slots).
    List(u64),
}

struct Source<'a> {
    p: &'a mut Process,
    va: Va,
}

impl Args for Source<'_> {
    fn next(&mut self) -> u64 {
        match &mut self.va {
            Va::Slots { regs, rsp, idx } => {
                let k = *idx;
                *idx += 1;
                if k < 4 { regs[k as usize] } else { self.p.stack_arg(*rsp, k + 1) }
            }
            Va::List(at) => {
                let v = self.p.mem.read_u64(*at).unwrap_or(0);
                *at += 8;
                v
            }
        }
    }
    fn string(&mut self, ptr: u64, wide: bool, max: usize) -> Vec<u32> {
        self.p.rd_z(ptr, wide, max)
    }
}

fn utf8(chars: &[u32]) -> Vec<u8> {
    let mut o = Vec::new();
    for c in chars {
        let ch = char::from_u32(*c).unwrap_or('?');
        let mut b = [0u8; 4];
        o.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
    }
    o
}

impl Process {
    fn std_file(&mut self, i: u64) -> u64 {
        let base = self.data.cell("_iob", 144).unwrap_or(0);
        let addr = base + i * FILE_SIZE;
        if base != 0 && !self.crt.files.contains_key(&addr) {
            let kind = match i {
                0 => Kind::Stdin,
                1 => Kind::Out,
                _ => Kind::Err,
            };
            self.crt.files.insert(addr, CFile { kind, eof: false, err: false, unget: None, fd: i as i32 });
        }
        addr
    }

    fn ensure_std(&mut self) {
        for i in 0..3 {
            self.std_file(i);
        }
    }

    /// Writes raw bytes to an open file; returns the count written.
    fn file_put(&mut self, f: u64, data: &[u8]) -> usize {
        self.ensure_std();
        let Some(file) = self.crt.files.get_mut(&f) else { return 0 };
        match &mut file.kind {
            Kind::Out | Kind::Err => {
                self.console_write(data);
                data.len()
            }
            Kind::Stdin => 0,
            Kind::Vfs { path, pos, write, append, .. } => {
                if !*write {
                    file.err = true;
                    return 0;
                }
                let Some(buf) = self.vfs.get_mut(path.as_str()) else { return 0 };
                if *append {
                    *pos = buf.len();
                }
                if *pos > buf.len() {
                    buf.resize(*pos, 0);
                }
                let end = *pos + data.len();
                if end > buf.len() {
                    buf.resize(end, 0);
                }
                buf[*pos..end].copy_from_slice(data);
                *pos = end;
                data.len()
            }
        }
    }

    /// Reads one byte, honouring `ungetc`.
    fn file_getc(&mut self, f: u64) -> Option<u8> {
        self.ensure_std();
        let file = self.crt.files.get_mut(&f)?;
        if let Some(b) = file.unget.take() {
            file.eof = false;
            return Some(b);
        }
        match &mut file.kind {
            Kind::Vfs { path, pos, read, .. } => {
                if !*read {
                    file.err = true;
                    return None;
                }
                match self.vfs.get(path.as_str()).and_then(|d| d.get(*pos)).copied() {
                    Some(b) => {
                        *pos += 1;
                        Some(b)
                    }
                    None => {
                        file.eof = true;
                        None
                    }
                }
            }
            _ => {
                file.eof = true;
                None
            }
        }
    }

    fn printf_to(&mut self, chars: &[u32], f: u64, wide_out: bool) -> u64 {
        let bytes: Vec<u8> = if wide_out { utf8(chars) } else { chars.iter().map(|c| *c as u8).collect() };
        let n = self.file_put(f, &bytes);
        if n == bytes.len() { chars.len() as u64 } else { u64::MAX }
    }

    /// Stores formatted text in a guest buffer with `snprintf` semantics; returns the C return value.
    fn printf_buf(&mut self, buf: u64, cap: u64, chars: &[u32], wide: bool, std_trunc: bool) -> u64 {
        let total = chars.len() as u64;
        if buf == 0 || cap == 0 {
            return if std_trunc || buf == 0 { total } else { u64::MAX };
        }
        let fits = total < cap;
        let take = if fits { chars.len() } else { (cap - 1) as usize };
        self.wr_z(buf, &chars[..take], wide);
        if fits || std_trunc {
            total
        } else {
            u64::MAX
        }
    }

    fn fmt_in(&mut self, ptr: u64, wide: bool) -> Vec<u32> {
        self.rd_z(ptr, wide, 1 << 16)
    }

    /// Stdio and printf calls. `None` for numbers outside this table.
    pub(crate) fn cio_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, _now: u64) -> Option<Ret> {
        let name = *NAMES.get(nr.checked_sub(CIO_BASE)? as usize)?;
        let v = |x: u64| Some(Ret::Value(x));
        let slots = |first: u64| Va::Slots { regs: a, rsp, idx: first };
        match name {
            "__acrt_iob_func" => v(self.std_file(a[0].min(2))),
            "__iob_func" => {
                self.ensure_std();
                v(self.data.cell("_iob", 144).unwrap_or(0))
            }
            "fopen" | "_wfopen" | "fopen_s" | "_wfopen_s" => {
                let wide = name.contains("wf");
                let s = name.ends_with("_s");
                let (path_p, mode_p) = if s { (a[1], a[2]) } else { (a[0], a[1]) };
                let path = self.rd_z(path_p, wide, 512);
                let mode: String = self.rd_z(mode_p, wide, 16).iter().filter_map(|c| char::from_u32(*c)).collect();
                let path: String = path.iter().filter_map(|c| char::from_u32(*c)).collect();
                let key = files::norm(&path);
                let plus = mode.contains('+');
                let (read, write, append, trunc, create) = match mode.chars().next() {
                    Some('r') => (true, plus, false, false, false),
                    Some('w') => (plus, true, false, true, true),
                    Some('a') => (plus, true, true, false, true),
                    _ => {
                        self.set_errno_pub(22);
                        return v(if s { 22 } else { 0 });
                    }
                };
                let exists = self.vfs.contains_key(&key);
                if (!exists && !create) || path.is_empty() {
                    self.set_errno_pub(2);
                    return v(if s { 2 } else { 0 });
                }
                if !exists || trunc {
                    self.vfs.insert(key.clone(), Vec::new());
                }
                let p = self.crt_alloc(FILE_SIZE);
                if p == 0 {
                    return v(if s { 12 } else { 0 });
                }
                self.crt.next_fd = self.crt.next_fd.max(2) + 1;
                let fd = self.crt.next_fd;
                self.crt.files.insert(p, CFile { kind: Kind::Vfs { path: key, pos: 0, read, write, append }, eof: false, err: false, unget: None, fd });
                if s {
                    let _ = self.mem.write(a[0], &p.to_le_bytes());
                    return v(0);
                }
                v(p)
            }
            "fclose" => {
                self.ensure_std();
                v(if self.crt.files.remove(&a[0]).is_some() { 0 } else { EOF })
            }
            "fread" => {
                let (size, count) = (a[1], a[2]);
                let Some(total) = size.checked_mul(count).filter(|t| *t <= (64 << 20)) else { return v(0) };
                let mut got = Vec::new();
                while (got.len() as u64) < total {
                    match self.file_getc(a[3]) {
                        Some(b) => got.push(b),
                        None => break,
                    }
                }
                let _ = self.mem.write(a[0], &got);
                v(if size == 0 { 0 } else { got.len() as u64 / size })
            }
            "fwrite" => {
                let Some(total) = a[1].checked_mul(a[2]).filter(|t| *t <= (64 << 20)) else { return v(0) };
                let Ok(b) = self.mem.read_vec(a[0], total as usize) else { return v(0) };
                let n = self.file_put(a[3], &b);
                v(if a[1] == 0 { 0 } else { n as u64 / a[1] })
            }
            "fputs" | "fputws" => {
                let wide = name == "fputws";
                let s = self.rd_z(a[0], wide, usize::MAX);
                let r = self.printf_to(&s, a[1], wide);
                v(if r == u64::MAX { EOF } else { 0 })
            }
            "puts" => {
                let mut s = self.rd_z(a[0], false, usize::MAX);
                s.push(10);
                let f = self.std_file(1);
                let r = self.printf_to(&s, f, false);
                v(if r == u64::MAX { EOF } else { 0 })
            }
            "fputc" | "putc" | "putchar" => {
                let (c, f) = if name == "putchar" { (a[0], self.std_file(1)) } else { (a[0], a[1]) };
                let n = self.file_put(f, &[c as u8]);
                v(if n == 1 { u64::from(c as u8) } else { EOF })
            }
            "fgetc" | "getc" | "getchar" => {
                let f = if name == "getchar" { self.std_file(0) } else { a[0] };
                v(self.file_getc(f).map_or(EOF, u64::from))
            }
            "ungetc" => {
                self.ensure_std();
                match self.crt.files.get_mut(&a[1]) {
                    Some(f) if a[0] as i32 != -1 && f.unget.is_none() => {
                        f.unget = Some(a[0] as u8);
                        f.eof = false;
                        v(u64::from(a[0] as u8))
                    }
                    _ => v(EOF),
                }
            }
            "fgets" => {
                let cap = a[1] as i32;
                if cap <= 0 {
                    return v(0);
                }
                let mut line = Vec::new();
                while (line.len() as i32) < cap - 1 {
                    match self.file_getc(a[2]) {
                        Some(b) => {
                            line.push(b);
                            if b == b'\n' {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                if line.is_empty() {
                    return v(0);
                }
                line.push(0);
                let _ = self.mem.write(a[0], &line);
                v(a[0])
            }
            "fflush" | "_fflush_nolock" | "setvbuf" | "setbuf" | "_set_printf_count_output" => v(0),
            "feof" | "ferror" | "clearerr" => {
                self.ensure_std();
                let Some(f) = self.crt.files.get_mut(&a[0]) else { return v(0) };
                match name {
                    "feof" => v(u64::from(f.eof)),
                    "ferror" => v(u64::from(f.err)),
                    _ => {
                        f.eof = false;
                        f.err = false;
                        v(0)
                    }
                }
            }
            "fseek" | "_fseeki64" | "rewind" => {
                self.ensure_std();
                let (off, whence) = if name == "rewind" { (0i64, 0) } else { (a[1] as i64, a[2] as u32) };
                let Some(f) = self.crt.files.get_mut(&a[0]) else { return v(EOF) };
                f.unget = None;
                let Kind::Vfs { path, pos, .. } = &mut f.kind else { return v(EOF) };
                let len = self.vfs.get(path.as_str()).map_or(0, Vec::len) as i64;
                let base = match whence {
                    0 => 0,
                    1 => *pos as i64,
                    2 => len,
                    _ => return v(EOF),
                };
                let np = base + if name == "fseek" || name == "_fseeki64" { if name == "fseek" { off as i32 as i64 } else { off } } else { 0 };
                if np < 0 {
                    return v(EOF);
                }
                *pos = np as usize;
                f.eof = false;
                v(0)
            }
            "ftell" | "_ftelli64" => {
                self.ensure_std();
                match self.crt.files.get(&a[0]) {
                    Some(CFile { kind: Kind::Vfs { pos, .. }, unget, .. }) => v((*pos as u64).saturating_sub(u64::from(unget.is_some()))),
                    _ => v(EOF),
                }
            }
            "_fileno" | "fileno" => {
                self.ensure_std();
                v(self.crt.files.get(&a[0]).map_or(EOF, |f| f.fd as u32 as u64))
            }
            "_write" => {
                let Ok(b) = self.mem.read_vec(a[1], (a[2] as usize).min(64 << 20)) else { return v(EOF) };
                let f = match a[0] as i32 {
                    1 => self.std_file(1),
                    2 => self.std_file(2),
                    _ => return v(EOF),
                };
                v(self.file_put(f, &b) as u64)
            }
            "_close" => v(0),
            "remove" | "_unlink" | "unlink" => {
                let p = self.rd_z(a[0], false, 512);
                let key = files::norm(&p.iter().filter_map(|c| char::from_u32(*c)).collect::<String>());
                if self.vfs.remove(&key).is_some() {
                    v(0)
                } else {
                    self.set_errno_pub(2);
                    v(EOF)
                }
            }
            "_access" => {
                let p = self.rd_z(a[0], false, 512);
                let key = files::norm(&p.iter().filter_map(|c| char::from_u32(*c)).collect::<String>());
                v(if self.vfs.contains_key(&key) { 0 } else { EOF })
            }
            "__local_stdio_printf_options" | "__local_stdio_scanf_options" => v(self.data.cell(name, 8).unwrap_or(0)),

            // ---- printf family ----
            "printf" | "wprintf" => {
                let wide = name == "wprintf";
                let f = self.fmt_in(a[0], wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: slots(1) });
                let so = self.std_file(1);
                v(self.printf_to(&out, so, wide))
            }
            "fprintf" | "fwprintf" => {
                let wide = name == "fwprintf";
                let f = self.fmt_in(a[1], wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: slots(2) });
                v(self.printf_to(&out, a[0], wide))
            }
            "sprintf" => {
                let f = self.fmt_in(a[1], false);
                let out = fmt::format(&f, false, &mut Source { p: self, va: slots(2) });
                v(self.printf_buf(a[0], u64::MAX, &out, false, true))
            }
            "sprintf_s" => {
                let f = self.fmt_in(a[2], false);
                let out = fmt::format(&f, false, &mut Source { p: self, va: slots(3) });
                v(self.printf_buf(a[0], a[1], &out, false, false))
            }
            "snprintf" | "_snprintf" | "_snwprintf" => {
                let wide = name == "_snwprintf";
                let f = self.fmt_in(a[2], wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: slots(3) });
                v(self.printf_buf(a[0], a[1], &out, wide, name == "snprintf"))
            }
            "vfprintf" | "vfwprintf" => {
                let wide = name == "vfwprintf";
                let f = self.fmt_in(a[1], wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: Va::List(a[2]) });
                v(self.printf_to(&out, a[0], wide))
            }
            "vprintf" => {
                let f = self.fmt_in(a[0], false);
                let out = fmt::format(&f, false, &mut Source { p: self, va: Va::List(a[1]) });
                let so = self.std_file(1);
                v(self.printf_to(&out, so, false))
            }
            "vsprintf" => {
                let f = self.fmt_in(a[1], false);
                let out = fmt::format(&f, false, &mut Source { p: self, va: Va::List(a[2]) });
                v(self.printf_buf(a[0], u64::MAX, &out, false, true))
            }
            "vsnprintf" | "_vsnprintf" | "_vsnwprintf" => {
                let wide = name == "_vsnwprintf";
                let f = self.fmt_in(a[2], wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: Va::List(a[3]) });
                v(self.printf_buf(a[0], a[1], &out, wide, name == "vsnprintf"))
            }
            "__stdio_common_vfprintf" | "__stdio_common_vfwprintf" => {
                // (options, FILE*, format, locale, va_list[5])
                let wide = name.ends_with("wprintf");
                let f = self.fmt_in(a[2], wide);
                let ap = self.stack_arg(rsp, 5);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: Va::List(ap) });
                v(self.printf_to(&out, a[1], wide))
            }
            "__stdio_common_vsprintf" | "__stdio_common_vswprintf" | "__stdio_common_vsnprintf_s" => {
                // (options, buffer, count, format, locale[5], va_list[6]); `_s` adds a max-count argument before the format.
                let wide = name.ends_with("swprintf");
                let (fmt_p, ap) = if name.ends_with("_s") { (self.stack_arg(rsp, 5), self.stack_arg(rsp, 7)) } else { (a[3], self.stack_arg(rsp, 6)) };
                let f = self.fmt_in(fmt_p, wide);
                let out = fmt::format(&f, wide, &mut Source { p: self, va: Va::List(ap) });
                // _CRT_INTERNAL_PRINTF_STANDARD_SNPRINTF_BEHAVIOR (bit 1) returns the untruncated length.
                v(self.printf_buf(a[1], a[2], &out, wide, a[0] & 2 != 0))
            }
            _ => None,
        }
    }

    pub(crate) fn set_errno_pub(&mut self, e: u32) {
        if let Some(c) = self.data.cell("<errno>", 4) {
            let _ = self.mem.write(c, &e.to_le_bytes());
        }
    }
}
