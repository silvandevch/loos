# LoOS — development workflow

Day-to-day: repo layout for the OS code, the loop, tests, CI, and the milestone
board. For how the *agents* coordinate, see [AGENTS.md](AGENTS.md).

## The OS repo layout (to be created by the agents)

```
src/
  boot/          UEFI loader, legacy chainload, boot menu
  kernel/        sched, vm, irql, syscalls, smp, HAL, sync, SEH
  exec/          Ob Mm Io Ps Se + registry
  fs/            btrfs (native), fat, ntfs (compat), mount manager
  drivers/       storage, USB/HID, display, audio, NIC, PCI, PnP
  net/           TCP/IP, DNS/DHCP, firewall, TLS, HTTP, RDP
  security/      SAM/logon, UAC, code signing, BitLocker-like, sandbox
  win32/         kernel32 user32 gdi32 shell32 advapi32 ole32 ws2_32 ucrt
  apisets/       api-ms-win-* forwarder tables
  graphics/      compositor, d3d11-on-vulkan, DirectWrite/D2D
  pkg/           package service: winget + apt + MSIX + MSI (docs/PACKAGING.md)
  install/       installer, recovery environment, media builder (docs/INSTALL.md)
  update/        OTA update service (delta, atomic, telemetry-free)
  shell/         the LoOS desktop (see UI-SPEC)   [Rust-first]
crates/          Rust workspace — default language for new code (LANGUAGE.md)
  loos-kernel/ loos-pe/ loos-btrfs/ loos-crypto/ loos-net/ loos-pkg/ ...
apps/            your rebranded apps (see COMPAT-APPS.md)
tests/
  boot/          QEMU boot assertions
  kernel/        kernel unit + integration
  compat/        the Win7/10/11 corpus + loos apps
  apisets/       forwarder resolution
branding/        shared brand kit (see BRANDING.md)
docs/            this folder
```

Build: **CMake + Ninja + clang** on Linux, plus a **Rust (Cargo) workspace** for
every module that can be Rust (see [LANGUAGE.md](LANGUAGE.md) — Rust-first, C
only where justified). Kernel target `x86_64-unknown-none`; Win32 modules build
on the host; PE desktop apps cross-compile with MinGW-w64. The NT 5.2 study tree
builds under Wine. Rust is wired in with corrosion; deps are vendored and built
offline.

The current privileged-independent kernel libraries are in `src/kernel/`.
`kernel.core_test` and `kernel.mm_test` exercise the host-buildable kernel core
and memory helpers under CTest, without booting a VM. `security.crypto_block_test`
checks the block and SHA-256 primitives. Architecture entry code and a bootable
image are separate work; see `prompts/spec/kernel-core.md`.

## The loop

1. Read your inbox: `./loos bus inbox --for <me> --unread`.
2. Claim the work (post before editing shared files). One writer per file.
3. Implement against the specs in `docs/` and `prompts/`.
4. Test locally (see below).
5. Report on the bus with `--ref` + how to verify; `ack` consumed messages.

## Testing

| Level | How | Gate |
|---|---|---|
| Unit | `./loos source-build` / `ctest` per module | must pass to merge |
| Kernel core | `ctest --preset host-clang -R '^kernel\\.(core|mm)_test$'` | host-only; no VM |
| Block and crypto | `ctest --preset host-clang -R '^security\\.crypto_block_test$'` | host-only block/crypto contracts |
| Kernel boot | `tests/boot/` boots the ISO headless in QEMU, asserts serial output | must pass |
| Compat corpus | launch each app in QEMU, assert it reaches its main window | must pass per milestone |
| API sets | resolve every `api-ms-win-*` import in the corpus | must pass |

Build the currently implemented LoOS targets:

```bash
./loos build
```

The kernel and loader are not buildable yet, so there is no LoOS ISO to boot.
The QEMU harness runs once those targets produce an image and serial markers.

