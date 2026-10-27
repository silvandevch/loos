//! SHA-256 / HMAC / PBKDF2 with the exact symbols of the old `src/security/crypto.c` and `kdf.c`.

use crate::{bytes, bytes_mut};
use loos_security::hash::{self, Sha256};

const CTX: usize = 128;

#[repr(C, align(16))]
pub struct LoosSha256 {
    opaque: [u8; CTX],
}

const _: () = assert!(std::mem::size_of::<Sha256>() <= CTX && std::mem::align_of::<Sha256>() <= 16);

#[repr(C)]
pub struct LoosHmac {
    inner: LoosSha256,
    outer: LoosSha256,
}

unsafe fn ctx<'a>(p: *mut LoosSha256) -> &'a mut Sha256 {
    // SAFETY: `p` was initialized by loos_sha256_init, which wrote a Sha256 at this (16-aligned) address.
    unsafe { &mut *(p as *mut Sha256) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_sha256_init(c: *mut LoosSha256) {
    // SAFETY: `c` points to writable, suitably aligned storage of CTX bytes.
    unsafe { std::ptr::write(c as *mut Sha256, Sha256::new()) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_sha256_update(c: *mut LoosSha256, data: *const u8, len: usize) {
    // SAFETY: see module contract.
    unsafe { ctx(c).update(bytes(data, len)) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_sha256_final(c: *mut LoosSha256, digest: *mut u8) {
    // SAFETY: `c` initialized; `digest` has 32 bytes. The Sha256 is consumed by a bitwise copy; the context must be
    // re-initialized before reuse (same contract as the C implementation).
    unsafe {
        let h = std::ptr::read(c as *const Sha256);
        bytes_mut(digest, 32).copy_from_slice(&h.finish());
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_sha256(data: *const u8, len: usize, digest: *mut u8) {
    // SAFETY: see module contract.
    unsafe { bytes_mut(digest, 32).copy_from_slice(&hash::sha256(bytes(data, len))) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_constant_time_equal(a: *const u8, b: *const u8, len: usize) -> bool {
    // SAFETY: see module contract.
    unsafe { hash::ct_eq(bytes(a, len), bytes(b, len)) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_hmac_sha256_init(h: *mut LoosHmac, key: *const u8, key_len: usize) {
    // SAFETY: `h` points to writable LoosHmac storage.
    unsafe {
        let key = bytes(key, key_len);
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&hash::sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        inner.update(&k.map(|b| b ^ 0x36));
        let mut outer = Sha256::new();
        outer.update(&k.map(|b| b ^ 0x5c));
        std::ptr::write(&mut (*h).inner as *mut LoosSha256 as *mut Sha256, inner);
        std::ptr::write(&mut (*h).outer as *mut LoosSha256 as *mut Sha256, outer);
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_hmac_sha256_update(h: *mut LoosHmac, data: *const u8, len: usize) {
    // SAFETY: `h` initialized by loos_hmac_sha256_init.
    unsafe { ctx(&mut (*h).inner).update(bytes(data, len)) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_hmac_sha256_final(h: *mut LoosHmac, mac: *mut u8) {
    // SAFETY: `h` initialized; `mac` has 32 bytes.
    unsafe {
        let inner = std::ptr::read(&(*h).inner as *const LoosSha256 as *const Sha256).finish();
        let outer = ctx(&mut (*h).outer);
        outer.update(&inner);
        let o = std::ptr::read(outer as *const Sha256);
        bytes_mut(mac, 32).copy_from_slice(&o.finish());
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_hmac_sha256(key: *const u8, key_len: usize, data: *const u8, len: usize, mac: *mut u8) {
    // SAFETY: see module contract.
    unsafe { bytes_mut(mac, 32).copy_from_slice(&hash::hmac(bytes(key, key_len), bytes(data, len))) }
}

#[no_mangle]
pub unsafe extern "C" fn loos_pbkdf2_sha256(pw: *const u8, pw_len: usize, salt: *const u8, salt_len: usize, iterations: u32, out: *mut u8, out_len: usize) -> bool {
    if iterations == 0 || out.is_null() || out_len == 0 || (out_len as u64) > 32 * (u32::MAX as u64) {
        return false;
    }
    // SAFETY: see module contract.
    unsafe { hash::pbkdf2(bytes(pw, pw_len), bytes(salt, salt_len), iterations, bytes_mut(out, out_len)) };
    true
}

/// AES-256-XTS sector encryption (disk encryption); `key` is 64 bytes (two 256-bit keys). 0 ok, -1 bad args.
#[no_mangle]
pub unsafe extern "C" fn loos_xts_crypt(key: *const u8, key_len: usize, sector: u64, buf: *mut u8, len: usize, decrypt: bool) -> i32 {
    // SAFETY: see module contract.
    unsafe {
        let Ok(x) = loos_security::aes::Xts::new(bytes(key, key_len)) else { return -1 };
        let b = bytes_mut(buf, len);
        let r = if decrypt { x.decrypt_sector(sector, b) } else { x.encrypt_sector(sector, b) };
        if r.is_ok() { 0 } else { -1 }
    }
}

/// AES-GCM seal in place; writes the 16-byte tag. 0 ok, -1 bad args.
#[no_mangle]
pub unsafe extern "C" fn loos_gcm_seal(key: *const u8, key_len: usize, nonce12: *const u8, aad: *const u8, aad_len: usize, data: *mut u8, len: usize, tag16: *mut u8) -> i32 {
    // SAFETY: see module contract.
    unsafe {
        let Ok(g) = loos_security::aes::Gcm::new(bytes(key, key_len)) else { return -1 };
        let n: [u8; 12] = bytes(nonce12, 12).try_into().unwrap_or([0; 12]);
        bytes_mut(tag16, 16).copy_from_slice(&g.seal(&n, bytes(aad, aad_len), bytes_mut(data, len)));
        0
    }
}

/// AES-GCM open in place; 0 ok, -2 authentication failure, -1 bad args.
#[no_mangle]
pub unsafe extern "C" fn loos_gcm_open(key: *const u8, key_len: usize, nonce12: *const u8, aad: *const u8, aad_len: usize, data: *mut u8, len: usize, tag16: *const u8) -> i32 {
    // SAFETY: see module contract.
    unsafe {
        let Ok(g) = loos_security::aes::Gcm::new(bytes(key, key_len)) else { return -1 };
        let n: [u8; 12] = bytes(nonce12, 12).try_into().unwrap_or([0; 12]);
        if g.open(&n, bytes(aad, aad_len), bytes_mut(data, len), bytes(tag16, 16)).is_ok() { 0 } else { -2 }
    }
}
