# LoOS — compatibility apps (rebranded)

Modern Windows apps that were decompiled, rewritten, and rebranded for LoOS.
They are **compatibility test targets**: if LoOS can run these *and* stock
Win7/10/11 apps, the Win32 surface is doing its job.

Branding is presentation-only — behavior, formats, and interfaces are unchanged
(see [BRANDING.md](BRANDING.md)).

## Registry

This registry tracks the application ports and their LoOS runtime status.

| App | Origin (era) | Source in repo | Rebrand only? | Runs on LoOS | Notes |
|---|---|---|---|---|---|
| LoOS Calculator | LoOS native app | `apps/calc/` | no | ☐ | `LoCalc.exe` cross-builds on Linux; arithmetic is local-only |
| LoCmd | NT command processor, extended for LoOS | `apps/loos-cmd/` | no | ☐ | Linux MinGW-w64 build; network clients/probes removed |
| LoNotepad | Windows 11 | `apps/loos-notepad/` | no | ◐ | Linux MinGW-w64 build. Boots in QEMU (`boot/gui_test.sh`): window, menu bar + drop-downs, Edit control, typing, File>Open (`shots/notepad_open.png`) and Save As (`shots/notepad_save.png`) against the in-memory guest volume. Missing: real dialog UI (open/save answer from `loos.gui.open/save=`), clipboard, printing, persistence to disk |
| LoTaskMgr | Windows 7 / early 10 | `apps/loos-taskmgr/` | no | ☐ | Linux MinGW-w64 build; online lookups and unsupported shell/firewall actions removed |

Columns:
- **Origin (era):** which Windows version the app came from (drives the
  compatibility profile).
- **Source in repo:** where your rewritten/original source lives (`apps/<name>/`).
- **Rebrand only?:** yes if you changed only branding/metadata; no if you also
  changed behavior (call it out and why).
- **Runs on LoOS:** ☐ not yet / ☑ works / ✗ blocked (note the missing API).
- **Notes:** what broke, which API/shim it needed.

## How they're used

1. Each app is added to the **compatibility corpus** — see
   [DEVELOPMENT.md](DEVELOPMENT.md). CI boots LoOS in QEMU and launches the app.
2. When an app fails, the missing API/dll is recorded here and a bus task is
   posted (`topic: compat`) to `claude` (Win32/API sets) or `astra`
   (kernel/syscalls).
3. A passing app stays in the corpus forever as a regression gate.

## Per-app requirements to capture

When you add an app, note in its row or a small `apps/<name>/REQUIREMENTS.md`:

- **Imports**: the DLLs / API sets it needs (run a PE import dump).
- **Claimed version**: what `GetVersion`/`RtlGetVersion` returns it expects.
- **Files/system**: registry keys, file paths, services it assumes.
- **Graphics**: GDI / D3D / DPI needs.
- **Network**: any — and remember we strip telemetry.

## Where the sources live

```
apps/
  <name>/           your rewritten app source (original, rebranded)
  <name>/REQUIREMENTS.md   optional per-app notes
```

Do **not** commit vendor binaries or vendor assets. Keep originals in your
private workspace; the repo holds your rewritten sources and the manifest entry.

## Relationship to stock-app testing

Two corpora, both in CI:

- **stock** — unmodified real apps per era (7/10/11) proving true compatibility.
- **loos apps** — your rebranded apps proving the LoOS app set ships.

Both must pass before a milestone is done.

## Windows runtime (loos-winrt)

`boot/gui_test.sh <exe> <out.png> [keys]` boots LoOS in QEMU with one PE64 from a tar module (`loos.gui=`, `loos.net=1` for the NIC),
runs an input script (`text`, `~` Enter, `@x,y;` click; prefix `!` traces calls) and saves a screendump. `EXTRA="host=guest/path"`
adds files to the guest volume; `GUI_OPEN`/`GUI_SAVE` answer the file dialogs. Console output goes to the serial log (`win:` lines).