Add a corpus app = add its launch test. Add a rebranded app (COMPAT-APPS.md) =
add it here too.

## CI

- Build on Linux (clang). Matrix: debug + release.
- Boot test in QEMU (KVM) on every push; serial log artifact.
- Compat corpus (subset for speed; full corpus nightly).
- **Privacy lint**: fail if any code opens an outbound socket outside the
  approved egress API. (Yes, enforce it — privacy is a build gate.)

## Milestone board

Track in the bus (topic `build`) and here. Each milestone = an increment with a
demo.

| M | Deliverable | Owner | State |
|---|---|---|---|
| M0 | loader + kernel banner in QEMU | astra | ☐ |
| M1 | kernel core (paging, threads, preemption, syscalls, timers) | astra | ☐ |
| M2 | kernel objects + first syscalls | astra | ◐ host executive layer; entry wiring remains |
| M3 | btrfs + registry | astra | ◐ registry hive parser/writer; btrfs service wiring remains |
| M4 | native test binary runs | astra | ☐ |
| M5 | PE loading (first real exe) | astra | ☐ |
| M6 | Win32 core (hello window) | astra | ☐ |
| M7 | API sets + UCRT (run a real Win10 console app) | sol | ☐ |
| M8 | compositor + d3d11 bridge | astra / sol | ☐ |
| M9 | Win7/11 shims + corpus green in CI | sol | ☐ |
| M10 | privacy hardening + reproducible ISO | astra / sol | ☐ |
| U0–U5 | shell/UI milestones | claude | see UI-SPEC |
| L0–L5 | Linux app compatibility core (ELF, syscalls, VFS, threads) | astra | see [LINUX-COMPAT.md](LINUX-COMPAT.md) |
| L6–L7 | Linux `liblooswin` bridge, LD_PRELOAD, Linux GUI | sol / claude | see [LINUX-COMPAT.md](LINUX-COMPAT.md) |

### Full-OS backlog (M11+)

Making LoOS a complete operating system, not just a Win32 layer. Live on the bus
with categories `hal/drivers/fs/net/security/pkg/installer/update/power/...`.

| M | Deliverable | Owner (computed) | State |
|---|---|---|---|
| M11 | HAL + full VM (physical allocator, VADs, paging, sync, SEH, mitigations) | astra | ☐ |
| M12 | driver model + real drivers (storage, USB/HID, display, audio, NIC, PCI) | astra / sol | ☐ |
| M13 | security core (SAM/logon, UAC, BitLocker-like, code signing, sandbox) | sol / astra | ☐ |
| M14 | **native package managers: winget + apt** | sol | see [PACKAGING.md](PACKAGING.md) |
| M15 | installer + recovery + OTA updates + media builder | sol / claude | see [INSTALL.md](INSTALL.md) |
| M16 | Win32 breadth (UCRT, OLE/COM, WOW64, conhost, DirectWrite/D2D) + system apps | sol / claude | ☐ |
| M17 | power/time, virtualization/containers, printing, accessibility, diagnostics | mixed | ☐ |
| M18 | **Rust-first migration**: toolchain, Cargo workspace, rewrite parsers/kernel/net/services in Rust | sol / astra | see [LANGUAGE.md](LANGUAGE.md) |
| M19 | **LoOS API/SDK**: `loos.h` + stable ABI + installable SDK | sol | see [API.md](API.md) |
| M20 | **Toolchains**: LoOS libc + gcc/g++/clang/rust targeting `x86_64-loos` | sol / astra | see [TOOLCHAINS.md](TOOLCHAINS.md) |
| M21 | **Native Python**: `python3` + stdlib + pip on LoOS | sol | see [PYTHON.md](PYTHON.md) |

## Conventions

- Interfaces documented; modules independent; C ABI boundary.
- Cite the spec/doc you implemented from (and `docs/PROVENANCE.md` for any copy).
- Update the doc when behavior changes.
- Keep messages on the bus short: what, where, how to verify.
