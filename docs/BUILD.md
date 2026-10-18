# LoOS — building

Linux and Wine are the supported build environment. A Windows VM is not
required. CMake builds native modules and cross-compiles the LoOS PE apps; Wine
drives the legacy NT source build and ISO tools.

One command checks readiness first:

```bash
./loos verify      # tools + paths + wine prefix
./loos status      # where things live + disk
```

---

## A. Build under Wine (no VM)

```bash
./loos setup       # once: fetch source + patchkit, verify layout
./loos prep        # once: overlay patches, OOBE fix, certs, tags, missing bins
./loos build       # prebuild + build (long — hours)
./loos iso         # make the bootable ISO with the tree's oscdimg
./loos qemu        # boot it
```

Or the original scripts directly:
`remote/build-wine.sh` (build+ISO in one), `scripts/09-build-all.sh` (build only).

## C. Build the LoOS source modules (CMake)

The CMake/Ninja build compiles the implemented API-set, PE, and image-loader
libraries with their host test programs. Artifacts go under `OUT_ROOT`, outside
the source checkout:

```bash
./loos source-build                 # configure, compile, and run host tests
./loos source-build kernel-clang    # configure the empty kernel target
./loos source-build windows-mingw   # cross-compile LoCmd, LoNotepad, LoTaskMgr
```

The equivalent explicit host commands are `source ./env.sh`, then
`cmake --preset host-clang`, `cmake --build --preset host-clang`, and
`ctest --preset host-clang`. `host-clang` builds `loos_apiset`, `loos_pe`, and `loos_ldr`; CTest runs the
API-set, loader, PE, and (when Python is available) compatibility-runner suites.
When standard Wine DLL paths are present, the PE test also checks those real
binaries.

The `windows-mingw` preset cross-compiles the desktop apps to x86-64 PE files
from Linux. Install CMake, Ninja, MinGW-w64 GCC, and MinGW-w64 binutils, then
run `./loos source-build windows-mingw`. Fedora package names are `cmake
ninja-build mingw64-gcc mingw64-binutils`; Debian/Ubuntu use `cmake
ninja-build gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64`.

Three cross-target toolchains are defined:

```bash
source ./env.sh
cmake --preset kernel-clang       # Clang x86-64 freestanding target
cmake --preset windows-msvc       # clang-cl, MSVC ABI, Windows SDK required
cmake --preset windows-mingw      # MinGW-w64 app build from Linux
cmake --build --preset windows-mingw --target locmd lonotepad lotaskmgr
```

The kernel preset is configure-only until `src/kernel/` has buildable sources;
`./loos source-build kernel-clang` reports this clearly.
The optional MSVC-ABI preset needs `clang-cl`, a COFF librarian, the Windows
resource compiler, and an MSVC-compatible Windows SDK. The Linux
`windows-mingw` preset is the normal app build path. The legacy RAZZLE build
above remains the build path for the NT 5.2 study tree and runs under Wine.

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

**What happens:** razzle starts, `prebuild.cmd` generates makefiles, then
`build -bcZP -M 4` compiles. Wine reports `PROCESSOR_ARCHITECTURE=AMD64`, so
razzle builds **x64 natively** (single pass). Threads are capped at 4 because the
tree is unstable above that. A full build is hours.

**First-time Wine setup** (if `./loos verify` complains):
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
