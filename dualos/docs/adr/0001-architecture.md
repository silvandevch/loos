# ADR 0001 — Dual-mode OS: native x86-64 and translated x86-64 on 32-bit x86

Status: accepted for Milestone 1. Nothing below is implemented unless marked **[verified]**.
Only `scripts/check-env.sh` exists today.

## 1. Feasibility

| Question | Answer |
|---|---|
| Can a 32-bit-only CPU run x86-64 code? | Only by software translation. No native claim is ever made for Mode B. |
| Can one codebase boot both? | Yes, as two kernel images (`kernel32.elf` ELF32, `kernel64.elf` ELF64) plus shared `no_std` crates. A 32-bit CPU cannot execute the 64-bit image, so selection must happen before either runs. |
| Where does selection happen? | GRUB2 (Multiboot2, `i386-pc`). `grub.cfg` runs `cpuid -l` (GRUB `cpuid` module, long-mode probe) and loads the matching kernel. Each kernel re-checks CPUID itself and halts with a message on mismatch (defence in depth). |
| Is a 32-bit guest address space limit real? | Yes. A 32-bit host process cannot map a 64-bit guest space. Guest memory is accessed through a software MMU (soft TLB) keyed by 64-bit guest addresses; host pointers never appear in guest state. Swap is a backing store, not extra RAM or address space. |
| Is full x86-64 compatibility plausible? | Not claimed. Target is a documented subset, expanded only as conformance tests pass. |

