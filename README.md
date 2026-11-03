# LoOS — build workspace + OS project

**This Project is in EARLY Alpha, like 90% of features aren't implemented yet. Also this Project is Vibecoded cuz NOWAY i'm making an WHOLE OS from scratch without AI. AI models used (no AD): Claude Sonnet 5.5, GPT-6 Astra, GPT-6.1 Sol, Claude Opus 5.5 and DeepSeek V4.1 Flash.**

**LoOS** = **Lo**cal **O**nly OS. A **Rust-first** privacy operating system
that runs Windows apps, with **btrfs** as its native filesystem, **native
package managers (winget + apt)**, and **zero telemetry**.

This repo holds two things:

1. **A build workspace** for the NT 5.2 (Server 2003 RTM) study tree, kept
   separate from LoOS development.
2. **The LoOS OS project** — the original code you publish, its docs, and the
   prompts for the three AI agents (Astra / Claude / Sol) who build it.

> **Start with [`docs/README.md`](docs/README.md).** Code-use policy (read the
> original source freely; write your own; don't 1:1 copy unless unavoidable;
> borrow open source with license notes): [`docs/SOURCE-POLICY.md`](docs/SOURCE-POLICY.md).

## One command

```bash
./loos help        # build LoOS, inspect docs/status, optional study-tree tools
./loos status      # where things are + disk + tools
```

## Layout

Big assets stay off this repo. The source tree lives on the fast internal NVMe;
outputs (ISOs, VM disks) go to the external **DATA** drive. Paths are set in
`env.sh` / `.loosrc`; see [`docs/LAYOUT.md`](docs/LAYOUT.md).

```
LoOS/                         (this repo, on DATA)
  loos                        the one CLI: build LoOS, docs/status, study tools
  env.sh                      paths (SRC_ROOT, OUT_ROOT, DATA)
  docs/                       START HERE — everything documented
  scripts/                    LoOS and optional study-tree build steps
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

## LoOS Switch (installer + migration GUI)

`migrationapp/` is a Rust GUI app for **Windows 10/11** and **Linux** that
writes the verified LoOS installer payload to a USB/dedicated disk, stages a
non-destructive UEFI setup loader, and copies user folders into a hashed,
resumable migration bundle. It is offline by construction and has zero
telemetry. See [`migrationapp/README.md`](migrationapp/README.md).

```bash
cd migrationapp
./scripts/build-linux.sh
./scripts/package-linux.sh      # dist/LoOS-Switch-*.AppImage
./scripts/package-windows.sh    # dist/LoOS-Switch-*.exe
```

## Quick start: build LoOS on Linux

```bash
./loos build                 # native modules/tests + all LoOS desktop apps
```

Requires CMake, Ninja, Clang, Cargo/Rust 1.63, and MinGW-w64. `./loos build`
does not build the NT study tree. The LoOS kernel/loader are not implemented
yet, so this builds all current LoOS targets rather than producing a bootable
OS image.

## Optional NT 5.2 study-tree tools

The NT study tree can be prepared and built separately when needed:

```
./loos setup
./loos prep
bash scripts/09-build-all.sh
./loos iso
```

These commands operate on the separate NT study tree. The Wine build uses a
writable overlay under `OUT_ROOT` so generated files do not alter `NT_SRC`.

## The LoOS OS project

Documented in [`docs/`](docs/README.md). The short version:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — boot → kernel → Win32 → UI,
  btrfs native, privacy-first.
- [`docs/BTRFS.md`](docs/BTRFS.md) — btrfs instead of NTFS (and how to borrow the
  Linux implementation).
- [`docs/BRANDING.md`](docs/BRANDING.md) + [`docs/COMPAT-APPS.md`](docs/COMPAT-APPS.md)
  — LoOS identity and the workflow for your decompiled, rebranded apps.
- [`docs/LANGUAGE.md`](docs/LANGUAGE.md) — **Rust-first** policy (C only where justified).
- [`docs/API.md`](docs/API.md) — the **LoOS API** (the native `loos.h` SDK).
- [`docs/API-STYLE.md`](docs/API-STYLE.md) — API style + full-documentation standard.
- [`docs/TOOLCHAINS.md`](docs/TOOLCHAINS.md) — gcc/g++/clang/rust targets + LoOS libc.
- [`docs/PYTHON.md`](docs/PYTHON.md) — **native Python** on LoOS.
- [`docs/BROWSER.md`](docs/BROWSER.md) — **LoOS Browser** (local-first, live pages in QEMU) and the
  staged Firefox/Gecko port plan.
- [`docs/DRIVERS.md`](docs/DRIVERS.md) — driver model, default drivers, AMD/NVIDIA GPU.
- [`docs/PACKAGING.md`](docs/PACKAGING.md) — native package managers: winget + apt.
- [`docs/INSTALL.md`](docs/INSTALL.md) — installer, recovery, and OTA updates.
- [`docs/SOURCE-POLICY.md`](docs/SOURCE-POLICY.md) — code-use rules.
- [`docs/AGENTS.md`](docs/AGENTS.md) + [`prompts/`](prompts/README.md) — how the
  three agents (Astra / Claude / Sol) work and talk (`.agentbus/`).

### Linux source and app builds

```bash
./loos build                        # all implemented LoOS targets
./loos source-build host-clang      # native modules and host tests only
./loos source-build windows-mingw   # LoOS apps for the PE runtime only
docker build -t loos-linux .        # reproducible Linux + MinGW build image
```

There is no bootable LoOS image yet; the loader and kernel targets are future
work. `./loos iso` is for the separate NT study tree.
