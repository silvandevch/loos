# LoOS — building

LoOS builds on Linux with CMake and Ninja. The top-level command builds the
implemented native modules, runs their host tests, and cross-compiles the LoOS
desktop apps. It does not build the separate NT 5.2 study tree.

```bash
./loos verify  # confirm required Linux and MinGW tools are installed
./loos build   # build all implemented LoOS targets
```

Install CMake, Ninja, Clang, MinGW-w64 GCC, and MinGW-w64 binutils. Fedora
package names are `cmake ninja-build clang mingw64-gcc mingw64-binutils`;
Debian/Ubuntu use `cmake ninja-build clang gcc-mingw-w64-x86-64
binutils-mingw-w64-x86-64`.

`./loos build` builds `host-clang` (native filesystem, kernel, and Win32 modules
plus CTest), `kernel-clang` (freestanding kernel libraries), and `windows-mingw`
(LoCalc, LoCmd, LoNotepad, and LoTaskMgr PE apps). When Clang and LLD are
available, the host build also links the LoOS
`ntdll.dll`, `kernelbase.dll`, and `kernel32.dll` PE images through the
`win32_pe_images` target. A failing host test is reported and
the independent app build still runs; the command returns failure if either
build or the host tests fail. Artifacts are written under `OUT_ROOT/cmake/`.
To build one part separately, use
`./loos source-build host-clang` or `./loos source-build windows-mingw`.
`host-clang` builds the filesystem, kernel core, and Win32 host libraries;
CTest includes `kernel.core_test`, `kernel.mm_test`, block/crypto checks, the
Btrfs fixture/reader checks, API-set, loader, PE, kernel32, and ANSI adapter
suites, plus the compatibility runner when Python is available.
When standard Wine DLL paths are present, the PE test also checks those real
binaries.

Build and runtime state defaults to `/tmp/opencode/loos-out` and
`/tmp/opencode/loos-runtime`, keeping locks, Unix sockets, incremental
artifacts, and logs off exFAT source volumes. Override `OUT_ROOT` or
`LOOS_LOCAL_ROOT` when using a native Linux filesystem.

If ordinary source reads still return `Input/output error`, the exFAT volume
needs host-side repair. Stop processes using the workspace, unmount it, run a
read-only check first, then repair and remount:

```bash
sudo umount /run/media/silvan/DATA/LoOS
sudo fsck.exfat -n /dev/sda1
sudo fsck.exfat -p /dev/sda1
udisksctl mount -b /dev/sda1
```

The container build environment does not expose `/dev/sda1`, so it cannot run
that repair itself.

The `kernel-clang` preset compiles the current freestanding kernel libraries;
it does not produce a bootable image yet. It compiles with `-nostdinc` against
the freestanding headers in `cmake/freestanding/include` (plus Clang's builtin
`stdint.h`/`stddef.h`), so it never pulls in host glibc headers.
The optional `windows-msvc` preset builds the Win32 subsystem libraries with
`clang-cl` + the LLVM COFF tools (`llvm-lib`, `llvm-rc`, `lld-link`) and the
MSVC ABI. It needs an MSVC-compatible Windows SDK to build the desktop apps, so
apps stay on the Linux `windows-mingw` preset, which is the normal LoOS
desktop-app build.

## Optional: build the NT 5.2 study tree under Wine

These commands are separate from `./loos build`:

```bash
./loos setup
./loos prep
bash scripts/09-build-all.sh
./loos iso
```

`NT_SRC` remains the read-only study tree. The Wine workflow uses a writable
`fuse-overlayfs` view at `$OUT_ROOT/nt-source-overlay/srv03rtm`; generated files
go into its upper layer. If `fuse-overlayfs` is unavailable, the script makes a
writable copy. Set `NT_BUILD_SRC` to choose a different path. Wine reports
`PROCESSOR_ARCHITECTURE=AMD64`, so RAZZLE builds x64 natively in one pass.
`_build-pass.cmd` runs `prebuild.cmd` and `build -bcZP -M 4`; the concurrency is
limited because the legacy tree is unstable above four workers.

