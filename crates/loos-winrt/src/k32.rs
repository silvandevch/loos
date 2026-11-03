//! kernel32 breadth for CRT start-up and GUI programs: heap arena, TLS/FLS, string conversion, environment,
//! time, and "not available" answers for file/process APIs the single-process sandbox does not provide yet.

use crate::{Process, Ret};
use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::PROT_EXEC;

/// First syscall number of this table.
pub const K32_BASE: u32 = 0x400;
const HEAP_SIZE: u64 = 256 << 20;
const INVALID_HANDLE: u64 = u64::MAX;

const NAMES: &[&str] = &[
    "HeapAlloc", "HeapReAlloc", "HeapSize", "HeapFree", "HeapCompact", "GlobalAlloc", "GlobalFree", "GlobalLock", "GlobalUnlock",
    "LocalAlloc", "LocalFree", "FlsGetValue",
    "InitializeCriticalSection",
    "EncodePointer",
    "DecodePointer", "GetStartupInfoW", "GetStartupInfoA", "GetCommandLineW", "GetEnvironmentStringsW",
    "FreeEnvironmentStringsW", "GetEnvironmentVariableW", "GetCPInfo", "GetOEMCP", "GetACP", "IsValidCodePage",
    "GetFileType", "GetConsoleMode", "GetConsoleOutputCP", "WriteConsoleW", "SetStdHandle", "FlushFileBuffers",
    "GetStringTypeW", "LCMapStringW", "WideCharToMultiByte", "MultiByteToWideChar", "FindFirstFileExW", "FindFirstFileW",
    "FindNextFileW", "FindClose", "GetFileAttributesW", "CreateFileW", "CreateDirectoryW", "DeleteFileW", "CopyFileW",
    "ReadFile", "GetFileSize", "SetFilePointerEx", "CloseHandle", "GetPrivateProfileIntW", "GetPrivateProfileStringW",
    "WritePrivateProfileStringW", "GetTimeFormatW", "GetDateFormatW", "FormatMessageW", "MulDiv", "GetModuleHandleExW",
    "GetModuleFileNameW", "LoadLibraryW", "LoadLibraryExW", "FreeLibrary", "GetProcAddress", "SetProcessWorkingSetSize",
    "GetExitCodeProcess", "CreateProcessW", "GlobalMemoryStatusEx", "GetLocalTime",
    "GetSystemTimeAsFileTime", "QueryPerformanceCounter", "QueryPerformanceFrequency", "RtlCaptureContext",
    "RtlLookupFunctionEntry", "RtlVirtualUnwind", "RtlUnwindEx", "RtlPcToFileHeader", "RaiseException", "IsDebuggerPresent",
    "SetUnhandledExceptionFilter", "UnhandledExceptionFilter", "IsProcessorFeaturePresent", "InitializeSListHead",
    "VirtualProtect", "OutputDebugStringW", "lstrlenW", "lstrcpyW", "lstrcmpW", "lstrcmpiW", "GetSystemInfo",
    "GetVersionExW", "GetUserDefaultLCID", "GetThreadLocale", "GetLocaleInfoW", "MulDivDummy",
    // user32/gdi32/shell32/advapi32/comctl32/comdlg32 answers that need a specific non-zero value
    "GetSysColor", "GetSysColorBrush", "GetCursorPos", "GetDeviceCaps", "InitCommonControlsEx", "CoInitializeEx",
    "CoUninitialize", "RegOpenKeyExW", "RegQueryValueExW", "RegCloseKey", "RegCreateKeyW", "RegSetValueExW",
    "SHGetFolderPathW", "CharNextW", "GetTextExtentPoint32W", "CreateFontIndirectW", "CreateFontW", "GetTextFaceW",
    "IsWindowVisible", "EnableWindow", "SetWindowTheme", "DragAcceptFiles", "MessageBeep", "SetEndOfFile", "GetFileSizeEx",
    "GetOpenFileNameW", "GetSaveFileNameW", "CommDlgExtendedError", "ChooseFontW", "PrintDlgW", "ChooseColorW", "LoadLibraryA", "LoadLibraryExA",
    "GetModuleHandleA", "__C_specific_handler",
];

/// Syscall number of a kernel32-family import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| K32_BASE + i as u32)
}

