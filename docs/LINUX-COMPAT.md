# LoOS — Linux application compatibility ("Lin")

Run **unmodified Debian/Ubuntu Linux apps natively** on LoOS. This is the
mirror image of Wine: instead of running a Windows app on a POSIX host by
translating Win32→POSIX, **Lin** runs a Linux *ELF* binary on the LoOS/NT kernel
by translating **Linux→LoOS**.

- **Native, not emulated.** The app's x86-64 machine code runs directly on the
  CPU in LoOS user mode. No QEMU, no binary translation, no recompilation, no
  JIT, no container/VM.
- **No app modification.** Stock `.deb` binaries from the Debian/Ubuntu archive
  run as-is (same ELF, same glibc, same files, same syscalls).
- **No sandbox.** By design the Linux process has full system access — the same
  rights a native LoOS process has. We do **not** isolate it. (The privacy
  *firewall* still governs network egress — see §7 — but that is a user policy,
  not a sandbox.)
- **Interoperable with the LoOS API.** Linux apps can call the LoOS/Win32 API
  directly (windows, files, registry, IPC) through a bridge library (§5).

Directory: `src/lin/` (the subsystem). The Linux root filesystem is an ordinary
tree on **btrfs** (`\Lin\`, i.e. the Linux `/`).

---

## 1. Why this is possible (and cheap)

LoOS is an NT-shaped kernel on x86-64. That is *exactly* the substrate Linux
expects: long mode, 4-level paging, an FS with POSIX-ish semantics (**btrfs**),
threads, and a scheduler. Linux user space only needs three things from us:

1. a **loader** that maps an ELF image and jumps to it,
2. a **syscall ABI** (`syscall`/`sysret`, x86-64) whose numbers we translate, and
3. a **root filesystem** with the layout Linux expects (`/lib`, `/usr`, `/etc`,
   `/proc`, `/dev`).

We already have (1)-class machinery for PE; Lin reuses it for ELF. We already
have btrfs natively, so the FS layer is real, not faked.

## 2. Architecture

```
   unmodified ELF app  (/usr/bin/app, dynamic or static)
            │
   ┌────────▼─────────┐   ELF loader (kernel) — PT_LOAD, PT_INTERP, auxv, TLS
   │   src/lin/ld      │   builds the initial stack/auxv exactly as Linux does
   └────────┬─────────┘
            │
   ┌────────▼─────────┐   Linux syscall dispatch (x86-64 numbers)
   │ src/lin/syscall   │   openat/read/write/mmap/brk/clone/futex/… → LoOS svcs
   └────────┬─────────┘
            │              ┌── src/lin/vfs      Linux VFS view on btrfs
            │              ├── src/lin/procfs   /proc, /sys (generated)
            │              ├── src/lin/devfs    /dev (null, zero, urandom, tty…)
            │              └── src/lin/bridge   liblooswin (LoOS/Win32 from Linux)
   ┌────────▼─────────┐
   │  LoOS executive   │  Ob / Mm / Io / Ps / Fs(btrfs) / Se / net
   └───────────────────┘
```

## 3. Components

### 3.1 ELF loader (`src/lin/ld`)
- Load `ET_EXEC` and `ET_DYN` (`PIE`) x86-64 ELFs; honour `PT_LOAD` segments,
  permissions, and `PT_GNU_STACK` (nx/non-nx).
- Honour `PT_INTERP` → load the dynamic linker (`/lib64/ld-linux-x86-64.so.2`)
  as a normal ELF too (it is just another program that makes syscalls).
- Build the process entry state Linux requires: stack with `argc/argv/envp`,
  `AT_*` auxv (`AT_PHDR`, `AT_ENTRY`, `AT_BASE`, `AT_RANDOM`, `AT_HWCAP`,
  `AT_SYSINFO_EHDR` for a vDSO), and `TLS` (`FS` base) from `PT_TLS`.
- `brk`, `mmap`-based heap, and `AT_PHDR`/`AT_BASE` must match enough that stock
  glibc's `ld.so` accepts and relocates the binary.

### 3.2 Linux syscall layer (`src/lin/syscall`)
- Entry via `syscall`/`sysret` with the **Linux x86-64 syscall numbers** (the
  apps' expectations, not ours). A generated table maps each to a LoOS service.
- Core set (must-have first):
  `read write openat close lseek fstat statx mmap mprotect munmap brk
   rt_sigaction rt_sigprocmask rt_sigreturn ioctl pread64 pwrite64
   access faccessat2 pipe2 dup2 getdents64 getcwd chdir execve exit exit_group
   arch_prctl set_tid_address set_robust_list futex clone nanosleep
   clock_gettime gettimeofday getrandom uname sysinfo prlimit64`.
- Return convention: `-errno` in `rax` on error (Linux), converted from LoOS
  NTSTATUS.
- SysV vs kernel: systemd-free assumption — we ship no init; apps are launched
  by a LoOS-resident launcher (`lin`), not by a PID-1 Linux init.

### 3.3 Linux VFS on btrfs (`src/lin/vfs`)
- btrfs already provides the on-disk format; the Linux view is a **path/ABI
  translation** on top of LoOS's VFS (device nodes, symlinks, exec bits, uid/gid
  mapping, `/etc/passwd` lookup, hardlink/symlink semantics).
- Path roots: a LoOS "Linux root" (default `\Lin`). A process's `/` is that
  tree; LoOS native dirs are reachable under a reserved mount if the user allows.
- `ld.so` needs: `/etc/ld.so.cache`, `/etc/ld.so.conf`, `/lib*`, `/usr/lib*`,
  `/usr/bin`. Provide them from the rootfs.

### 3.4 Generated filesystems
- `/proc` — `self/`, `cpuinfo`, `meminfo`, `mounts`, `self/maps`, `self/status`,
  `self/fd/*`; enough for glibc and common tools.
- `/sys` — minimal, read-mostly.
- `/dev` — `null zero full random urandom tty pts/* shm/*` (real LoOS objects).
- `/dev/shm`, `/tmp` — real dirs on btrfs.

### 3.5 Threads, signals, time
- `clone()` → LoOS threads (`CLONE_THREAD|CLONE_VM|CLONE_FS|...`); `futex` →
  a LoOS wait primitive (fast path in user space, slow path a syscall).
- Signals via Linux `rt_sigaction`/`rt_sigreturn`; altstack; `SIGSEGV`/`SIGILL`
  from LoOS exceptions.
- Clocks from the LoOS timer; `clock_gettime` monotonic + realtime.

## 4. Dynamic linking / glibc

Two supported modes:
1. **Static** (busybox-style): the simplest path — no interpreter, run and go.
2. **Dynamic**: bundle an **unmodified** upstream glibc + `ld-linux-x86-64.so.2`
   from Debian/Ubuntu in the rootfs. We make it run by implementing the syscalls
   it uses. We do **not** replace or patch glibc.
- Optional later: a vDSO at `AT_SYSINFO_EHDR` for `clock_gettime`/`gettimeofday`.

## 5. LoOS API bridge (`src/lin/bridge`, `liblooswin`)

The differentiator: Linux apps can use the **LoOS/Win32 API**.
- A small shared library `liblooswin.so` exposes selected LoOS/Win32 entry
  points with C signatures a Linux app can `dlopen`/link: window create/message,
  file mapping, registry, events, process/thread, the compat profile, and the
  permission/firewall API.
- Handles are unified: a LoOS `HANDLE` is a real object handle; the bridge keeps
  a per-process translation so a Linux `fd` and a LoOS handle can be used
  together (e.g. a Linux app opens a LoOS window and reads a Linux `fd` in the
  same event loop).
- A LoOS "console/lib" can also inject the bridge into a Linux process at launch
  (via `LD_PRELOAD` pointing at our `.so`) so an unmodified app can call LoOS
  without a rebuild — the app just `dlsym`s a LoOS symbol if present.
- Direction 2 (optional): native LoOS apps may spawn and talk to Linux apps via
  the same bridge.

## 6. Running apps

- `lin /usr/bin/ls` — run from the Linux root.
- `lin --root=\Lin2 /usr/bin/app` — choose a rootfs.
- `.deb`: `lin-pkg install foo.deb` unpacks into the rootfs and registers the
  ELF (a dpkg-free installer; we don't run apt's maintainer scripts as root
  blindly — document what we do and what we skip).
- **No sandbox**: the process runs with the rights of the invoking LoOS user.
  It may read/write anywhere that user can, load any driver-available device,
  and call the LoOS API. This is a deliberate choice (see §7).

## 7. Privacy & the no-sandbox stance (explicit)

- **No sandbox, by design.** Lin does not confine the app. It is a compatibility
  layer, not a container. This matches how Wine behaves on a normal host.
- **Network egress** still passes through the LoOS stack, so the user's
  **firewall** and the **network LED** still see and can block it — that is a
  policy the user sets, not an app sandbox.
- Both the Linux ABI and the LoOS API can create network sockets; the same
  egress rules apply to both.
- Like Windows apps, Lin apps can be given a **compat profile** (e.g. a syscall
  logging level and a "no internet" flag) as convenience, not confinement.

## 8. Milestones

| L | Deliverable | Owner |
|---|---|---|
| L0 | native ELF loader; static ELF prints + exits (serial) | astra |
| L1 | Linux x86-64 syscall table + core syscalls | astra |
| L2 | run unmodified **static** Linux binaries (busybox `echo`/`ls`) | astra |
| L3 | run unmodified **dynamic** Debian/Ubuntu binary (bundled ld.so+glibc) | astra |
| L4 | Linux VFS + `/proc` `/sys` `/dev` `/etc`; `.deb` base install | sol / claude |
| L5 | threads, `futex`, `clone`, signals (`bash`, a threaded app) | astra |
| L6 | `liblooswin` bridge — Linux apps call the LoOS/Win32 API | sol |
| L7 | GUI: X11/Wayland served on the LoOS compositor; first big target is stock Firefox ESR ([BROWSER.md](BROWSER.md)) | claude |

## 9. Non-goals (v1)

- No emulation of other architectures (x86-64 host only).
- No isolation/sandbox, no per-app Linux namespaces.
- No Linux *kernel modules*; Lin uses the LoOS driver model.
- No requirement that the app be rebuilt, patched, or statically linked.

## Related
- `docs/BTRFS.md` (the FS the Linux view sits on), `prompts/UI-SPEC.md` (the
  compositor X11/Wayland would target), `prompts/BACKEND-SPEC.md` (executive
  services Lin maps onto), `docs/COMPAT-APPS.md` (the corpus model).
