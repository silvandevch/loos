# Backend architecture (original code)

This is the detailed plan for **LoOS** — original source, compatible interfaces.
It is what Claude Code builds. The summary lives in
[`../docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md); if they disagree, fix both.
Code-use rules are in [`../docs/SOURCE-POLICY.md`](../docs/SOURCE-POLICY.md)
(read source freely; write your own; don't 1:1 copy unless unavoidable; borrow
open source with license notes).

Target: a privacy-first, telemetry-free desktop OS, x86-64, that runs real
Win32/Win64 applications written for Windows 7, 10, and 11, exposes the modern
APIs those apps expect, and keeps the classic NT structure underneath.

---

## 1. Goals / non-goals

**Goals**
- Boot on x86-64 (UEFI + legacy BIOS), on QEMU and real hardware.
- ABI-compatible: run unmodified PE32/PE32+ binaries (user mode) that import
  the common Win32 API set; enough kernel surface that real user-mode apps
  work.
- Modern API surface — API sets (`api-ms-win-*`), CRT, BCrypt, DXGI/D3D,
  COM/OLE, shell, threads/fibers, TLS, sockets (Winsock2), registry, services.
- **Zero telemetry, zero phone-home, offline-first.** No IDs, no analytics, no
  activation callbacks. User data stays local.
- Open, reviewable code with recorded provenance for any copy/borrow
  (`docs/PROVENANCE.md`, `THIRD-PARTY.md`).

**Non-goals (v1)**
- Kernel driver binary compatibility for arbitrary third-party `.sys` (aim for
  a documented original driver model; MS driver ABI can come later).
- Bit-for-bit Windows behavior in every corner.
- Running DRM/anticheat/AV kernel filters — impossible by design.

---

## 2. Layers

```
+---------------------------------------------------------------+
|  Shell / UI          (your custom desktop — UI-SPEC.md)        |
+---------------------------------------------------------------+
|  Win32 subsystem     user32, gdi32, shell32, comctl32, advapi32|
|                      ole32/oleaut32, ws2_32, d3d*/dxgi, dwmapi  |
+---------------------------------------------------------------+
|  API sets + shims    api-ms-win-*, compat shims for 7/10/11     |
+---------------------------------------------------------------+
|  Executive / syssvcs  object mgr, registry, VFS, Ps, Io, Mm, Se |
|                       (NT-compatible interfaces, original impl)|
+---------------------------------------------------------------+
|  Kernel               scheduler, VM, IRQLs, syscall dispatch,   |
|                       HAL abstraction, driver model             |
+---------------------------------------------------------------+
|  Boot                 UEFI loader / legacy chainload            |
+---------------------------------------------------------------+
```

### 2.1 Boot
- **UEFI loader**: PE32+ `BOOTX64.EFI`; loads kernel + initrd; ExitBootServices;
  passes a boot table (memory map, ACPI, SMBIOS) to the kernel.
- **Legacy**: a BIOS `MBR → stage2` path, plus El Torito for the ISO (reuse the
  etfsboot-style flow, but with *your* loader).
- Goal: same kernel image boots both ways; boot menu; safe-mode flag.

### 2.2 Kernel
- Syscall entry/return, `KiSystemCall64`-equivalent, nonpaged/paged pools, IRQL
  discipline, DPC/APC and timers, object-based handles.
- **Syscall ABI**: define your own stable numbers; provide a thin translation
  table from the *documented* NT syscall contract so compat shims can map. Do
  **not** replicate MS's private syscall numbers as source; model the contract.
- SMP bring-up, per-CPU structures, synchronization (mutex/fast-mutex/event/
  semaphore/ERESOURCE equivalents).

### 2.3 Executive / system services
- **Ob** object manager (namespaces, handles, security descriptors/ACL model with
  your own SID scheme where not mandated).
- **Mm** memory manager: sections, demand paging, copy-on-write, `NtAllocateVirtualMemory`.
- **Io** I/O manager: IRP-like request model, device stack, plug-and-play.
- **Ps** processes/threads: `EPROCESS`/`ETHREAD` equivalents, jobs, fibers.
- **Fs** filesystems: **btrfs is the native read/write filesystem** (see
  [`../docs/BTRFS.md`](../docs/BTRFS.md)); FAT32 for ESP/tooling; NTFS may be
  mounted read-only later for legacy disks, never as the system FS. Registry
  hive format compatible enough to read/write real hives.
- **Se** security: tokens, privileges, integrity levels (modeled, not copied).
- **Registry** with the classic hive structure (necessary for app compat).

### 2.4 Win32 subsystem
- `kernel32`/`kernelbase` core: file, process, memory, sync, locale, heaps.
- `user32`: window classes, messages, hooks, input, DPI, monitors.
- `gdi32` + a modern compositor; `dwmapi` subset.
- `shell32`/`comctl32`/`shlwapi`: shell behaviors apps rely on.
- `advapi32`: registry, services, security, crypto shims.
- `ole32`/`oleaut32`: COM, automation, typelib.
- `ws2_32`: Winsock2 over your network stack.
- `dxgi`/`d3d11`/`d3d12`/`dxvk`-style GL/Vulkan bridge for graphics apps.

### 2.5 API sets and shims (the "modern APIs on an old base" trick)
The key to running modern apps is that current Windows apps import
**API sets**, not raw DLLs:
`api-ms-win-core-*`, `api-ms-win-crt-*`, etc. Provide:
- A **forwarder/loader** that resolves `api-ms-win-*` names to your
  implementations, so apps load without a shim per app.
- **Version shims** keyed off the manifest/`GetVersion`/`RtlGetVersion` so apps
  that gate on "Windows 10"/"Windows 11" feature-set see a supported version
  (and honor a per-app override — no lying to the point of breakage).
- Down-level compat for **Win7-era apps**: keep the classic entry points those
  apps import and the legacy behaviors they expect (e.g. shadow-copy-free
  fallbacks, old shell paths).
- Set your reported OS version deliberately; apps branch on it heavily.

---

## 3. Running Windows apps from 7 / 10 / 11

| App era | What it needs | How you cover it |
|---|---|---|
| **Win7** | old imports, `GetVersion` ≤ 6.1, Aero-era shell assumptions, non-DPI-aware GDI | classic kernel32/user32/gdi32 entry points + version shim + DPI virtualization |
| **Win10** | API sets, UCRT, `bcrypt`, `d3d11`, modern shell, per-monitor DPI | API-set forwarders, UCRT, DirectX bridge, modern compositor |
| **Win11** | newer API sets, rounded/rounded-corner APIs, `dwmapi` additions, MSIX-ish packaging | superset API sets, report Win11 levels where needed, packaging loader |

**Compatibility test bench (must-have):** a corpus of real installers and apps
per era (7-Zip, Notepad++, VLC, older MS Office/Visual Studio runtimes, .NET
Framework apps, a .NET Core app, a small WinUI app) run headless in QEMU as
regression gates. Add them as CI fixtures.

---

## 4. Privacy / no-telemetry (a first-class requirement)

- No outbound connections you didn't initiate. No per-install IDs. No
  "compatibility telemetry". Document a network-egress audit.
- Local-only by default: search, updates (opt-in, self-hostable), time sync
  (opt-in), certificate revocation (offline-tolerant).
- **Transparency**: build from source, reproducible ISO, no vendor keys needed
  for your own components. A "network firewall" panel that shows every process's
  sockets.
- Remove/replace anything that phones home; anything that can't be replaced is
  disabled with a clear notice.

## 5. Language, toolchain, build

- **Kernel/HAL/loader:** C or Rust (a Rust kernel with a C ABI to user mode is a
  strong modern choice; C is faster to bootstrap). Pick per module, keep the ABI
  C.
- **User-mode subsystem:** C/C++ (or Rust) behind C ABI; expose exactly the
  Win32 signatures.
- **Build:** CMake + Ninja on Linux; clang cross-target `x86_64-unknown-none`
  for kernel and `x86_64-pc-windows-msvc` (via clang + lld + mingw headers you
  write yourself) for user mode. **Do not** depend on MS build tools for the
  published build.
- **ISO:** `remote/make-iso.sh` / `xorriso` with your loader as the El Torito
  image.
- Reproducible builds; signed with *your* key.

## 6. Milestones (each is a shippable increment)

1. **M0 Boot**: your loader + kernel print to screen, then serial, in QEMU.
2. **M1 Kernel core**: paging, threads, preemption, syscall entry, timers.
3. **M2 Kernel objects + first syscalls**: memory, process, thread, handles.
4. **M3 btrfs VFS + registry**: read/write btrfs + hive; mount root.
5. **M4 ELF first**: a static native test binary runs (your own loader for it).
6. **M5 PE loading**: load a PE32+ that calls only `Nt*`/`kernel32` basic — runs.
7. **M6 Win32 core**: kernel32/user32/gdi32 enough for a hello-window app.
8. **M7 API sets + CRT**: run a real Win10 console app unmodified.
9. **M8 Graphics/compositor + d3d11 bridge**: run a GUI app with GPU.
10. **M9 Win7/11 shims + compat corpus in CI**: the regression bench passes.
11. **M10 Privacy hardening + network firewall + reproducible ISO**.

## 7. Module → agent split (see prompts/)

- **Astra**: M1–M4 (kernel/VMM/syscalls/VFS) — the hardest, correctness-
  critical core, plus novel security and hardest RE.
- **Sol**: M5–M8 (PE/Win32 subsystem/API sets/compositor) — the bulk of
  implementation: compat shims, backend services, graphics bridge, Lin bridge,
  toolchain, tests.
- **Claude**: the UI/shell (U0–U5), design & branding, docs & writing, review,
  and mechanical tables/CI.

Division is a default; it is computed per task by `scripts/seed_tasks.py` and the
bus (`./loos bus …`) lets them hand work back and forth.
