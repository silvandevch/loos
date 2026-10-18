# LoOS documentation

Everything an agent (or you) needs to understand this project **without reading
the code**. Start here, then follow the links.

**LoOS** — "Lo" = **Local Only**. A privacy-first operating system. No telemetry,
no accounts, no phone-home. Runs Windows applications. Native filesystem is
**btrfs**, not NTFS.

## Read in this order

| # | Doc | Why |
|---|---|---|
| 1 | [OVERVIEW.md](OVERVIEW.md) | What LoOS is, what we're building, principles. |
| 2 | [ARCHITECTURE.md](ARCHITECTURE.md) | The layers: boot → kernel → Win32 → UI. |
| 3 | [LAYOUT.md](LAYOUT.md) | Where every file lives on disk (and how to stay low-space). |
| 4 | [BUILD.md](BUILD.md) | How to build, step by step (Wine **and** Windows VM). |
| 5 | [TOOLS.md](TOOLS.md) | Every command/tool, what it does, examples. |
| 6 | [AGENTS.md](AGENTS.md) | How Astra + Claude + Sol work here (simple rules). |
| 7 | [SOURCE-POLICY.md](SOURCE-POLICY.md) | What code we may use and how. **Not** clean-room. |
| 8 | [BRANDING.md](BRANDING.md) | LoOS branding; how to rebrand your decompiled apps. |
| 9 | [BTRFS.md](BTRFS.md) | btrfs as the native FS (NTFS is not used). |
| 10 | [COMPAT-APPS.md](COMPAT-APPS.md) | The modern Windows apps you rebranded for LoOS. |
| 11 | [DEVELOPMENT.md](DEVELOPMENT.md) | Day-to-day workflow, tests, CI, QEMU. |
| 12 | [AGENT-WORKSPACE.md](AGENT-WORKSPACE.md) | Where the agents work, and how to set that folder. |
| 13 | [BUDGET.md](BUDGET.md) | Staying cheap: tier routing, small context, few iterations. |
| 14 | [LINUX-COMPAT.md](LINUX-COMPAT.md) | Running unmodified Debian/Ubuntu apps **natively** (Linux→LoOS, not emulated). |
| 15 | [PROVENANCE.md](PROVENANCE.md) | Log of any 1:1 copies (audit trail). |
| 16 | [PACKAGING.md](PACKAGING.md) | Native package managers: winget + apt on LoOS. |
| 16b | [DRIVERS.md](DRIVERS.md) | Driver model, default drivers, graphics + AMD/NVIDIA GPU compat. |
| 16c | [LANGUAGE.md](LANGUAGE.md) | Rust-first policy: what's Rust, when C is allowed. |
| 16d | [API.md](API.md) | The LoOS API / SDK — ultra-easy, AI-friendly, fully documented `loos.h`. |
| 16e | [API-STYLE.md](API-STYLE.md) | API authoring + doc standard (every symbol documented; enforced). |
| 16f | [TOOLCHAINS.md](TOOLCHAINS.md) | Compilers/targets: gcc, g++, clang, rust, LoOS libc. |
| 16g | [PYTHON.md](PYTHON.md) | Native Python (python3, pip, C extensions) on LoOS. |
| 17 | [INSTALL.md](INSTALL.md) | Booting and installing LoOS (installer, recovery, updates). |
| 18 | [STATUS.md](STATUS.md) | Honest codebase snapshot: what works, what doesn't yet. |

## 30-second orientation

- **This repo (`LoOS`)** is two things: a *build workspace* for the NT 5.2
  source you're studying, and the *clean repo* for the original LoOS OS.
- Big stuff (source tree, ISOs, VM disks) is **not** in this repo — see
  [LAYOUT.md](LAYOUT.md). Outputs default to the external **DATA** drive.
- The agents coordinate through a message bus (`.agentbus/`, run with
  `./loos bus start`) — [AGENTS.md](AGENTS.md).
- One command runs everything: `./loos help`.

## The rules in one line each

1. Read the original source freely; write your own code; **do not 1:1 copy**
   unless there is truly no other way (see [SOURCE-POLICY.md](SOURCE-POLICY.md)).
2. Borrow from Linux/BSD/other open source when stuck — respect those licenses.
3. btrfs native. Privacy always. No telemetry, ever.
4. Keep it low-space: outputs on DATA, no redundant copies.
5. Keep the docs current — if you change behavior, change the doc.
