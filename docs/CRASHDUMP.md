# Kernel crash dump (`crates/kdump`, crate `loos-kdump`)

Local-only crash capture for bugchecks (task #223). No network code exists in the crate.

- `bugcheck` – stop codes (NT numbering where one exists; `0x1000_0001` KERNEL_PANIC, `0x1000_0002` BTRFS_CORRUPTION are LoOS-specific) + parameter meanings.
- `format` – `DumpWriter` streams header → sections → END trailer into any `Sink` (`SliceSink` for the reserved dump area, no allocation in the crash path). `DumpReader::parse` verifies header CRC, every section CRC, trailer length and section count; unknown sections are skipped (forward compatible).
- `stack` – frame-pointer walk over a fallible memory reader; a bad frame ends the walk instead of double-faulting.
- `report` – text report (`render`) and retention (`prune`: max count/bytes, newest dump never deleted).

Sections: CONTEXT, STACK, MEMORY, MODULE, THREAD, LOG. Wiring into the bugcheck path (#110, sol) = build `Header` + `Context`, `stack::walk`, write to the dump partition via `SliceSink`, reboot; the next boot parses, renders, and shows the report locally.

Verify: `cargo +1.97.1 test -p loos-kdump` (from `crates/`).

## Boot kernel stop screen, on-disk dumps, startup menu

- Stop screen: blue screen with `:(`, progress while the dump is written, stop code name and
  number, faulting address. Restarts after 8 s (`loos.test` exits QEMU with status 39 instead).
- Kernel stack overflow: a guard page sits under the 64 KiB kernel stack; #PF runs on IST2 and
  #DF/#NMI/#MC on IST1 (TSS at GDT 0x18), so overflow ends in a normal `0x50` stop.
- Dump area: the installer reserves 1 MiB before the LoOS data partition (BIOS and UEFI) and
  marks sector 0 `LOOSDUMP-AREA`. Dump data goes first, then the header (length, code), so a
  torn write is never treated as valid. The next boot reports it once on serial and opens the
  startup menu. Live media has no area: the dump stays in memory only.
- Startup menu: F8/Esc during the first 0.6 s, `bootmenu` on the command line, or automatic
  after a crash. Items: Start normally, Safe mode, Restart. `bootmenu.auto=N` answers it (tests).
- Tests: `boot/bios/bugcheck_test.sh` (pf, ud, gp, manual, stack), `boot/bios/crashdump_test.sh`.
