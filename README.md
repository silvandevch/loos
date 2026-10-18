# LoOS — build workspace + OS project

**LoOS** = **Lo**cal **O**nly OS. A privacy-first operating system that runs
Windows apps, with **btrfs** as its native filesystem and **zero telemetry**.

This repo holds two things:

1. **A build workspace** for the NT 5.2 (Server 2003 RTM) source you're studying:
   prep (OOBE, certs, media tags, missing binaries), a Wine/RAZZLE build, a
   remote Windows-VM build, ISO creation, and QEMU boot. It runs from `./loos`.
2. **The LoOS OS project** — the original code you publish, its docs, and the
   prompts for the three AI agents (Astra / Claude / Luna) who build it.

> **Start with [`docs/README.md`](docs/README.md).** Code-use policy (read the
> original source freely; write your own; don't 1:1 copy unless unavoidable;
> borrow open source with license notes): [`docs/SOURCE-POLICY.md`](docs/SOURCE-POLICY.md).

## One command

```bash
./loos help        # everything: setup, prep, build, iso, qemu, vm-build, docs, status
./loos status      # where things are + disk + tools
```

## Layout

Big assets stay off this repo. The source tree lives on the fast internal NVMe;
outputs (ISOs, VM disks) go to the external **DATA** drive. Paths are set in
`env.sh` / `.loosrc`; see [`docs/LAYOUT.md`](docs/LAYOUT.md).

```
LoOS/                         (this repo, on DATA)
  loos                        the one CLI: setup/prep/build/iso/qemu/vm/docs
  env.sh                      paths (SRC_ROOT, OUT_ROOT, DATA)
  docs/                       START HERE — everything documented
  scripts/                    prep + Wine build steps
  remote/                     VM build + ISO + QEMU
  prompts/                    agent prompts (multi-part) + specs
  agents/                     the agent message bus (broker + CLI)
  branding/                   LoOS brand kit + app-rebrand manifests
  apps/                       your rebranded apps (see docs/COMPAT-APPS.md)
  certs/ missing/ logs/ backups/
  .agentbus/  .loosrc

$SRC_ROOT/  (default /home/silvan/LoOS-build, fast NVMe)
  srv03rtm/                   source tree   |  patchkit/  openxp tools

$OUT_ROOT/  (default /run/media/silvan/DATA/loos/out, external DATA)
  binaries.*/  loos.iso  loos.qcow2
```

## Quick start

```bash
scripts/00-fetch.sh          # clone/update repos
scripts/run-all.sh           # overlay + oobe + certs + tags + stubs
scripts/10-verify-env.sh     # pre-flight
scripts/fix-wine-assoc.sh    # stop Wine opening .cmd in notepad
remote/build-wine.sh         # build + ISO on this host under Wine
#   or, on a Windows VM:
cp remote/config.example.env remote/config.env   # edit host/user
python3 remote/loos-build.py all                 # sync->build->iso->fetch->qemu
```

## Build flow

RAZZLE selects the target from `%PROCESSOR_ARCHITECTURE%`. Wine reports `AMD64`,
so razzle takes its **Native Support** path and builds x64 directly — a single
pass, no x86 bootstrap:

```
tools\razzle.cmd free offline no_certcheck exec <workspace>\scripts\_build-pass.cmd
```

`_build-pass.cmd` runs `tools\prebuild.cmd` (generates makefiles + build tools),
then `build -bcZP -M 4` (threads capped at 4 — the leaked tree is unstable above
that). `09-build-all.sh` runs it under Wine, using `winepath` to translate the
NVMe paths, and tees output to `logs/`.

**x86-host / cross variant:** in a 32-bit (`WINEARCH=win32`) prefix razzle builds
x86 natively; there, amd64 is cross-built by passing `win64 amd64` (razzle gates
AMD64 behind `Win64` and then selects `tools\win64\x86\amd64`). Set `CFG_CROSS=1`
to force this form. Note `x86` is *not* a valid razzle argument, and `Win32`
exists only to build x86 *on an AMD64 host* (unsupported by MS).

### Verified under Wine

`tools\razzle.cmd free offline no_certcheck exec <cmd>` initialises the full
environment under wine-staging 11.0 (reports `_BuildArch=AMD64`, no errors; only
cosmetic `ALIAS`/`on REM` noise) and `exec` correctly hands off to a script at an
absolute `Z:\` path.

## Notes and deviations from the original brief

- **OOBE path**: there is no `enduser/oobe`. OOBE is `base/ntsetup/oobe`; that is
  what `03-patch-oobe.sh` patches. `enduser/oobe` does not exist in this tree.
- **What the OOBE fix actually is**: patchkit's `msobmain.h` disables
  `#include "licdll.h"` and a non-existent `setup.h`, and `idl/sources` drops
  the unavailable `licdlli.c`. There is no architecture-specific string/ASM
  crash in OOBE; the tree already carries correct `{amd64}` dir gating.
- **Overlay policy**: the source tree is already "PrePatched_v10". Overlaying
  all of patchkit would clobber 321 of anika's validated files, so the overlay
  is additive (adds 1575 x64 sources/tools, skips conflicts). Use
  `02-overlay-patchkit.sh --force` to take patchkit's versions (backed up).
- **Missing binaries** cannot be synthesised — activation/crypto and third-party
  code were stripped from the leak. `06-stubs.sh` parses `tools/missing.cmd` and
  either materialises from a retail ISO / binaries pack (`RETAIL_SRC` or
  `RETAIL_ISO`), or emits clearly-marked placeholders for `winlogon.exe`,
  `pidgen.dll`, `l3codeca.acm`, `l3codecx.ax`, `ovcodec2.dll`, `wstcodec.sys`.
  (`msgina.dll` is built from source, not a missing binary.)
- **Certs**: `certutil/generate.sh` works on Linux via openssl. Two upstream
  defects are fixed at run time: CRLF endings, and a missing space between two
  `-i` paths on line 103 that silently skipped `vercert.cpp`.
- **Building from Linux is unsupported upstream.** RAZZLE is Windows batch + PE
  tooling; this project runs it under Wine. It is plausible but not a tested
  configuration — a Windows host remains the reliable path.

## The LoOS OS project

Documented in [`docs/`](docs/README.md). The short version:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — boot → kernel → Win32 → UI,
  btrfs native, privacy-first.
- [`docs/BTRFS.md`](docs/BTRFS.md) — btrfs instead of NTFS (and how to borrow the
  Linux implementation).
- [`docs/BRANDING.md`](docs/BRANDING.md) + [`docs/COMPAT-APPS.md`](docs/COMPAT-APPS.md)
  — LoOS identity and the workflow for your decompiled, rebranded apps.
- [`docs/SOURCE-POLICY.md`](docs/SOURCE-POLICY.md) — code-use rules.
- [`docs/AGENTS.md`](docs/AGENTS.md) + [`prompts/`](prompts/README.md) — how the
  three agents (Astra / Claude / Luna) work and talk (`.agentbus/`).