For app compatibility corpus checks, see [COMPAT-APPS.md](COMPAT-APPS.md) and
`tests/compat/README.md`. The manifest can point to private stock binaries;
those files are not stored in the repository. A real launcher is required to
claim runtime coverage.

For headless LoOS boot checks, pass the ISO and each exact serial marker from
the loader/kernel boot contract to `tests/boot/qemu-serial.sh` using repeated
`--expect` options. The harness defaults to TCG, disables networking, and saves
the serial log under `OUT_ROOT/logs/`. The M0 loader task has not supplied a
LoOS ISO or stable serial marker yet, so a real LoOS boot cannot run yet.

## CI

`.github/workflows/ci.yml` runs the CMake/CTest suite with Clang and GCC on
pushes and pull requests, checks the generated API-set host report, and rejects
direct network API/library calls from `src/` and `apps/`. Its
QEMU job is manually enabled with `workflow_dispatch` inputs for an ISO path
and serial marker; it runs on a self-hosted runner with QEMU installed and
uploads the serial log even if an assertion fails. This job remains opt-in
until the M0 boot image and serial marker exist.

**First-time Wine setup** (for the optional NT study-tree workflow):
```bash
./loos fixwine      # make .cmd/.bat run via cmd.exe, not notepad
```

---

## Optional remote Windows VM

The local Linux/Wine build does not depend on this. The optional remote workflow
is retained for operators who specifically need Microsoft's Windows toolchain.

1. Stand up the VM — follow **`remote/win10-iot-vm-setup.md`** (Windows 10 IoT
   LTSC + OpenSSH; XP/2000 fallback included).
2. Configure and run:
   ```bash
   cp remote/config.example.env remote/config.env   # edit host/user
   ./loos vm-build          # sync → build → fetch ISO → qemu
   # or step by step:
   ./loos vm sync
   ./loos vm build
   ./loos vm iso            # re-make ISO only, no rebuild
   ./loos vm fetch          # pull ISO back to OUT_ROOT
   ./loos vm qemu
   ```

`loos-build.py` (behind `loos vm`) pushes the tree over SSH, runs
`build-in-vm.cmd` inside razzle on the guest (prebuild + build + oscdimg), and
scps the ISO back. Everything is `ssh … "cmd /c …"`, so `.cmd` runs as batch.

---

## Prerequisites

| Need | For | Check |
|---|---|---|
| `wine` (staging ok) | Wine build | `./loos verify` |
| `git rsync openssl perl python3 7z` | prep + transport | `./loos verify` |
| `mkisofs`/`xorriso` | Linux ISO fallback | `command -v mkisofs` |
| `qemu-system-x86_64`, `/dev/kvm` | boot the ISO | `ls /dev/kvm` |
| Windows VM + OpenSSH | VM build | `remote/win10-iot-vm-setup.md` |

## Missing binaries

The tree cannot build a few files (activation/crypto + third-party were
stripped). `./loos prep` parses `tools/missing.cmd` and either pulls them from
retail media or writes placeholders:

```bash
RETAIL_ISO=/path/to/srv2003_rtm.iso scripts/06-stubs.sh    # preferred
```

Placeholders produce a non-bootable image — fine for compile testing.

## ISO without a full build

If you already have a postbuild layout directory on Linux:

```bash
remote/make-iso.sh <layout-dir> "$OUT_ROOT/loos.iso"
```

## Variants

`VARIANT` picks the SKU layout `tools\oscdimg.cmd` uses (`srv`, `srv_usa`,
`wks`, `per`, `blad`). Default `srv`. Pass it where the tool accepts it, e.g.
`VARIANT=wks ./loos iso`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `.cmd` opens in notepad | `./loos fixwine` |
| `razzle … Unknown argument: x86` | don't pass `x86`; razzle has no x86 arg (see BUILD notes) |
| Wine reports wrong arch | it should say `AMD64`; `./loos verify` |
| ISO missing after build | postbuild layout incomplete; check the variant / build log in `logs/` |
| Out of space | outputs are on DATA; check `./loos status` |
