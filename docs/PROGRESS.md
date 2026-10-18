# Build environment construction — progress report

## Completed

### 1. Repository acquisition
| Target | Result |
|---|---|
| `theopenxp/patchkit` | cloned, branch `amd64_patches` (commit `9788158`, "Compilers from SP1") |
| `mrcxlinux/srv03rtm-anika` | cloned, `main` (`9e4d6bae`), full tree |
| base NT 5.2 source | **is** the anika tree — 248,849 files, `PrePatched_v10` |

Initial integrity scare: 70,040 files (`admin/`, `base/`, …) went missing after an
aborted `mv` depleted the staging source. Recovered exactly via `git checkout -- .`
in the (intact) repo. Verified: 248,849 files, `git status` clean.

### 2. Workspace structuring
- Source tree and patchkit live on NVMe (`/home/silvan/LoOS-build/`) — the USB
  workspace volume cannot hold symlinks and is ~80 files/s.
- Small assets (scripts, certs, manifests, backups, docs) live in this repo.
- Verified with `01-structure.sh`: all landmarks + 3 toolchains present.

### 3. Patchkit overlay (`02-overlay-patchkit.sh`)
patchkit = 2046 files → 1575 additive, ~150 identical, ~319 conflicting.
Initially applied **additive** only (1575 copied) to preserve anika's
modern-build patches; the conflicts were left intact.

#### 3b. Conflict resolution (`11-resolve-patchkit-conflicts.sh`)
All 319 conflicts resolved **in favour of patchkit** (the openxp x64 patchset —
it adds the AMD64 additions, e.g. MSVCRT alias exports, `ProcessCookie`, amd64
`dirs`/`mk.inc`/postbuild changes). For each path:
- the anika version is backed up (240 backups: 116 text + 124 binary),
- text files are rewritten to the patchkit content **normalised to the tree's
  line-ending convention** (so `.cmd` stay CRLF and the tree is consistent),
- binaries are copied byte-for-byte.
Result: **0 content conflicts remain** (2046/2046 patchkit files now match the
tree modulo line endings). Backups:
`backups/patchkit-resolve-<stamp>/`; report:
`logs/patchkit-resolve-<stamp>.tsv`. Re-runnable with `--dry-run`.

### 4. OOBE patches (`03-patch-oobe.sh`)
Real fix set (path is `base/ntsetup/oobe`, not `enduser/oobe`):
- `idl/sources` — dropped `licdlli.c` (source not shipped)
- `msobmain/msobmain.h` — disabled `#include "licdll.h"` and non-existent `setup.h`
- `msobmain/msobmain.cpp` — added patched implementation
Backups retained. Post-patch assertions pass.

### 5. Certificates (`04-certs.sh`)
Ran `certutil/generate.sh` under openssl → 11 regenerated source files folded
back into the tree + `certs/tools/{testroot,testpca,vbl03ca}.cer`, `driver.pfx`.
Fixed two upstream defects: CRLF endings; missing space before the second `-i`
(generate.sh:103) which skipped `vercert.cpp`.

### 6. Media tags (`05-mediatags.sh`)
`WIN51`, `WIN51IA`, `WIN51IS` created in the layout root.

### 7. Missing binaries (`06-stubs.sh`)
Parsed `tools/missing.cmd` → `missing/manifest.tsv` (4334 entries). Supports
real materialisation from `RETAIL_SRC`/`RETAIL_ISO`, else emits marked stubs.

### 8. Build wrappers
`_build-pass.cmd` (prebuild + `build -bcZP -M 4`), `build-amd64.cmd`
(razzle init → `_build-pass.cmd`, native or `win64 amd64` cross),
`build-all.cmd`, `09-build-all.sh` (Wine harness with `winepath` translation +
arch detection + log tee).

Corrected after testing: an earlier draft passed `x86` to razzle, which is **not
a valid argument** (razzle usage list has no `x86`), so it fell through to
usage. The right model on this host is one native-AMD64 pass — see §9.

