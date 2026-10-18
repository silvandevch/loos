#!/usr/bin/env python3
"""
seed_tasks.py — populate the agent bus task board with the LoOS plan.

Derived from docs/DEVELOPMENT.md (milestones M0–M10), prompts/UI-SPEC.md
(U0–U5), the backend contract, and the app-porting work. Owners match the
tier routing: astra=core, claude=breadth/UI/review, luna=mechanical/porting.

Idempotent: a task whose title already exists is left alone; dependencies are
resolved to real task ids. Run:  python3 scripts/seed_tasks.py
"""
from __future__ import annotations
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "agents"))
os.environ.setdefault("AGENTBUS_DIR", str(ROOT / ".agentbus"))
from client import Bus  # noqa: E402

# key, title, owner, priority, deps(keys), notes
TASKS = [
    # ---- kernel / core (astra) ----
    ("M0", "M0 loader + kernel banner in QEMU", "astra", "high", [], "docs/DEVELOPMENT.md"),
    ("M1", "M1 kernel core: paging, threads, preemption, syscalls, timers", "astra", "high", ["M0"], ""),
    ("M2", "M2 kernel objects + first syscalls", "astra", "high", ["M1"], ""),
    ("M3", "M3 btrfs + registry", "astra", "high", ["M2"], ""),
    ("M4", "M4 native test binary runs", "astra", "high", ["M3"], ""),
    ("boot", "boot: UEFI loader + legacy chainload", "astra", "high", [], "src/boot/"),
    ("sched", "kernel: scheduler + SMP bring-up", "astra", "high", ["M1"], ""),
    ("scabi", "kernel: x64 syscall ABI", "astra", "high", ["M2"], ""),
    ("mmsect", "mm: demand paging for section objects", "astra", "high", ["M2"], "prompts/spec/mm-sections.md"),
    ("btrfsr", "fs: btrfs read path", "astra", "high", ["M3"], "docs/BTRFS.md"),
    ("btrfsw", "fs: btrfs write path + CoW", "astra", "high", ["btrfsr"], "docs/BTRFS.md"),
    ("vfs", "fs: VFS layer + mount", "astra", "high", ["M3"], ""),
    ("io", "io: block layer + disk drivers", "astra", "high", ["M3"], ""),
    ("reghive", "registry: hive format + load/save", "astra", "high", ["M3"], ""),
    ("crypto", "security: crypto primitives", "astra", "high", ["M2"], ""),
    ("acpi", "drivers: ACPI + timer + basic bus", "astra", "norm", ["M1"], ""),

    # ---- Win32 / subsystem (claude) ----
    ("M5", "M5 PE loading (first real exe)", "claude", "high", ["M4"], ""),
    ("M6", "M6 Win32 core (hello window)", "claude", "high", ["M5"], ""),
    ("ntdll", "win32: ntdll loader + imports", "claude", "high", ["M5"], ""),
    ("k32", "win32: kernel32 core surface", "claude", "high", ["M5"], ""),
    ("apisetsF", "apisets: api-ms-win-* forwarder framework", "claude", "high", ["M5"], ""),
    ("u32", "win32: user32 window manager basics", "claude", "norm", ["M6"], ""),
    ("gdi32", "win32: gdi32 basics", "claude", "norm", ["M6"], ""),
    ("advapi", "win32: advapi32 (registry/security)", "claude", "norm", ["M6", "reghive"], ""),
    ("ole32", "win32: ole32/COM stubs", "claude", "norm", ["M6"], ""),
    ("ws2", "win32: ws2_32 + loopback-only net stack", "claude", "norm", ["M6"], ""),
    ("M7", "M7 API sets + UCRT (run a real Win10 console app)", "claude", "high", ["M6", "apisetsF"], ""),
    ("cvshim", "compat: version-claim shim (GetVersion/RtlGetVersion)", "claude", "high", ["M7"], ""),
    ("cshim", "compat: per-era shim framework", "claude", "high", ["M7"], ""),
    ("M8", "M8 compositor + d3d11 bridge", "claude", "high", ["M6"], ""),
    ("comp", "graphics: compositor", "claude", "high", ["M6"], "prompts/UI-SPEC.md"),
    ("d3dvk", "graphics: d3d11-on-vulkan bridge", "claude", "norm", ["comp"], ""),
    ("M9", "M9 Win7/11 shims + corpus green in CI", "claude", "high", ["M7"], ""),
    ("M10", "M10 privacy hardening + reproducible ISO", "claude", "high", ["M9"], ""),
    ("reproiso", "build: reproducible ISO + signing", "claude", "norm", ["M10"], ""),

    # ---- backend services for the UI (claude) ----
    ("bNet", "backend: NetMonitor + Firewall service", "claude", "high", ["M6"], "privacy LED/firewall"),
    ("bPerm", "backend: Perms service per app+resource", "claude", "norm", ["M6"], ""),
    ("bPs", "backend: Ps.snapshot service", "claude", "norm", ["M2"], ""),
    ("bFs", "backend: Fs list/stat/copy/move service", "claude", "norm", ["btrfsr"], ""),
    ("bCompat", "backend: Compat.getAppProfile/setClaimedVersion", "claude", "norm", ["cvshim"], ""),
    ("bNotify", "backend: Notify service + permission gate", "claude", "norm", ["M6"], ""),

    # ---- UI milestones (claude) ----
    ("U0", "U0 compositor rectangle + window draws/takes input", "claude", "high", ["M6"], "prompts/UI-SPEC.md"),
    ("U1", "U1 taskbar + launcher wired to app inventory", "claude", "high", ["U0"], ""),
    ("U2", "U2 window manager: snap, virtual desktops, DPI", "claude", "norm", ["U1"], ""),
    ("U3", "U3 settings: privacy/firewall/permissions + live LED", "claude", "norm", ["U1", "bNet", "bPerm"], ""),
    ("U4", "U4 file manager + task monitor", "claude", "norm", ["U1", "bFs", "bPs"], ""),
    ("U5", "U5 compatibility console + accessibility pass", "claude", "norm", ["U1", "bCompat"], ""),
    ("usignin", "shell: session/sign-in (local accounts only)", "claude", "norm", ["U0"], ""),
    ("unotif", "shell: notifications + per-app permission model", "claude", "norm", ["U1", "bNotify"], ""),

    # ---- toolchain / tests / CI / porting (luna) ----
    ("toolchain", "build: CMake+Ninja+clang toolchain (kernel + windows-msvc user)", "luna", "high", [], "docs/BUILD.md"),
    ("tboot", "tests: boot harness (headless QEMU + serial assertions)", "luna", "high", ["M0"], ""),
    ("tkernel", "tests: kernel unit harness (ctest)", "luna", "norm", ["M1"], ""),
    ("tapiset", "tests: apiset resolution test", "luna", "norm", ["apisetsF"], ""),
    ("tcorpus", "tests: compat corpus runner (7/10/11 + loos apps)", "luna", "norm", ["M7"], ""),
    ("ci", "ci: build matrix + QEMU boot + serial artifact", "luna", "high", ["tboot"], ""),
    ("cipri", "ci: privacy lint (outbound-socket gate)", "luna", "norm", ["ci"], ""),
    ("genapisets", "apisets: generate api-ms-win-* forwarder tables", "luna", "norm", ["apisetsF"], ""),
    ("genshims", "compat: per-era shim tables", "luna", "norm", ["cshim"], ""),
    ("portnotepad", "port: Notepad to a LoOS app (apps/notepad)", "luna", "norm", ["M6"], "docs/COMPAT-APPS.md"),
    ("portcalc", "port: Calculator to a LoOS app (apps/calc)", "luna", "norm", ["M6"], ""),
    ("brand", "branding: apply LoOS brand kit to shell + apps", "luna", "norm", ["U1"], "docs/BRANDING.md"),
    ("docs", "docs: keep docs current (ongoing)", "luna", "low", [], ""),
]


def main() -> int:
    b = Bus("user")
    existing = {t["title"]: t["id"] for t in b.task_list()}
    ids: dict[str, int] = {}
    added = skipped = 0
    for key, title, owner, prio, deps, notes in TASKS:
        if title in existing:
            ids[key] = existing[title]
            skipped += 1
            continue
        dep_ids = [ids[d] for d in deps if d in ids]
        tid = b.task_add(title, owner=owner, priority=prio,
                         notes=notes, deps=dep_ids)
        ids[key] = tid
        added += 1

    total = len(b.task_list())
    print(f"added {added} tasks, skipped {skipped} existing; board now has {total}")
    owners: dict[str, int] = {}
    for t in b.task_list():
        owners[t["owner"]] = owners.get(t["owner"], 0) + 1
    for o, n in sorted(owners.items()):
        print(f"  {o or '(none)':8} {n}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
