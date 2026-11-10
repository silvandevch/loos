//! The vDSO every Linux process gets (see `vdso/vdso.c`): clock reads without a system call.
//!
//! The kernel keeps one time page ([`VVAR_ADDR`], read-only to programs) current through
//! [`crate::sys::LinSystem::set_clock_page`]; the code page sits right after it.

use alloc::vec;
use alloc::vec::Vec;

/// Address of the time page (`struct vvar` in vdso.c).
pub const VVAR_ADDR: u64 = 0x7fff_f800_0000;
/// Address the vDSO image is mapped at (`AT_SYSINFO_EHDR`).
pub const VDSO_ADDR: u64 = VVAR_ADDR + 0x1000;

static IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vdso.so"));

fn u16_at(b: &[u8], o: usize) -> usize {
    u16::from_le_bytes([b[o], b[o + 1]]) as usize
}
fn u64_at(b: &[u8], o: usize) -> usize {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) as usize
}

/// The image laid out as it must sit in memory (each PT_LOAD at its p_vaddr), or `None` when
/// the build had no C compiler.
pub fn memory_image() -> Option<Vec<u8>> {
    let b = IMAGE;
    if b.len() < 64 || &b[..4] != b"\x7fELF" {
        return None;
    }
    let (phoff, phent, phnum) = (u64_at(b, 32), u16_at(b, 54), u16_at(b, 56));
    let mut segs = Vec::new();
    for k in 0..phnum {
        let ph = phoff + k * phent;
        if u32::from_le_bytes(b[ph..ph + 4].try_into().unwrap()) == 1 {
            segs.push((u64_at(b, ph + 8), u64_at(b, ph + 16), u64_at(b, ph + 32), u64_at(b, ph + 40)));
        }
    }
    let end = segs.iter().map(|s| s.1 + s.3).max()?;
    let mut img = vec![0u8; end.div_ceil(4096) * 4096];
    for (off, vaddr, filesz, _) in segs {
        img[vaddr..vaddr + filesz].copy_from_slice(&b[off..off + filesz]);
    }
    // Mark PT_DYNAMIC read-only, as in Linux's vDSO: glibc then uses it in place instead of
    // relocating it (the mapping is not writable). The headers sit in the first segment at offset 0.
    for k in 0..phnum {
        let ph = phoff + k * phent;
        if u32::from_le_bytes(img[ph..ph + 4].try_into().unwrap()) == 2 {
            img[ph + 4..ph + 8].copy_from_slice(&4u32.to_le_bytes()); // PF_R
        }
    }
    Some(img)
}