fn put_wide(s: &str, out: &mut Vec<u8>) {
    for c in s.encode_utf16() {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

impl Process {
    pub(crate) fn heap_alloc(&mut self, n: u64) -> u64 {
        if self.heap_base == 0 {
            self.heap_base = self.mem.map_anywhere(HEAP_SIZE, 3, &[], "heap").unwrap_or(0);
            self.heap_next = self.heap_base;
        }
        let total = (n.max(1) + 15 & !15) + 16;
        if self.heap_base == 0 || self.heap_next + total > self.heap_base + HEAP_SIZE {
            return 0;
        }
        let block = self.heap_next;
        self.heap_next += total;
        let _ = self.mem.write(block, &n.to_le_bytes());
        block + 16
    }

    fn block_size(&mut self, p: u64) -> u64 {
        if p < 16 { 0 } else { self.mem.read_u64(p - 16).unwrap_or(0) }
    }

    pub(crate) fn stack_arg(&mut self, rsp: u64, n: u64) -> u64 {
        self.mem.read_u64(rsp + 0x28 + 8 * (n - 5)).unwrap_or(0)
    }

    fn wstr_at(&mut self, a: u64, max: usize) -> Option<String> {
        let mut v = Vec::new();
        for i in 0..max as u64 {
            let mut b = [0u8; 2];
            self.mem.read(a + i * 2, &mut b).ok()?;
            let c = u16::from_le_bytes(b);
            if c == 0 {
                break;
            }
            v.push(c);
        }
        Some(String::from_utf16_lossy(&v))
    }

    /// kernel32-family calls with CRT/GUI-start-up semantics. Returns `None` for numbers outside this table.
    pub fn k32_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now_ms: u64) -> Option<Ret> {
        let name = *NAMES.get(nr.checked_sub(K32_BASE)? as usize)?;
        let v = |x: u64| Some(Ret::Value(x));
        if let Some(r) = self.file_syscall(name, a, rsp) {
            return Some(r);
        }
        if let Some(r) = self.seh_syscall(name, a, rsp) {
            return Some(r);
        }
        match name {
            "LoadLibraryW" | "LoadLibraryExW" | "LoadLibraryA" | "LoadLibraryExA" => {
                let n = if name.ends_with('W') { self.wstr_at(a[0], 520) } else { self.mem.read_cstr(a[0], 520).ok() };
                let h = n.and_then(|n| self.load_library(&n)).unwrap_or(0);
                return Some(if h != 0 && !self.pending_init.is_empty() { Ret::Init(h) } else { Ret::Value(h) });
            }
            "GetModuleHandleA" => {
                let h = if a[0] == 0 { self.base } else { self.mem.read_cstr(a[0], 520).ok().and_then(|n| self.module_handle(&n)).unwrap_or(0) };
                return v(h);
            }
            "GetProcAddress" => {
                let r = if a[1] < 0x1_0000 { self.proc_address(a[0], None, a[1] as u32) } else {
                    let n = self.mem.read_cstr(a[1], 256).unwrap_or_default();
                    self.proc_address(a[0], Some(&n), 0)
                };
                return v(r);
            }
            "FreeLibrary" => return v(u64::from(self.free_library(a[0]))),
            "GetOpenFileNameW" | "GetSaveFileNameW" => {
                let path = if name == "GetOpenFileNameW" { self.dialog_open.clone() } else { self.dialog_save.clone() };
                if path.is_empty() {
                    return v(0); // cancelled
                }
                let (buf, cap) = (self.mem.read_u64(a[0] + 48).unwrap_or(0), self.mem.read_u32(a[0] + 56).unwrap_or(0) as usize);
                let mut b = Vec::new();
                put_wide(&path.chars().take(cap.saturating_sub(1)).collect::<String>(), &mut b);
                let _ = self.mem.write(buf, &b);
                let off = path.rfind('\\').map_or(0, |i| i + 1) as u16;
                let ext = path.rfind('.').map_or(0, |i| i + 1) as u16;
                let _ = self.mem.write(a[0] + 60, &[off.to_le_bytes(), ext.to_le_bytes()].concat());
                return v(1);
            }
            "CommDlgExtendedError" | "ChooseFontW" | "PrintDlgW" | "ChooseColorW" => return v(0),
            _ => {}
        }
        match name {
            "HeapAlloc" | "GlobalAlloc" | "LocalAlloc" => {
                // HeapAlloc(heap, flags, n) vs GlobalAlloc(flags, n): the arena is zero-filled and never reused.
                let n = if name == "HeapAlloc" { a[2] } else { a[1] };
                v(self.heap_alloc(n))
            }
            "HeapReAlloc" => {
                let old = self.block_size(a[2]);
                let new = self.heap_alloc(a[3]);
                if new != 0 && old != 0 {
                    if let Ok(d) = self.mem.read_vec(a[2], old.min(a[3]) as usize) {
                        let _ = self.mem.write(new, &d);
                    }
                }
                v(new)
            }
            "HeapSize" => v(self.block_size(a[2])),
            "HeapFree" | "GlobalFree" | "LocalFree" | "GlobalUnlock" => v(if name == "HeapFree" || name == "GlobalUnlock" { 1 } else { 0 }),
            "GlobalLock" => v(a[0]),
            "HeapCompact" => v(0),
            "TlsAlloc" | "FlsAlloc" => {
                self.tls.push(0);
                v(self.tls.len() as u64 - 1)
            }
            "TlsFree" | "FlsFree" => v(1),
            "TlsGetValue" | "FlsGetValue" => v(self.tls.get(a[0] as usize).copied().unwrap_or(0)),
            "TlsSetValue" | "FlsSetValue" => {
                if let Some(s) = self.tls.get_mut(a[0] as usize) {
                    *s = a[1];
                }
                v(1)
            }
            "InitializeCriticalSectionEx" | "InitializeCriticalSectionAndSpinCount" | "TryEnterCriticalSection" => v(1),
            "InitializeCriticalSection" | "EnterCriticalSection" | "LeaveCriticalSection" | "DeleteCriticalSection" => v(0),
            "EncodePointer" | "DecodePointer" => v(a[0]),
            "GetStartupInfoW" | "GetStartupInfoA" => {
                let mut b = [0u8; 104];
                b[0..4].copy_from_slice(&104u32.to_le_bytes());
                let _ = self.mem.write(a[0], &b);
                v(0)
            }
            "GetCommandLineW" => {
                if self.cmdline_w == 0 {
                    let line = self.mem.read_cstr(self.cmdline_addr(), 4096).unwrap_or_default();
                    let mut b = Vec::new();
                    put_wide(&line, &mut b);
                    self.cmdline_w = self.mem.map_anywhere(b.len() as u64, 3, &b, "cmdlinew").unwrap_or(0);
                }
                v(self.cmdline_w)
            }
            "GetEnvironmentStringsW" => {
                if self.env_w == 0 {
                    self.env_w = self.mem.map_anywhere(8, 3, &[], "envw").unwrap_or(0);
                }
                v(self.env_w)
            }
            "FreeEnvironmentStringsW" | "IsValidCodePage" | "SetStdHandle" | "FlushFileBuffers" | "WriteConsoleW" | "CloseHandle"
            | "FindClose" | "FreeLibrary" | "SetProcessWorkingSetSize" | "VirtualProtect" | "InitCommonControlsEx" | "IsWindowVisible"
            | "IsDebuggerPresent_" | "CoInitializeEx_" => {
                if name == "VirtualProtect" && a[3] != 0 {
                    let _ = self.mem.write(a[3], &4u32.to_le_bytes());
                }
                if name == "WriteConsoleW" && a[3] != 0 {
                    let _ = self.mem.write(a[3], &(a[2] as u32).to_le_bytes());
                }
                v(1)
            }
            "GetEnvironmentVariableW" | "GetTimeFormatW" | "GetDateFormatW" | "FormatMessageW" | "IsDebuggerPresent" | "WaitForSingleObject"
            | "SetUnhandledExceptionFilter" | "UnhandledExceptionFilter" | "IsProcessorFeaturePresent" | "RtlLookupFunctionEntry"
            | "RtlVirtualUnwind" | "RtlUnwindEx" | "RtlCaptureContext" | "InitializeSListHead" | "OutputDebugStringW" | "GetFileType_"
            | "GetConsoleMode" | "GetExitCodeProcess" | "CreateProcessW" | "CreateDirectoryW" | "DeleteFileW" | "CopyFileW" | "ReadFile"
            | "SetFilePointerEx" | "WritePrivateProfileStringW" | "LoadLibraryW" | "LoadLibraryExW" | "GetProcAddress" | "RegOpenKeyExW"
            | "RegQueryValueExW" | "RegCreateKeyW" | "RegSetValueExW" | "SHGetFolderPathW" | "MessageBeep" | "EnableWindow"
            | "SetWindowTheme" | "DragAcceptFiles" | "CreateFontIndirectW" | "CreateFontW" | "GetTextFaceW" | "MulDivDummy" => {
                if matches!(name, "LoadLibraryW" | "LoadLibraryExW" | "GetProcAddress") {
                    self.set_last_error(if name == "GetProcAddress" { 127 } else { 126 });
                }
                if matches!(name, "RegOpenKeyExW" | "RegQueryValueExW" | "RegCreateKeyW") {
                    return v(2); // ERROR_FILE_NOT_FOUND: the program falls back to defaults
                }
                if matches!(name, "CreateFontIndirectW" | "CreateFontW") {
                    return v(0x30);
                }
                v(0)
            }
            "GetFileType" => v(if matches!(a[0], 0x10 | 0x14 | 0x18) { 2 } else { 0 }),
            "GetConsoleOutputCP" => v(437),
            "GetOEMCP" | "GetACP" => v(1252),
            "GetCPInfo" => {
                let mut b = [0u8; 18];
                b[0] = 1;
                b[4] = b'?';
                let _ = self.mem.write(a[1], &b);
                v(1)
            }
            "GetStringTypeW" => {
                let n = if a[2] as i32 == -1 { 0 } else { a[2] as usize };
                let _ = self.mem.write(a[3], &alloc::vec![0u8; n * 2]);
                v(1)
            }
            "LCMapStringW" => {
                let src_len = a[3] as i32;
                let n = if src_len < 0 { 0 } else { src_len as usize };
                let dst = self.stack_arg(rsp, 5);
                let cap = self.stack_arg(rsp, 6) as usize;
                if cap != 0 && n != 0 {
                    if let Ok(d) = self.mem.read_vec(a[2], n * 2) {
                        let _ = self.mem.write(dst, &d[..d.len().min(cap * 2)]);
                    }
                }
                v(n as u64)
            }
            "WideCharToMultiByte" => {
                // (cp, flags, wstr, wlen, out, outlen, defchar, used): narrow to the low byte.
                let wlen = a[3] as i32;
                let (out, cap) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6) as usize);
                let src = if wlen < 0 { self.wstr_at(a[2], 1 << 16).map_or(0, |s| s.encode_utf16().count()) + 1 } else { wlen as usize };
                if cap == 0 {
                    return v(src as u64);
                }
                let raw = self.mem.read_vec(a[2], src * 2).unwrap_or_default();
                let b: Vec<u8> = raw.chunks(2).take(cap).map(|c| if c.len() == 2 && c[1] == 0 { c[0] } else { b'?' }).collect();
                let _ = self.mem.write(out, &b);
                v(b.len() as u64)
            }
            "MultiByteToWideChar" => {
                // (cp, flags, str, len, out, outlen)
                let len = a[3] as i32;
                let (out, cap) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6) as usize);
                let src = if len < 0 { self.mem.read_cstr(a[2], 1 << 16).map_or(0, |s| s.len()) + 1 } else { len as usize };
                if cap == 0 {
                    return v(src as u64);
                }
                let raw = self.mem.read_vec(a[2], src).unwrap_or_default();
                let b: Vec<u8> = raw.iter().take(cap).flat_map(|&c| [c, 0]).collect();
                let _ = self.mem.write(out, &b);
                v(b.len() as u64 / 2)
            }
            "FindFirstFileExW" | "FindFirstFileW" | "CreateFileW" => {
                self.set_last_error(2);
                v(INVALID_HANDLE)
            }
            "FindNextFileW" => v(0),
            "GetFileAttributesW" => {
                self.set_last_error(2);
                v(0xffff_ffff)
            }
            "GetFileSize" => v(0xffff_ffff),
            "GetPrivateProfileIntW" => v(a[2]),
            "GetPrivateProfileStringW" => {
                let def = self.wstr_at(a[2], 1024).unwrap_or_default();
                let cap = self.stack_arg(rsp, 5) as usize;
                let mut b = Vec::new();
                put_wide(&def.chars().take(cap.saturating_sub(1)).collect::<String>(), &mut b);
                let _ = self.mem.write(a[3], &b);
                v((b.len() / 2 - 1) as u64)
            }
            "MulDiv" => {
                let (x, y, z) = (a[0] as i32 as i64, a[1] as i32 as i64, a[2] as i32 as i64);
                v(if z == 0 { u64::MAX } else { ((x * y + z / 2) / z) as u64 })
            }
            "GetModuleHandleExW" => {
                if a[2] != 0 {
                    let _ = self.mem.write(a[2], &self.base.to_le_bytes());
                }
                v(1)
            }
            "GetModuleFileNameW" => {
                let mut b = Vec::new();
                put_wide(&alloc::format!("C:\\Windows\\System32\\{}", self.name), &mut b);
                let n = (b.len() / 2).min(a[2] as usize);
                let _ = self.mem.write(a[1], &b[..n * 2]);
                v(n as u64 - 1)
            }
            "RtlPcToFileHeader" => {
                if a[1] != 0 {
                    let _ = self.mem.write(a[1], &self.base.to_le_bytes());
                }
                v(self.base)
            }
            "RaiseException" => {
                let msg = alloc::format!("[winrt] {}: RaiseException({:#x})\n", self.name, a[0]);
                self.console_write(msg.as_bytes());
                Some(Ret::Exit(a[0] as u32))
            }
            "GlobalMemoryStatusEx" => {
                let mut b = [0u8; 64];
                b[0..4].copy_from_slice(&64u32.to_le_bytes());
                b[8..16].copy_from_slice(&(512u64 << 20).to_le_bytes());
                b[16..24].copy_from_slice(&(256u64 << 20).to_le_bytes());
                let _ = self.mem.write(a[0], &b);
                v(1)
            }
            "GetLocalTime" => {
                let mut b = [0u8; 16];
                for (i, x) in [2026u16, 11, 2, 3, 12, 0, 0, 0].iter().enumerate() {
                    b[i * 2..i * 2 + 2].copy_from_slice(&x.to_le_bytes());
                }
                let _ = self.mem.write(a[0], &b);
                v(0)
            }
            "GetSystemTimeAsFileTime" => {
                let t = 0x01DC_0000_0000_0000u64 + now_ms * 10_000;
                let _ = self.mem.write(a[0], &t.to_le_bytes());
                v(0)
            }
            "QueryPerformanceCounter" => {
                let _ = self.mem.write(a[0], &(now_ms * 1000).to_le_bytes());
                v(1)
            }
            "QueryPerformanceFrequency" => {
                let _ = self.mem.write(a[0], &1_000_000u64.to_le_bytes());
                v(1)
            }
            "lstrlenW" => v(self.wstr_at(a[0], 1 << 16).map_or(0, |s| s.encode_utf16().count() as u64)),
            "lstrcpyW" => {
                let s = self.wstr_at(a[1], 1 << 16).unwrap_or_default();
                let mut b = Vec::new();
                put_wide(&s, &mut b);
                let _ = self.mem.write(a[0], &b);
                v(a[0])
            }
            "lstrcmpW" | "lstrcmpiW" => {
                let (x, y) = (self.wstr_at(a[0], 1 << 16).unwrap_or_default(), self.wstr_at(a[1], 1 << 16).unwrap_or_default());
                let (x, y) = if name == "lstrcmpiW" { (x.to_lowercase(), y.to_lowercase()) } else { (x, y) };
                v(match x.cmp(&y) {
                    core::cmp::Ordering::Less => u64::MAX,
                    core::cmp::Ordering::Equal => 0,
                    core::cmp::Ordering::Greater => 1,
                })
            }
            "GetSystemInfo" => {
                let mut b = [0u8; 48];
                b[4..8].copy_from_slice(&4096u32.to_le_bytes());
                b[8..16].copy_from_slice(&0x10000u64.to_le_bytes());
                b[16..24].copy_from_slice(&0x7fff_ffff_0000u64.to_le_bytes());
                b[24..32].copy_from_slice(&1u64.to_le_bytes());
                b[32..36].copy_from_slice(&1u32.to_le_bytes());
                b[36..40].copy_from_slice(&8664u32.to_le_bytes());
                b[44..48].copy_from_slice(&0x10000u32.to_le_bytes());
                let _ = self.mem.write(a[0], &b);
                v(0)
            }
            "GetVersionExW" => v(0),
            "GetUserDefaultLCID" | "GetThreadLocale" => v(0x409),
            "GetLocaleInfoW" => v(0),
            "GetSysColor" => v(0x00F0_F0F0),
            "GetSysColorBrush" | "CoUninitialize" => v(0x20),
            "GetCursorPos" => {
                let _ = self.mem.write(a[0], &[0u8; 8]);
                v(1)
            }
            "GetDeviceCaps" => v(match a[1] {
                8 => 1024, // HORZRES
                10 => 768, // VERTRES
                88 | 90 => 96, // LOGPIXELSX / LOGPIXELSY
                12 => 32,  // BITSPIXEL
                _ => 0,
            }),
            "CoInitializeEx" => v(0),
            "CharNextW" => v(if self.mem.read_vec(a[0], 2).is_ok_and(|b| b == [0, 0]) { a[0] } else { a[0] + 2 }),
            "GetTextExtentPoint32W" => {
                // 8x16 cells: (hdc, str, count, SIZE*)
                let mut b = [0u8; 8];
                b[0..4].copy_from_slice(&((a[2] as u32).saturating_mul(8)).to_le_bytes());
                b[4..8].copy_from_slice(&16u32.to_le_bytes());
                let _ = self.mem.write(a[3], &b);
                v(1)
            }
            _ => {
                let _ = PROT_EXEC;
                v(0)
            }
        }
    }
}
