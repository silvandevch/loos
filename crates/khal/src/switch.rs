//! Kernel thread context switching (assembly in `switch.S`).

use core::arch::global_asm;

global_asm!(include_str!("switch.S"), options(att_syntax));

extern "C" {
    fn loos_context_switch(save_sp: *mut u64, new_sp: u64);
    fn loos_thread_start();
}

/// A suspended kernel context: just a saved stack pointer.
#[derive(Debug, Default)]
pub struct Context {
    sp: u64,
}

/// Entry point of a kernel thread; must never return.
pub type Entry = extern "C" fn(usize) -> !;

impl Context {
    /// An empty context to save the current execution into.
    pub const fn empty() -> Self {
        Self { sp: 0 }
    }

    /// Prepares a context that starts `entry(arg)` on the stack ending at `stack_top`.
    ///
    /// # Safety
    /// `stack_top` must be the one-past-the-end address of writable memory with at least 256 bytes below it
    /// that stays valid for the life of the thread.
    pub unsafe fn new(stack_top: *mut u8, entry: Entry, arg: usize) -> Self {
        let top = (stack_top as usize & !15) as *mut u64;
        // Layout (low -> high): r15 r14 r13 r12 rbx rbp ret ; one pad slot keeps `loos_thread_start`
        // entered with rsp % 16 == 0 so its `call` leaves the callee with the ABI-required alignment.
        // SAFETY: caller guarantees the memory.
        unsafe {
            let sp = top.sub(8);
            let frame: [u64; 8] = [0, 0, 0, entry as usize as u64, arg as u64, 0, loos_thread_start as usize as u64, 0];
            core::ptr::copy_nonoverlapping(frame.as_ptr(), sp, 8);
            Self { sp: sp as u64 }
        }
    }

    /// Saves the current context into `from` and resumes `to`.
    ///
    /// # Safety
    /// `to` must be a context produced by [`Context::new`] or saved by an earlier `switch`, and must not be
    /// resumed twice without being saved again. Interrupts should be disabled by the caller.
    pub unsafe fn switch(from: &mut Context, to: &Context) {
        // SAFETY: caller upholds the contract.
        unsafe { loos_context_switch(&mut from.sp, to.sp) }
    }
}