Hardware floor for Mode B: **Pentium-class (i586): CPUID, TSC, CMPXCHG8B.** 486 and early CPUID-less parts are out of scope (GRUB's probe and 64-bit atomics both need CPUID/CX8). Kernel code is compiled without SSE/x87 use. The translator emits only i586 baseline instructions (no CMOV, no SSE) unless the CPU feature profile reports more; the feature profile is part of the cache key.

**[verified on this host, 2026-10-26]** by `scripts/check-env.sh`: host CPU has `lm`; QEMU 10.2.2 provides `qemu-system-i386` with `pentium` and `coreduo` models; GRUB `i386-pc` has `cpuid`, `multiboot2`, `serial`, `iso9660`; `grub2-mkrescue`, `xorriso`, nightly rustc 1.101 (pinned 2026-10-08, with `rust-src`) and `rust-lld` present.
**Not yet verified:** that those QEMU models lack `lm` at boot (done in M2), anything on genuine legacy hardware. QEMU TCG does not prove real-CPU behaviour; genuine hardware testing is a separate, currently unscheduled step.

## 2. Component diagram

```
 BIOS ─► GRUB2 (i386-pc, Multiboot2)
            │ cpuid -l ?
      ┌─────┴───────────────────────────────┐
  lm: │                                     │ no lm:
      ▼                                     ▼
 kernel64.elf (x86_64)                kernel32.elf (i586)
 ┌───────────────────────┐            ┌────────────────────────────┐
 │ arch/x86_64: boot,    │            │ arch/i586: boot, GDT/IDT,  │
 │  paging, syscall, ring3│            │  paging(PAE opt), int 0x80 │
 │ native process runner │            │ emu host process (ring 3)  │
 │  (guest runs natively)│            │  ├ decoder (shared)        │
 └──────────┬────────────┘            │  ├ interpreter (shared)    │
            │                         │  ├ translator x64→i586     │
            │                         │  ├ code cache (AOT/JIT)    │
            │                         │  └ soft MMU + pager        │
            │                         └──────────┬─────────────────┘
            └───────────────┬─────────────────────┘
                shared no_std crates (portable Rust)
   abi-linux (syscall subset) · elf64 loader · vfs/fs · log · cpuid/features
```

Execution of untrusted guest code in Mode B happens in a **ring-3 host process**, not the kernel: a translator bug or wild host access faults the process, not the kernel. Guest memory accesses are bounds/permission-checked by the soft MMU in generated code (software fault isolation).

## 3. Mode-specific design

**Mode A (native).** `kernel64` in Rust (+ small asm for entry/trampolines). Long-mode paging, per-process PML4, `syscall/sysret`, ring 3. ELF64 loaded directly and executed on the CPU; the emulator crates are not linked into the default path. Syscall ABI = same Linux-subset as Mode B, implemented once in `abi-linux` against a small `Process`/`Memory` trait, with a native implementation (direct user-pointer access with validation) and an emulated one (soft-MMU access).

**Mode B (translated).** `kernel32` boots, sets up paging and a ring-3 host process for each guest process. Pipeline:
1. Decoder (pure, shared) → `Insn` IR.
2. Reference **interpreter** (shared; defines semantics; used on a 64-bit Linux host for differential tests).
3. Translator emits i586 code per basic block. Guest GPRs/RIP/RFLAGS live in memory (`GuestCpu` in the host process); i586 has too few registers to pin them. 64-bit ops become lo/hi pairs (`add/adc`, `sub/sbb`, `shld/shrd`, `mul`-based 64×64, `div` helpers). RFLAGS is computed lazily (store operands + op kind, materialise on use); this is the main correctness risk.
4. Block cache keyed by guest RIP; indirect branches via lookup helper; block chaining later.
5. SSE/SSE2/x87: Tier 1 integer only; Tier 2 SSE/SSE2/x87 via helper calls with memory-resident XMM state (needed because the x86-64 ABI uses SSE2 for all floating point). AVX/AVX2/AVX-512: **not supported**; CPUID presented to the guest reports them absent. Any undecoded or unimplemented opcode raises a guest `#UD`/an explicit "unsupported instruction at RIP" exit, never a silent skip.

**Guest memory (Mode B).** 64-bit guest virtual address → 4-level sparse guest page table (software) → guest frame → host frame (pool) or swap slot. Inline fast path in generated code: index soft-TLB by `(gva>>12)&mask`, compare 64-bit tag, add offset; miss → helper (page walk, demand fault, permission check, swap-in, guest `#PF`). Cross-page and unaligned accesses go through a slow path. Pages holding translated code carry a "code" flag; stores to them invalidate affected blocks (self-modifying code). Handles are used only for host-side resources (fds, mappings), never substituted for guest pointers. Mode A uses ordinary hardware virtual memory and shares no memory code with Mode B.

**Persistent cache.** Entry key = hash(binary content hash, text-segment range, translator version, CPU feature profile, execution settings, layout/ABI version). Each entry carries a checksum and header magic; any mismatch ⇒ discard and retranslate. AOT covers statically discoverable blocks only; JIT handles the rest; dynamic code is translated on demand and not persisted unless it is file-backed.

## 4. Decisions

| Decision | Choice | Why / alternative rejected |
|---|---|---|
| Language | Rust nightly (pinned date) `no_std` for kernels and emulator; asm only for entry/trampolines; no C initially | Memory safety for the decoder/translator/loader (spec §9). C rejected: higher memory-safety risk, no offsetting benefit. |
| Targets | custom JSON targets `i586-dualos-none`, `x86_64-dualos-none` via `build-std` | Stock targets assume SSE2 (i686) or a red zone; kernels need neither. |
| Bootloader | GRUB2 Multiboot2, one ISO | Already installed here; has the `cpuid` probe. Limitation: GRUB needs BIOS (or i386-efi) and an i386+ CPU; Limine rejected because its 32-bit-only/old-hardware support is narrower and it is not installed. |
| Guest ABI | Linux x86-64, static ELF, tiny syscall subset first | Smallest stable ABI; Windows/graphics/audio explicitly out of scope. |
| Tests | Host-side differential: native execution on this x86-64 Linux host is the reference | Trusted, free, deterministic for integer semantics. |
| Layout | New workspace in `dualos/`, own `rust-toolchain.toml`; independent of parent LoOS code | LoOS is a separate project (Windows-app OS, btrfs). Reuse only once a concrete need exists. |

## 5. Risks and mitigations

| Risk | Mitigation |
|---|---|
| RFLAGS/lazy-flag bugs | Interpreter is the oracle; per-instruction vector tests (native-vs-interpreter, then interpreter-vs-translator); no flag optimisation before tests pass. |
| 5–50× slowdown vs native; inline soft-TLB cost | Documented; correctness first; block chaining, flag elision, register caching only after conformance. |
| SSE2 mandatory for real programs | Tier 2 helper-based implementation before claiming any libc-using program works. |
| 32-bit RAM ceiling (4 GiB, PAE optional) | Memory budget per process; swap to disk with locality-aware eviction; no "swap as RAM" claims. |
| Self-modifying/JIT guests | Code-page flag + invalidation, tested before AOT/JIT milestone closes. |
| Cache poisoning/staleness | Strong key + checksum + validate-before-use; cache is untrusted input. |
| Wrong-arch execution | GRUB probe + per-kernel CPUID gate + separate images; kernel32 built with no 64-bit-only instructions (asserted by objdump check in CI script). |
| Real-hardware divergence from QEMU | Report QEMU-only results as such; hardware matrix kept separate. |
| Toolchain drift | Pin nightly by date in `rust-toolchain.toml`; `toolchain.lock`. |

## 6. Smallest realistic end-to-end prototype

**P1 (host, Milestone 3 entry):** `cargo test` on this x86-64 Linux host runs a hand-assembled static ELF64 (sum loop + `write` + `exit`) through decoder → interpreter with all 64-bit arithmetic forced through `u32` lo/hi pairs; stdout and exit code must equal the native run of the same ELF.
**P2 (boot, Milestone 2):** one ISO; under `qemu-system-i386 -cpu pentium` serial prints `mode=B (no long mode)`, under `qemu-system-x86_64 -cpu qemu64` prints `mode=A`.
Combined success = P2 boots both, then P1's interpreter runs inside kernel32 as a ring-3 process (Milestone 4 adds the translator).

## 7. Commands

```bash
cd dualos
./scripts/check-env.sh                       # M1: hardware/toolchain assumptions
rustup component add rust-src                # one-time (pinned toolchain auto-installs it)
# Planned, not yet present:
make iso32 / make iso64 / make iso           # M2
make test-host                               # M3 differential tests
make qemu32   # qemu-system-i386   -cpu pentium  -cdrom out/dualos.iso -serial stdio
make qemu64   # qemu-system-x86_64 -cpu qemu64   -cdrom out/dualos.iso -serial stdio
```

## 8. Repository structure (created incrementally)

```
dualos/
  README.md  rust-toolchain.toml  Cargo.toml  Makefile
  docs/adr/  docs/COMPAT.md (instruction/syscall coverage matrix)
  scripts/            check-env.sh  mkiso.sh  ref-test.sh
  targets/            i586-dualos-none.json  x86_64-dualos-none.json
  boot/               grub.cfg
  crates/
    cpuid/  log/  elf64/  abi-linux/       shared, no_std
    x64-decode/  x64-interp/               shared, no_std, host-testable
    x64-xlate/  codecache/  softmmu/       Mode B only
    kernel32/  kernel64/                   arch kernels
  tests/guest/        freestanding guest programs + expected output
```

## 9. Milestone plan

M2 boot both kernels + serial log + arch detection · M3 decoder/interpreter + pair arithmetic + vectors · M4 ELF loader + translator, run in QEMU `pentium` · M5 cache + JIT/AOT + SMC invalidation · M6 soft MMU, demand paging, swap · M7 syscall subset, Tier 2 SSE2/x87 · M8 integration, compat matrix, benchmarks. Exit criteria per milestone: tests executed and results recorded in `docs/COMPAT.md`; no claim beyond them.
