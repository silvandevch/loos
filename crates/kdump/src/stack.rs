//! Frame-pointer stack walk.

use alloc::vec::Vec;

/// Walk `rbp` chains: each frame is `[saved rbp][return address]`. `read_u64` must return
/// `None` for unmapped memory so a corrupted stack ends the walk instead of faulting again.
/// Stops on null/unaligned/non-monotonic frames, a null return address, or `max` frames.
pub fn walk<F: FnMut(u64) -> Option<u64>>(rip: u64, mut rbp: u64, stack_lo: u64, stack_hi: u64, max: usize, mut read_u64: F) -> Vec<u64> {
    let mut out = Vec::new();
    if max == 0 {
        return out;
    }
    out.push(rip);
    while out.len() < max {
        if rbp == 0 || rbp & 7 != 0 || rbp < stack_lo || rbp.saturating_add(16) > stack_hi {
            break;
        }
        let (Some(next), Some(ret)) = (read_u64(rbp), read_u64(rbp + 8)) else { break };
        if ret == 0 {
            break;
        }
        out.push(ret);
        if next <= rbp {
            break; // stack grows down, so saved rbp must be higher; guards against loops
        }
        rbp = next;
    }
    out
}
