//! Guest file set: an in-memory volume (`C:\...`) behind CreateFile/ReadFile/WriteFile/FindFirstFile, plus the
//! open-file and find-handle tables. Persistence is the kernel's business: it seeds [`Process::vfs`] and reads it back.

use crate::{Process, Ret};
use alloc::string::String;
use alloc::vec::Vec;

const INVALID: u64 = u64::MAX;
/// First file handle value; find handles start at [`FIND_BASE`].
pub const FILE_BASE: u64 = 0x100;
const FIND_BASE: u64 = 0x8000;

/// An open file.
pub struct OpenFile {
    path: String,
    pos: u64,
}

/// Normalises a Windows path to the key used in [`Process::vfs`]: `/dir/file`, lower case, no drive letter.
pub fn norm(p: &str) -> String {
    let mut s: String = p.replace('\\', "/").to_lowercase();
    for pre in ["//?/", "//./"] {
        if let Some(r) = s.strip_prefix(pre) {
            s = r.into();
        }
    }
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        s.drain(..2);
    }
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

fn matches(pat: &[u8], name: &[u8]) -> bool {
    match (pat.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => matches(&pat[1..], name) || (!name.is_empty() && matches(pat, &name[1..])),
        (Some(b'?'), Some(_)) => matches(&pat[1..], &name[1..]),
        (Some(a), Some(b)) if a == b => matches(&pat[1..], &name[1..]),
        _ => false,
    }
}

impl Process {
    fn is_dir(&self, p: &str) -> bool {
        let pre = if p == "/" { String::from("/") } else { alloc::format!("{p}/") };
        p == "/" || self.vfs.keys().any(|k| k.starts_with(&pre))
    }

    fn wpath(&mut self, a: u64) -> Option<String> {
        let mut v = Vec::new();
        for i in 0..520u64 {
            let mut b = [0u8; 2];
            self.mem.read(a + i * 2, &mut b).ok()?;
            let c = u16::from_le_bytes(b);
            if c == 0 {
                return Some(norm(&String::from_utf16_lossy(&v)));
            }
            v.push(c);
        }
        None
    }

    fn file_of(&self, h: u64) -> Option<usize> {
        let i = h.checked_sub(FILE_BASE)? / 4;
        (h < FIND_BASE && self.open.get(i as usize)?.is_some()).then_some(i as usize)
    }

