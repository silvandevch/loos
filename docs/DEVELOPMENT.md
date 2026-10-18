# LoOS — development workflow

Day-to-day: repo layout for the OS code, the loop, tests, CI, and the milestone
board. For how the *agents* coordinate, see [AGENTS.md](AGENTS.md).

## The OS repo layout (to be created by the agents)

```
src/
  boot/          UEFI loader, legacy chainload
  kernel/        sched, vm, irql, syscalls, smp
  exec/          Ob Mm Io Ps Se + registry
  fs/            btrfs (native), fat, ntfs-readonly (optional)
  win32/         kernel32 user32 gdi32 shell32 advapi32 ole32 ws2_32
  apisets/       api-ms-win-* forwarder tables
  graphics/      compositor, d3d11-on-vulkan bridge
  shell/         the LoOS desktop (see UI-SPEC)
apps/            your rebranded apps (see COMPAT-APPS.md)
tests/
  boot/          QEMU boot assertions
  kernel/        kernel unit + integration
  compat/        the Win7/10/11 corpus + loos apps
  apisets/       forwarder resolution
branding/        shared brand kit (see BRANDING.md)
docs/            this folder
```

Build: **CMake + Ninja + clang** on Linux. Kernel target
`x86_64-unknown-none`; user mode `x86_64-pc-windows-msvc` via clang+lld with our
own headers. No MS build tools required for the shipped build.

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
| Kernel boot | `tests/boot/` boots the ISO headless in QEMU, asserts serial output | must pass |
| Compat corpus | launch each app in QEMU, assert it reaches its main window | must pass per milestone |
| API sets | resolve every `api-ms-win-*` import in the corpus | must pass |

Run the whole thing:

```bash
./loos build && ./loos iso
./loos qemu            # interactive boot
# CI runs run-qemu.sh headless with -nographic + serial log assertions
```

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
| M2 | kernel objects + first syscalls | astra | ☐ |
| M3 | btrfs + registry | astra | ☐ |
| M4 | native test binary runs | astra | ☐ |
| M5 | PE loading (first real exe) | claude | ☐ |
| M6 | Win32 core (hello window) | claude | ☐ |
| M7 | API sets + UCRT (run a real Win10 console app) | claude / luna | ☐ |
| M8 | compositor + d3d11 bridge | claude | ☐ |
| M9 | Win7/11 shims + corpus green in CI | claude / luna | ☐ |
| M10 | privacy hardening + reproducible ISO | claude | ☐ |
| U0–U5 | shell/UI milestones | claude | see UI-SPEC |

## Conventions

- Interfaces documented; modules independent; C ABI boundary.
- Cite the spec/doc you implemented from (and `docs/PROVENANCE.md` for any copy).
- Update the doc when behavior changes.
- Keep messages on the bus short: what, where, how to verify.
