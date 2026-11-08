//! Web fetch for the browser (C `web.h` contract). The freestanding kernel first uses the native network stack (HTTP/HTTPS over
//! the NIC, implemented in `kffi::webnet`) and only falls back to the QEMU development bridge on COM2 when the NIC has no address.
//! Hosted builds expose the offline stub.
#![allow(missing_docs)]

use core::ffi::c_char;

#[cfg(not(target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn loos_web_fetch(
    _url: *const c_char,
    _out: *mut c_char,
    _cap: usize,
    _out_len: *mut usize,
    _raw_len: *mut usize,
    _final_url: *mut c_char,
    _final_cap: usize,
) -> bool {
    false
}

/// Why the last fetch failed, NUL-terminated into `out`; returns the length (0 when unknown).
#[cfg(not(target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn loos_web_last_error(_out: *mut c_char, _cap: usize) -> usize {
    0
}

#[cfg(target_os = "none")]
extern "C" {
    fn loos_net_fetch(url: *const u8, out: *mut u8, cap: usize, out_len: *mut usize, raw_len: *mut usize, final_url: *mut u8, final_cap: usize) -> i32;
    fn loos_net_last_error(out: *mut u8, cap: usize) -> usize;
}

/// Why the last fetch failed, NUL-terminated into `out`; returns the length (0 when unknown).
#[cfg(target_os = "none")]
#[no_mangle]
pub unsafe extern "C" fn loos_web_last_error(out: *mut c_char, cap: usize) -> usize {
    // SAFETY: forwarded contract.
    unsafe { loos_net_last_error(out.cast(), cap) }
}

#[cfg(target_os = "none")]
mod kernel {
    use super::*;


    const COM2: u16 = 0x2f8;
    const RX_TIMEOUT: u32 = 4_000_000;
    const TX_TIMEOUT: u32 = 4_000_000;
    const MAX_URL: usize = 1400;
    const HDR_MAX: usize = 2048;

    fn outb(port: u16, v: u8) {
        // SAFETY: port I/O on the COM2 UART, the same access the C bridge used.
        unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags)); }
    }
    fn inb(port: u16) -> u8 {
        let v: u8;
        // SAFETY: as above.
        unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack, preserves_flags)); }
        v
    }

    fn init() {
        static mut READY: bool = false;
        // SAFETY: single-threaded shell; the UART is initialized once.
        unsafe {
            if READY {
                return;
            }
            outb(COM2 + 1, 0x00);
            outb(COM2 + 3, 0x80);
            outb(COM2 + 0, 0x01);
            outb(COM2 + 1, 0x00);
            outb(COM2 + 3, 0x03);
            outb(COM2 + 2, 0xc7);
            outb(COM2 + 4, 0x0b);
            READY = true;
        }
    }
    fn putc(c: u8) -> bool {
        for _ in 0..TX_TIMEOUT {
            if inb(COM2 + 5) & 0x20 != 0 {
                outb(COM2, c);
                return true;
            }
        }
        false
    }
    fn puts(s: &[u8]) -> bool {
        for &c in s {
            if !putc(c) {
                return false;
            }
        }
        true
    }
    fn getc() -> i32 {
        for _ in 0..RX_TIMEOUT {
            if inb(COM2 + 5) & 0x01 != 0 {
                return inb(COM2) as i32;
            }
        }
        -1
    }
    fn fail(_msg: &[u8]) -> bool {
        false
    }
    fn parse_u32(s: &[u8]) -> Option<(usize, u32)> {
        let mut v: u32 = 0;
        let mut i = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            if v > u32::MAX / 10 {
                return None;
            }
            v = v * 10 + (s[i] - b'0') as u32;
            i += 1;
        }
        if i == 0 { None } else { Some((i, v)) }
    }
    fn strlen(p: *const c_char) -> usize {
        let mut n = 0;
        // SAFETY: caller contract gives a NUL-terminated string.
        while unsafe { *p.add(n) } != 0 {
            n += 1;
        }
        n
    }

    pub unsafe fn fetch(url: *const c_char, out: *mut c_char, cap: usize, out_len: *mut usize, raw_len: *mut usize, final_url: *mut c_char, final_cap: usize) -> bool {
        if url.is_null() || out.is_null() || out_len.is_null() || raw_len.is_null() || final_url.is_null() || final_cap == 0 {
            return false;
        }
        // SAFETY: output pointers are caller-owned; lengths per the C contract.
        unsafe {
            *out_len = 0;
            *raw_len = 0;
            *final_url = 0;
        }
        let ulen = strlen(url);
        if ulen == 0 || ulen > MAX_URL {
            return false;
        }
        // SAFETY: native fetch contract: same buffers, NUL-terminated URL. 1 = done, -1 = failed with a reason, 0 = no native network.
        match unsafe { super::loos_net_fetch(url.cast(), out.cast(), cap, out_len, raw_len, final_url.cast(), final_cap) } {
            1 => return true,
            -1 => return false,
            _ => {}
        }
        // SAFETY: ulen bytes before the NUL.
        let url = unsafe { core::slice::from_raw_parts(url as *const u8, ulen) };
        init();
        if !puts(b"WGET ") || !puts(url) || !putc(b'\n') {
            return false;
        }
        let mut hdr = [0u8; HDR_MAX];
        let mut n = 0usize;
        loop {
            let c = getc();
            if c < 0 {
                return false;
            }
            if c == b'\n' as i32 {
                break;
            }
            if c == b'\r' as i32 {
                continue;
            }
            if n + 1 >= hdr.len() {
                return false;
            }
            hdr[n] = c as u8;
            n += 1;
        }
        if n < 3 || &hdr[..3] != b"OK " {
            return false;
        }
        let (used, clen) = match parse_u32(&hdr[3..n]) {
            Some(v) => v,
            None => return fail(b"web: bad compressed length\n\0"),
        };
        let mut p = 3 + used;
        if p >= n || hdr[p] != b' ' {
            return false;
        }
        let (used2, rlen) = match parse_u32(&hdr[p + 1..n]) {
            Some(v) => v,
            None => return fail(b"web: bad raw length\n\0"),
        };
        p += 1 + used2;
        if p >= n || hdr[p] != b' ' {
            return false;
        }
        p += 1;
        let fl = (n - p).min(final_cap - 1);
        unsafe {
            core::ptr::copy_nonoverlapping(hdr[p..p + fl].as_ptr(), final_url as *mut u8, fl);
            *final_url.add(fl) = 0;
        }
        if clen == 0 || clen as usize > cap || rlen == 0 {
            return false;
        }
        for i in 0..clen as usize {
            let c = getc();
            if c < 0 {
                return false;
            }
            unsafe { *out.add(i) = c as c_char };
        }
        unsafe {
            *out_len = clen as usize;
            *raw_len = rlen as usize;
        }
        true
    }
}

#[cfg(target_os = "none")]
#[no_mangle]
pub unsafe extern "C" fn loos_web_fetch(
    url: *const c_char,
    out: *mut c_char,
    cap: usize,
    out_len: *mut usize,
    raw_len: *mut usize,
    final_url: *mut c_char,
    final_cap: usize,
) -> bool {
    // SAFETY: kernel bridge contract from web.h.
    unsafe { kernel::fetch(url, out, cap, out_len, raw_len, final_url, final_cap) }
}
