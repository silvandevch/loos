# Kernel crash dump (`crates/kdump`, crate `loos-kdump`)

Local-only crash capture for bugchecks (task #223). No network code exists in the crate.

- `bugcheck` – stop codes (NT numbering where one exists; `0x1000_0001` KERNEL_PANIC, `0x1000_0002` BTRFS_CORRUPTION are LoOS-specific) + parameter meanings.
- `format` – `DumpWriter` streams header → sections → END trailer into any `Sink` (`SliceSink` for the reserved dump area, no allocation in the crash path). `DumpReader::parse` verifies header CRC, every section CRC, trailer length and section count; unknown sections are skipped (forward compatible).
- `stack` – frame-pointer walk over a fallible memory reader; a bad frame ends the walk instead of double-faulting.
- `report` – text report (`render`) and retention (`prune`: max count/bytes, newest dump never deleted).

Sections: CONTEXT, STACK, MEMORY, MODULE, THREAD, LOG. Wiring into the bugcheck path (#110, sol) = build `Header` + `Context`, `stack::walk`, write to the dump partition via `SliceSink`, reboot; the next boot parses, renders, and shows the report locally.

Verify: `cargo +1.97.1 test -p loos-kdump` (from `crates/`).