### 9. Verification (done)
- `10-verify-env.sh`: all tooling OK (wine-staging 11.0, 32-bit-capable prefix,
  `cl.exe` present under `tools/win64/x86/amd64`).
- Wine `cmd /c` executes; `tools\razzle.cmd /?` prints usage.
- **RAZZLE full init succeeds under Wine**: `razzle.cmd free offline
  no_certcheck no_title exec echo` printed its marker, `_BuildArch=AMD64`, no
  errors (only cosmetic `ALIAS`/`on REM` noise). Environment is created.
- `exec` hand-off confirmed: a probe script at `Z:\run\media\...\scripts\_probe.cmd`
  received `SDXROOT=Z:\home\silvan\LoOS-build\srv03rtm` and `_BuildArch=AMD64`.

## Not attempted
- **Full compile.** It is a multi-hour job (250k files) and the final image
  needs the retail missing-binary pack. The wrapper + razzle init are proven,
  but this box has not run an end-to-end build.
- **Real missing-binary population** — needs retail media or the community
  binaries pack.

## Honest corrections to the original brief
- `enduser/oobe` does not exist → patched `base/ntsetup/oobe`.
- No "configuration strings that crash the 64-bit compiler wrapper" exist in
  OOBE; the real OOBE issues are missing `licdll.h`/`setup.h` and `licdlli.c`.
- Native Linux cross-compilation is **not** supported upstream; RAZZLE is
  Windows PE tooling, run here under Wine as a best-effort path.

## OS repo status (audited)

The agents started the original LoOS OS repo (`src/`, `apps/`, `tests/`, CMake).
Audited by actually building and testing (not by reading claims):

- **Host build works**: `cmake --preset host-clang` configures and builds; it
  links PE32+ `ntdll.dll` (174 exports) + `kernel32.dll` (278 exports).
- **Tests: 15/16 pass** (`ctest --preset host-clang`, ~6.5 s).
- **Two real defects** (now on the bus):
  - `security.crypto_block_test` **fails** ("constant-time length boundary") →
    task `security: fix constant-time length boundary`.
  - `make test` (the legacy btrfs Makefile harness) failed to compile under
    `-Werror` (`-Wtype-limits` at `src/fs/btrfs/btrfs.c:1661`) — **fixed** (the
    dead overflow guard now computes `needed` as `size_t`); `make test` is green.
- **All four presets build**: `host-clang`, `kernel-clang`, `windows-msvc`,
  `windows-mingw`. `kernel-clang` uses `-nostdinc` + `cmake/freestanding/include`;
  `windows-msvc` uses `clang-cl` with the LLVM COFF tools (`llvm-lib`, `llvm-rc`,
  `lld-link`) and builds the Win32 subsystem (apps need the Windows SDK, so they
  stay on `windows-mingw`). Warning flags are centralized in
  `cmake/LoosWarnings.cmake` so the MSVC-simulated driver is not fed `-pedantic`.
- Only remaining red test is the crypto one above.
- Present: `src/{kernel,fs/btrfs,io,security,win32/{apisets,compat,crt,kernel32,ldr,ntdll}}`,
  `apps/{calc,loos-cmd,loos-notepad,loos-taskmgr}`, `tests/{boot,kernel,fs,win32,compat,privacy,review}`,
  `cmake/toolchains/`, `.github/workflows/ci.yml`, `CMakePresets.json`.

Task board: 100 tasks (see `./loos bus task ls`).

## Roster

The third agent is **GPT 6.1 Sol** (`sol`) — near-Astra at ~⅕ the cost; it
replaced GPT 6 Luna. Capability order is **astra > sol > claude**, with
category-aware routing: owners are computed by `scripts/seed_tasks.py` from a
competence matrix + load balancer (astra capped so it stays on the hard core).
Board split: **astra 32 · sol 39 · claude 29**. See `docs/AGENTS.md` §Routing.