    fn find_data(&mut self, out: u64, path: &str) {
        let mut b = alloc::vec![0u8; 592];
        let dir = self.is_dir(path) && !self.vfs.contains_key(path);
        let len = self.vfs.get(path).map_or(0, |f| f.len() as u64);
        b[0..4].copy_from_slice(&(if dir { 0x10u32 } else { 0x80 }).to_le_bytes());
        b[28..32].copy_from_slice(&((len >> 32) as u32).to_le_bytes());
        b[32..36].copy_from_slice(&(len as u32).to_le_bytes());
        let name = path.rsplit('/').next().unwrap_or("");
        for (i, c) in name.encode_utf16().take(259).enumerate() {
            b[44 + i * 2..46 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        let _ = self.mem.write(out, &b);
    }

    fn find_start(&mut self, pat: u64, out: u64) -> u64 {
        let Some(full) = self.wpath(pat) else { return INVALID };
        let (dir, pattern) = full.rsplit_once('/').map_or(("", full.as_str()), |(d, p)| (d, p));
        let dir = if dir.is_empty() { "/" } else { dir };
        let mut found: Vec<String> = Vec::new();
        let prefix = if dir == "/" { String::from("/") } else { alloc::format!("{dir}/") };
        for k in self.vfs.keys() {
            if let Some(rest) = k.strip_prefix(&prefix) {
                let first = rest.split('/').next().unwrap_or("");
                let entry = alloc::format!("{prefix}{first}");
                if matches(pattern.as_bytes(), first.as_bytes()) && !found.contains(&entry) {
                    found.push(entry);
                }
            }
        }
        if found.is_empty() {
            self.set_last_error(2);
            return INVALID;
        }
        self.find_data(out, &found[0]);
        self.finds.push(Some((found, 1)));
        FIND_BASE + (self.finds.len() as u64 - 1) * 4
    }

    /// File-system calls. `None` when `name` is not one of them.
    pub(crate) fn file_syscall(&mut self, name: &str, a: [u64; 4], rsp: u64) -> Option<Ret> {
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "CreateFileW" => {
                let path = self.wpath(a[0])?;
                let disposition = self.stack_arg(rsp, 5);
                let exists = self.vfs.contains_key(&path);
                let ok = match disposition {
                    1 if exists => {
                        self.set_last_error(80);
                        false
                    }
                    1 | 4 => true,
                    2 => {
                        if exists {
                            self.set_last_error(183);
                        }
                        true
                    }
                    3 | 5 if !exists => {
                        self.set_last_error(2);
                        false
                    }
                    _ => true,
                };
                if !ok {
                    return v(INVALID);
                }
                if !exists || disposition == 2 || disposition == 5 {
                    self.vfs.insert(path.clone(), Vec::new());
                }
                self.open.push(Some(OpenFile { path, pos: 0 }));
                v(FILE_BASE + (self.open.len() as u64 - 1) * 4)
            }
            "ReadFile" => {
                let Some(i) = self.file_of(a[0]) else { return v(0) };
                let (path, pos) = (self.open[i].as_ref()?.path.clone(), self.open[i].as_ref()?.pos as usize);
                let data = self.vfs.get(&path)?;
                let n = (a[2] as usize).min(data.len().saturating_sub(pos));
                let chunk = data[pos..pos + n].to_vec();
                let _ = self.mem.write(a[1], &chunk);
                if a[3] != 0 {
                    let _ = self.mem.write(a[3], &(n as u32).to_le_bytes());
                }
                self.open[i].as_mut()?.pos += n as u64;
                v(1)
            }
            "GetFileSize" | "GetFileSizeEx" => {
                let Some(i) = self.file_of(a[0]) else { return v(0xffff_ffff) };
                let len = self.vfs.get(&self.open[i].as_ref()?.path)?.len() as u64;
                if name == "GetFileSizeEx" {
                    let _ = self.mem.write(a[1], &len.to_le_bytes());
                    return v(1);
                }
                if a[1] != 0 {
                    let _ = self.mem.write(a[1], &((len >> 32) as u32).to_le_bytes());
                }
                v(len & 0xffff_ffff)
            }
            "SetFilePointerEx" => {
                let Some(i) = self.file_of(a[0]) else { return v(0) };
                let len = self.vfs.get(&self.open[i].as_ref()?.path)?.len() as i64;
                let f = self.open[i].as_mut()?;
                let base = match a[3] {
                    1 => f.pos as i64,
                    2 => len,
                    _ => 0,
                };
                f.pos = (base + a[1] as i64).max(0) as u64;
                if a[2] != 0 {
                    let p = f.pos;
                    let _ = self.mem.write(a[2], &p.to_le_bytes());
                }
                v(1)
            }
            "SetEndOfFile" => {
                let Some(i) = self.file_of(a[0]) else { return v(0) };
                let f = self.open[i].as_ref()?;
                let (path, pos) = (f.path.clone(), f.pos as usize);
                self.vfs.get_mut(&path)?.resize(pos, 0);
                v(1)
            }
            "CloseHandle" => {
                if let Some(i) = self.file_of(a[0]) {
                    self.open[i] = None;
                } else if a[0] >= FIND_BASE {
                    self.close_obj(a[0]);
                    return v(1);
                }
                v(1)
            }
            "GetFileAttributesW" => {
                let path = self.wpath(a[0])?;
                if self.vfs.contains_key(&path) {
                    return v(0x80);
                }
                if self.is_dir(&path) {
                    return v(0x10);
                }
                self.set_last_error(2);
                v(0xffff_ffff)
            }
            "DeleteFileW" => {
                let path = self.wpath(a[0])?;
                if self.vfs.remove(&path).is_some() {
                    return v(1);
                }
                self.set_last_error(2);
                v(0)
            }
            "CopyFileW" => {
                let (src, dst) = (self.wpath(a[0])?, self.wpath(a[1])?);
                let Some(d) = self.vfs.get(&src).cloned() else {
                    self.set_last_error(2);
                    return v(0);
                };
                if a[2] != 0 && self.vfs.contains_key(&dst) {
                    self.set_last_error(80);
                    return v(0);
                }
                self.vfs.insert(dst, d);
                v(1)
            }
            "CreateDirectoryW" => v(1),
            "FindFirstFileW" => v(self.find_start(a[0], a[1])),
            "FindFirstFileExW" => v(self.find_start(a[0], a[2])),
            "FindNextFileW" => {
                let i = a[0].checked_sub(FIND_BASE)? as usize / 4;
                let (list, at) = self.finds.get_mut(i)?.as_mut()?;
                if *at >= list.len() {
                    self.set_last_error(18);
                    return v(0);
                }
                let p = list[*at].clone();
                *at += 1;
                self.find_data(a[1], &p);
                v(1)
            }
            "FindClose" => {
                if let Some(s) = a[0].checked_sub(FIND_BASE).and_then(|o| self.finds.get_mut(o as usize / 4)) {
                    *s = None;
                }
                v(1)
            }
            _ => None,
        }
    }

    /// `WriteFile` to a file handle (std handles are handled by the caller). `None` if `a[0]` is not a file.
    pub(crate) fn file_write(&mut self, a: [u64; 4]) -> Option<Ret> {
        let i = self.file_of(a[0])?;
        let data = self.mem.read_vec(a[1], (a[2] as usize).min(1 << 24)).ok()?;
        let f = self.open[i].as_mut()?;
        let (path, pos) = (f.path.clone(), f.pos as usize);
        f.pos += data.len() as u64;
        let file = self.vfs.get_mut(&path)?;
        if file.len() < pos + data.len() {
            file.resize(pos + data.len(), 0);
        }
        file[pos..pos + data.len()].copy_from_slice(&data);
        if a[3] != 0 {
            let _ = self.mem.write(a[3], &(data.len() as u32).to_le_bytes());
        }
        Some(Ret::Value(1))
    }
}
