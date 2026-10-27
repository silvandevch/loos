//! Local crash report and dump retention.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::format::DumpReader;

/// Render a plain-text report. Nothing leaves the machine; this is what the local
/// "your PC ran into a problem" screen and `C:\LoOS\Dumps\*.txt` show.
pub fn render(d: &DumpReader) -> String {
    let mut s = String::new();
    let bc = &d.header.bugcheck;
    let _ = writeln!(s, "*** STOP 0x{:08X} ({})", bc.code, bc.name());
    for (i, p) in bc.params.iter().enumerate() {
        let m = bc.param_meaning(i);
        if m.is_empty() {
            let _ = writeln!(s, "  param{}: 0x{:016X}", i + 1, p);
        } else {
            let _ = writeln!(s, "  param{}: 0x{:016X}  ({})", i + 1, p, m);
        }
    }
    let _ = writeln!(s, "cpu {} of {}  build {:016x}  t={}ns", d.header.cpu, d.header.ncpus, d.header.build_id, d.header.time_ns);
    if let Some(c) = &d.context {
        let _ = writeln!(s, "rip={:016x} rsp={:016x} rbp={:016x} rflags={:x}", c.rip, c.rsp(), c.rbp(), c.rflags);
        let _ = writeln!(s, "cr0={:x} cr2={:x} cr3={:x} cr4={:x}", c.cr[0], c.cr[1], c.cr[2], c.cr[3]);
    }
    if !d.stack.is_empty() {
        let _ = writeln!(s, "stack:");
        for (i, a) in d.stack.iter().enumerate() {
            match d.resolve(*a) {
                Some((m, off)) => {
                    let _ = writeln!(s, "  #{:02} {:016x}  {}+0x{:x}", i, a, m, off);
                }
                None => {
                    let _ = writeln!(s, "  #{:02} {:016x}", i, a);
                }
            }
        }
    }
    let _ = writeln!(s, "{} module(s), {} thread(s), {} memory range(s)", d.modules.len(), d.threads.len(), d.memory.len());
    if !d.log.is_empty() {
        let _ = writeln!(s, "--- log tail ---\n{}", d.log);
    }
    s
}

/// Retention policy for stored dumps: given `(id, size_bytes)` oldest-first, return the ids to
/// delete so that at most `max_count` dumps remain and their total is within `max_bytes`.
/// The newest dump is never deleted.
pub fn prune(dumps: &[(u64, u64)], max_count: usize, max_bytes: u64) -> Vec<u64> {
    let mut total: u64 = dumps.iter().map(|d| d.1).sum();
    let mut count = dumps.len();
    let mut out = Vec::new();
    for (i, (id, size)) in dumps.iter().enumerate() {
        if i + 1 == dumps.len() {
            break;
        }
        if count <= max_count.max(1) && total <= max_bytes {
            break;
        }
        out.push(*id);
        count -= 1;
        total -= size;
    }
    out
}