| Area | State | Proof |
|---|---|---|
| PE64 loader, ~300 kernel32/user32/gdi32 imports, TEB/PEB | works | `shots/hello_gui.png`, `shots/notepad*.png` |
| `LoadLibrary`/`GetProcAddress`/`DllMain`, relocation | works | `tests/win/selftest.exe` |
| Files (in-memory volume), menus, Edit control | works | `shots/notepad_open.png`, `notepad_save.png` |
| Console programs (CUI): console window, typed lines to stdin, ReadConsole/WriteConsoleW, SetConsoleTitleW | works | `shots/cmd_console.png` (`cmd.exe`: ver, echo, dir) |
| DLL dependencies at start: EXE imports bound from DLL files on the volume, DllMain in dependency order | works | Qt 6.11: 21 DLLs + qwindows load, QtCore initialises (`tests/win/qt/build.sh`) |
| Qt 6 Widgets | ✗ blocked | needs DirectWrite (Qt's default font database calls the DWrite factory unconditionally): bus #157 |
| Threads: CreateThread, events, mutexes, semaphores, critical sections, TLS, timeouts (cooperative) | works | `tests/win/threads.exe` |
| SEH: table-driven unwind, `__try/__except/__finally`, RaiseException, CPU faults -> exceptions, RtlUnwindEx, `__C_specific_handler` | works (C); C++ personality routines use the same RtlUnwindEx path, untested | `tests/win/seh.exe` (clang `-Xclang -fasync-exceptions`) |
| Winsock: TCP/UDP, blocking + non-blocking, select, DNS, getaddrinfo | works over loos-net + e1000 | `tests/win/nettest.exe` (HTTP GET to the QEMU host) |
| OpenGL 1.x software subset (immediate mode, matrices, depth, Gouraud, SwapBuffers) | works; no textures, lighting, display lists, near-plane clipping | `shots/gltest.png` |
| TLS / HTTPS (schannel, winhttp), DirectX/DXGI, shaders, WoW64 (32-bit PE), preemptive threads, registry, clipboard, real dialog UI | **not implemented** | - |

Test programs in `tests/win/*.c` are bring-up harnesses, not compatibility targets; `apps/loos-notepad/notepad.exe` is the real app.

## Windows programs on Wine (the main path for real apps and games)

`loos-winrt` re-implements Win32 by hand (~11k lines, ~27% of the imports real programs need). That cannot reach
"most apps and games", so third-party Windows programs run the way Proton runs them on Linux: **Wine on the LoOS
Linux layer** (`loos-lin`), which already runs Firefox, VS Code and Discord. Wine supplies the Win32/NT API
(user32, gdi32, ole, the registry, services, WoW64 for 32-bit programs); DXVK and vkd3d turn Direct3D 8-12 into
Vulkan, drawn by Mesa's software Vulkan (lavapipe) into Wine's Wayland driver, which shows up as a LoOS window.

| Piece | Where | Notes |
|---|---|---|
| Runtime stage | `tools/wine-runtime.sh OUTDIR` | Host Wine (WoW64 build), DXVK, vkd3d, lavapipe; debug info stripped (1.6 GB → ~0.75 GB); cached per Wine version. `WINE_I386=0` drops 32-bit DLLs |
| Prefix | built by the same script | `C:` drive + registry made with `wineboot` on the host for user `user` (no Mono/Gecko); only `c:`/`z:` links ship, nothing from the build host's home |
| Test | `boot/lin_app_test.sh win-notepad` / `win-winecfg` | Boots LoOS in QEMU, starts the program, types, compares screenshots |

Linux-layer fixes Wine needed (each has a test in `crates/loos-lin/tests/lin.rs`):

- `bind()` of a path-named Unix socket makes the name visible (`stat` → `S_IFSOCK`), relative names resolve against
  the caller's directory, `unlink()` removes the name. Wine's client waits for wineserver's `socket` file.
- `arch_prctl(ARCH_SET_GS)` reaches the CPU for every Linux process (it only did for Prism): Wine keeps the Windows
  TEB in `%gs`.
- A fault while its signal is blocked or ignored kills the process (Linux `force_sig_fault`) instead of re-running
  the faulting instruction forever.
