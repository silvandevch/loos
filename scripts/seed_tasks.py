#!/usr/bin/env python3
"""
seed_tasks.py — populate the agent bus task board with the LoOS plan.

Derived from docs/DEVELOPMENT.md (milestones M0–M10), prompts/UI-SPEC.md
(U0–U5), the backend contract, and the app-porting work.

Owners are *computed*, not hand-picked, from a per-category competence matrix
plus a load balancer (see docs/AGENTS.md §"Routing"). For each task we take the
model that is best at its category, unless that model is already loaded past its
cap — then we hand the task to the next-best model whose competence still clears
a threshold. This keeps GPT-6 Astra (the strongest, slowest, most expensive)
on the hard core instead of drowning it in volume.

Competence is grounded in public 2026 evals (numbers rounded to a 0–100 scale):
  astra  — FrontierMath T4 97.6, ExploitBench 100, SRE-Bench 88.0, ARC-AGI-3
           99.9, Terminal-Bench 57.9, DeepSWE 74.1, KernelGen leader.
  sol    — DeepSeek V4.1 Flash 1017 at `reasoning_effort=high` (as run in
           DeepSeek Harness). DeepSeek's own table: DeepSWE v1.1 77.0 (best in
           the table, above GPT-5.6 Sol, Claude Opus 5 and Astra's 74.1),
           Terminal-Bench 4.0 42.3 (above GPT-5.6 Sol's 39.9), Apex Bench 72.1,
           Automation-Bench 60.5, Agents' Last Exam 35.4, Chartography 83.0.
           V4.1-Flash release: CyberGym 88.1, SEC-Bench Pro 62.8, Terminal-Bench
           2.1 90.6 / 3.0 30.0, HLE 42.6 (text 44.0), Codeforces 3471. Weak:
           ExploitGym 15.3, ProgramBench 29.1, NL2Repo 68.2. RL: "wish-driven"
           end-to-end project builds; 1M context.
  claude — Claude Sonnet 5.5 at `medium` effort (Claude Code / app default).
           Effort table puts medium at roughly half of the max score: Terminal-Bench 4.0
           28.8 (max 70.6, high 43.0), FrontierCode 1.1 36.5 (max 46.2, high
           49.4), CursorBench 4.0 39.2 (max 55.5); AA Intelligence Index 40.7
           (high 46.7). System card: SWE-Bench Pro 81.3, SWE-Bench Multilingual
           90.3, AA-Briefcase/GDPval-AA 1811-1844 Elo (knowledge work), OSWorld
           2.1 80.1, docs/slides/design/review. Cheap + fast; needs a tight spec.

Idempotent: a task whose title already exists is left alone (but its owner is
reconciled with the computed plan). Dependencies resolve to real task ids.
Run:  python3 scripts/seed_tasks.py [--print-plan]
"""
from __future__ import annotations
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "agents"))
os.environ.setdefault("AGENTBUS_DIR", str(ROOT / ".agentbus"))
from client import Bus  # noqa: E402

MODELS = ("astra", "sol", "claude")

# Per-category competence, ordered as MODELS. 0–100. Grounded in the evals
# summarised above; used only for relative ranking + the load balancer.
COMPETENCE: dict[str, tuple[int, int, int]] = {
    # hard core — astra's home turf. sol moves up on the syscall/VFS/kernel-object
    # tier (DeepSWE 77.0, TB 42.3) but the novel/debug-heavy paths stay astra's.
    "boot":        (95, 84, 62),
    "kernel":      (96, 84, 62),
    "io":          (93, 84, 64),
    "fs":          (94, 82, 64),
    "drivers":     (92, 82, 58),
    "net":         (94, 82, 64),
    "crypto":      (98, 80, 64),
    "security":    (95, 82, 65),
    "privacy":     (90, 84, 78),
    "graphics":    (90, 84, 66),
    # Win32 / subsystem — sol's core: strongest agentic coder in the table
    "pe":          (90, 90, 78),
    "win32":       (88, 90, 80),
    "apisets":     (80, 92, 82),
    "compat":      (82, 92, 80),
    "backend":     (84, 92, 84),
    # breadth / UI / writing / review — claude's edge (at medium it needs a
    # tight, self-contained scope; see the note in PART-1 for `claude`)
    "ui":          (76, 84, 91),
    "design":      (68, 80, 93),
    "docs":        (72, 82, 93),
    "review":      (90, 88, 90),
    # tooling / volume
    "build":       (86, 92, 85),
    "tests":       (88, 91, 84),
    "ci":          (84, 90, 89),
    "porting":     (78, 92, 82),
    # mechanical codegen — agents' last exam 35.4 is sol's weakest row
    "gen":         (82, 80, 88),
    # env — computer use: OSWorld 80.1 vs Astra 72.6
    "env":         (76, 84, 90),
    # Lin (native Linux compat)
    "lin-core":    (95, 84, 66),
    "lin-fs":      (90, 84, 70),
    "lin-bridge":  (88, 86, 74),
    "lin-gui":     (82, 84, 82),
    "lin-base":    (78, 90, 82),
    "lin-tooling": (80, 91, 84),
    "lin-corpus":  (78, 88, 88),
    # full-OS breadth (M11+): drivers, subsystems, services, packaging, install
    "pkg":         (84, 93, 86),
    "installer":   (90, 90, 82),
    "recovery":    (88, 88, 82),
    "update":      (86, 93, 84),
    "power":       (92, 84, 66),
    "time":        (92, 84, 64),
    "audio":       (88, 84, 72),
    "print":       (84, 88, 80),
    "virt":        (94, 84, 64),
    "a11y":        (72, 82, 92),
    # Rust-first language policy (docs/LANGUAGE.md)
    "rust":        (90, 94, 84),
    # SDK / compilers / Python (docs/API.md, TOOLCHAINS.md, PYTHON.md)
    "sdk":         (88, 92, 86),
    "compiler":    (86, 91, 82),
    "python":      (80, 93, 82),
}

# Load caps. astra is capped so the load balancer offloads the work it is only
# a few points ahead on (sol: V4.1 Flash's best-in-table coding; claude: review/
# design/docs); sol carries the workhorse volume and claude the breadth/UI/docs.
# Hard-core work still stays with astra past its cap (see HARD in plan_owners).
# With the current backlog the split comes out ~astra 82 · sol 122 · claude 91.
CAP = {"astra": 44, "sol": 122, "claude": 160}
THRESHOLD = 72  # a fallback model must be at least this good to take a task


def _rank(cat: str) -> list[tuple[str, int]]:
    scores = COMPETENCE[cat]
    order = {m: i for i, m in enumerate(MODELS)}
    return sorted(zip(MODELS, scores), key=lambda kv: (-kv[1], order[kv[0]]))


# The hard, correctness-critical core: if nobody else clears the threshold the
# task still belongs to astra even if astra is past its cap.
HARD = {"boot", "kernel", "io", "fs", "drivers", "net", "crypto", "security",
        "power", "time", "virt", "lin-core"}


def plan_owners(tasks):
    """tasks: list of (key, title, cat, prio, deps, notes). Returns {key: owner}."""
    load = {m: 0 for m in MODELS}
    prio_rank = {"high": 0, "norm": 1, "low": 2}
    order = sorted(range(len(tasks)),
                   key=lambda i: (prio_rank.get(tasks[i][3], 1), i))
    owners = {}
    for i in order:
        key, _title, cat, _prio, _deps, _notes = tasks[i]
        ranked = _rank(cat)
        best, best_score = ranked[0]
        if load[best] < CAP[best]:
            chosen = best
        else:
            # Best model is capped: hand off to the next model that is still
            # good enough (>= threshold) and has room.
            chosen = None
            for model, score in ranked[1:]:
                if score >= THRESHOLD and load[model] < CAP[model]:
                    chosen = model
                    break
            if chosen is None:
                # Nobody else clears the threshold. Hard core work stays with
                # the best model even past its cap (never dump it on a poor
                # fit); volume work goes to whoever has the most headroom.
                if cat in HARD:
                    chosen = best
                else:
                    chosen = min(MODELS, key=lambda m: load[m] - CAP[m])
        owners[key] = chosen
        load[chosen] += 1
    return owners


# key, title, category, priority, deps(keys), notes
TASKS = [
    # ---- kernel / core ----
    ("M0", "M0 loader + kernel banner in QEMU", "boot", "high", [], "docs/DEVELOPMENT.md"),
    ("M1", "M1 kernel core: paging, threads, preemption, syscalls, timers", "kernel", "high", ["M0"], ""),
    ("M2", "M2 kernel objects + first syscalls", "kernel", "high", ["M1"], ""),
    ("M3", "M3 btrfs + registry", "fs", "high", ["M2"], ""),
    ("M4", "M4 native test binary runs", "tests", "high", ["M3"], ""),
    ("boot", "boot: UEFI loader + legacy chainload", "boot", "high", [], "src/boot/"),
    ("bootlegacy", "boot: legacy BIOS MBR->stage2 + El Torito + boot menu/safe-mode", "boot", "norm", ["boot"], ""),
    ("ob", "exec: Ob object manager (namespaces, handles, security descriptors)", "kernel", "high", ["M2"], "BACKEND-SPEC 2.3"),
    ("se", "exec: Se security - tokens, privileges, integrity levels", "security", "norm", ["M2"], ""),
    ("psx", "exec: Ps processes/threads + jobs/fibers", "kernel", "high", ["M2"], ""),
    ("iomgr", "exec: Io I/O manager (IRP model, device stack, PnP)", "kernel", "norm", ["M2"], ""),
    ("net", "net: TCP/IP stack (IPv4/IPv6) + winsock glue", "net", "norm", ["M2"], ""),
    ("sched", "kernel: scheduler + SMP bring-up", "kernel", "high", ["M1"], ""),
    ("scabi", "kernel: x64 syscall ABI", "kernel", "high", ["M2"], ""),
    ("mmsect", "mm: demand paging for section objects", "kernel", "high", ["M2"], "prompts/spec/mm-sections.md"),
    ("btrfsr", "fs: btrfs read path", "fs", "high", ["M3"], "docs/BTRFS.md"),
    ("btrfsw", "fs: btrfs write path + CoW", "fs", "high", ["btrfsr"], "docs/BTRFS.md"),
    ("vfs", "fs: VFS layer + mount", "fs", "high", ["M3"], ""),
    ("io", "io: block layer + disk drivers", "io", "high", ["M3"], ""),
    ("reghive", "registry: hive format + load/save", "fs", "high", ["M3"], ""),
    ("crypto", "security: crypto primitives", "crypto", "high", ["M2"], ""),
    ("acpi", "drivers: ACPI + timer + basic bus", "drivers", "norm", ["M1"], ""),

    # ---- Win32 / subsystem ----
    ("M5", "M5 PE loading (first real exe)", "pe", "high", ["M4"], ""),
    ("M6", "M6 Win32 core (hello window)", "win32", "high", ["M5"], ""),
    ("ntdll", "win32: ntdll loader + imports", "win32", "high", ["M5"], ""),
    ("k32", "win32: kernel32 core surface", "win32", "high", ["M5"], ""),
    ("apisetsF", "apisets: api-ms-win-* forwarder framework", "apisets", "high", ["M5"], ""),
    ("u32", "win32: user32 window manager basics", "win32", "norm", ["M6"], ""),
    ("gdi32", "win32: gdi32 basics", "win32", "norm", ["M6"], ""),
    ("advapi", "win32: advapi32 (registry/security)", "win32", "norm", ["M6", "reghive"], ""),
    ("ole32", "win32: ole32/COM stubs", "win32", "norm", ["M6"], ""),
    ("ws2", "win32: ws2_32 + loopback-only net stack", "win32", "norm", ["M6"], ""),
    ("M7", "M7 API sets + UCRT (run a real Win10 console app)", "apisets", "high", ["M6", "apisetsF"], ""),
    ("cvshim", "compat: version-claim shim (GetVersion/RtlGetVersion)", "compat", "high", ["M7"], ""),
    ("cshim", "compat: per-era shim framework", "compat", "high", ["M7"], ""),
    ("M8", "M8 compositor + d3d11 bridge", "graphics", "high", ["M6"], ""),
    ("comp", "graphics: compositor", "graphics", "high", ["M6"], "prompts/UI-SPEC.md"),
    ("d3dvk", "graphics: d3d11-on-vulkan bridge", "graphics", "norm", ["comp"], ""),
    ("dwm", "win32: dwmapi subset", "win32", "norm", ["comp"], ""),
    ("dxgi", "win32: dxgi/d3d12 surface", "graphics", "low", ["d3dvk"], ""),
    ("shell32", "win32: shell32/comctl32/shlwapi", "win32", "norm", ["M6"], ""),
    ("scm", "win32: SCM/services control", "win32", "norm", ["advapi"], ""),
    ("M9", "M9 Win7/11 shims + corpus green in CI", "compat", "high", ["M7"], ""),
    ("M10", "M10 privacy hardening + reproducible ISO", "privacy", "high", ["M9"], ""),
    ("reproiso", "build: reproducible ISO + signing", "privacy", "norm", ["M10"], ""),
    ("egressaudit", "privacy: network-egress audit doc", "docs", "norm", ["M10"], "docs/"),

    # ---- backend services for the UI ----
    ("bNet", "backend: NetMonitor + Firewall service", "backend", "high", ["M6"], "privacy LED/firewall"),
    ("bPerm", "backend: Perms service per app+resource", "backend", "norm", ["M6"], ""),
    ("bPs", "backend: Ps.snapshot service", "backend", "norm", ["M2"], ""),
    ("bFs", "backend: Fs list/stat/copy/move service", "backend", "norm", ["btrfsr"], ""),
    ("bCompat", "backend: Compat.getAppProfile/setClaimedVersion", "backend", "norm", ["cvshim"], ""),
    ("bNotify", "backend: Notify service + permission gate", "backend", "norm", ["M6"], ""),

    # ---- UI milestones ----
    ("U0", "U0 compositor rectangle + window draws/takes input", "ui", "high", ["M6"], "prompts/UI-SPEC.md"),
    ("U1", "U1 taskbar + launcher wired to app inventory", "ui", "high", ["U0"], ""),
    ("U2", "U2 window manager: snap, virtual desktops, DPI", "ui", "norm", ["U1"], ""),
    ("U3", "U3 settings: privacy/firewall/permissions + live LED", "ui", "norm", ["U1", "bNet", "bPerm"], ""),
    ("U4", "U4 file manager + task monitor", "ui", "norm", ["U1", "bFs", "bPs"], ""),
    ("U5", "U5 compatibility console + accessibility pass", "ui", "norm", ["U1", "bCompat"], ""),
    ("usignin", "shell: session/sign-in (local accounts only)", "ui", "norm", ["U0"], ""),
    ("unotif", "shell: notifications + per-app permission model", "ui", "norm", ["U1", "bNotify"], ""),

    # ---- toolchain / tests / CI / porting ----
    ("toolchain", "build: CMake+Ninja+clang toolchain (kernel + windows-msvc user)", "build", "high", [], "docs/BUILD.md"),

    # ---- Rust-first language policy (docs/LANGUAGE.md) ----
    ("rust-toolchain", "build: Rust toolchain + rustup targets + CMake/corrosion integration", "rust", "high", ["toolchain"], "docs/LANGUAGE.md"),
    ("rust-workspace", "build: Rust Cargo workspace (crates/) + no_std kernel crate", "rust", "high", ["rust-toolchain"], ""),
    ("rust-ffi", "build: C<->Rust FFI layer (cbindgen/bindgen) + C-ABI exports", "rust", "norm", ["rust-workspace"], ""),
    ("rust-vendor", "build: vendored crates + cargo deny/vet + offline reproducible builds", "rust", "norm", ["rust-workspace"], "supply chain"),
    ("rust-kernel", "kernel: reimplement kernel core in no_std Rust", "rust", "norm", ["M1", "rust-workspace"], ""),
    ("rust-pe", "pe: reimplement PE/COFF parser+loader in Rust (untrusted input)", "rust", "norm", ["M5", "rust-workspace"], ""),
    ("rust-btrfs", "fs: reimplement btrfs in Rust (untrusted input)", "rust", "norm", ["btrfsw", "rust-workspace"], ""),
    ("rust-crypto", "security: crypto primitives on RustCrypto", "rust", "norm", ["crypto", "rust-workspace"], ""),
    ("rust-net", "net: TCP/IP stack in Rust", "rust", "norm", ["net", "rust-workspace"], ""),
    ("rust-services", "backend: backend services in Rust", "rust", "norm", ["M6", "rust-workspace"], ""),
    ("rust-pkg", "pkg: package service in Rust (serde manifests)", "rust", "norm", ["pkgd", "rust-workspace"], ""),
    ("rust-comp", "graphics: compositor core in Rust", "rust", "norm", ["comp", "rust-workspace"], ""),
    ("rust-win32", "win32: subsystem surface as Rust extern C crates", "rust", "low", ["M5", "rust-workspace"], ""),
    ("rust-apps", "apps: new LoOS apps in Rust (windows crate)", "rust", "low", ["M6", "rust-workspace"], ""),
    ("keep-c", "docs: record modules that stay C and why (FFI/asm/unsafe-heavy)", "docs", "norm", [], "docs/LANGUAGE.md"),
    # ---- hand-added work (not part of the seed backlog) ----
    ("rust-comp-slice", "graphics: port the compositor + display-path slice (comp backend modules) from C to Rust", "rust", "high", ["rust-comp", "comp"], "one batched request; docs/LANGUAGE.md"),

    # ---- LoOS API / SDK (docs/API.md) ----
    ("sdk-api", "sdk: LoOS public API (loos.h + per-subsystem headers, opaque-handle ABI)", "sdk", "high", [], "docs/API.md"),
    ("sdk-abi", "sdk: ABI versioning + symbol versioning + deprecation policy", "sdk", "norm", ["sdk-api"], ""),
    ("sdk-pkg", "sdk: installable SDK (headers, import libs, pkg-config/CMake package)", "sdk", "norm", ["sdk-api"], ""),
    ("sdk-bindings", "sdk: language bindings (Rust crate, Python ctypes) for the LoOS API", "sdk", "norm", ["sdk-api"], ""),
    ("sdk-style", "sdk: API style guide (naming, one-call-one-job, ownership, async shape)", "sdk", "high", ["sdk-api"], "docs/API-STYLE.md"),
    ("sdk-docstd", "sdk: doc-comment standard (summary/since/threadsafe/params/errors/ownership/example/see)", "sdk", "high", ["sdk-style"], "docs/API-STYLE.md"),
    ("sdk-apidoc", "sdk: reference generator (headers -> docs/api/ Markdown)", "sdk", "high", ["sdk-docstd"], ""),
    ("sdk-ailint", "sdk: CI gate apilint - fail the build if any public symbol is under-documented", "sdk", "high", ["sdk-docstd"], ""),
    ("sdk-manifest", "sdk: machine-readable API manifest (api/loos.api.json) for AI + tooling", "sdk", "norm", ["sdk-apidoc"], ""),
    ("sdk-examples", "sdk: minimal runnable example per subsystem, doctested in CI", "sdk", "norm", ["sdk-api"], ""),
    ("sdk-errors", "sdk: complete documented error enum + loos_status_str + per-call error lists", "sdk", "norm", ["sdk-api"], ""),
    ("sdk-search", "sdk: API index (A-Z and by-category) for discoverability", "sdk", "low", ["sdk-apidoc"], ""),
    ("sdk-aiguide", "sdk: AI guide - how agents read/use/extend the API via the manifest", "docs", "norm", ["sdk-style"], "docs/API-STYLE.md"),

    # ---- compilers & language toolchains (docs/TOOLCHAINS.md) ----
    ("libc-loos", "toolchain: LoOS C library (POSIX subset) over the LoOS API", "compiler", "high", ["sdk-api"], "docs/TOOLCHAINS.md"),
    ("gcc-port", "toolchain: binutils + GCC (x86_64-loos)", "compiler", "high", ["libc-loos"], ""),
    ("gpp-port", "toolchain: G++ / libstdc++ (C++ on LoOS)", "compiler", "norm", ["gcc-port"], ""),
    ("clang-port", "toolchain: clang/LLVM native + lld targeting x86_64-loos", "compiler", "norm", ["libc-loos"], ""),
    ("rust-target", "toolchain: rustc x86_64-loos target (no_std + std)", "compiler", "norm", ["rust-toolchain", "libc-loos"], ""),
    ("langs-optional", "toolchain: optional frontends Zig/Go/Fortran [deferred]", "compiler", "low", ["libc-loos"], ""),
    ("native-toolchain", "toolchain: self-hosted gcc on LoOS compiles a LoOS program", "compiler", "low", ["gcc-port"], ""),
    ("csharp-deferred", "toolchain: C#/.NET - intentionally deferred (example only)", "compiler", "low", [], "not planned now"),

    # ---- native Python (docs/PYTHON.md) ----
    ("python", "python: native CPython (python3) on LoOS", "python", "high", ["libc-loos"], "docs/PYTHON.md"),
    ("python-stdlib", "python: CPython stdlib + C extension modules on LoOS", "python", "norm", ["python"], ""),
    ("python-pip", "python: pip + venv + wheels via the package service", "python", "norm", ["python", "pkgd"], ""),
    ("python-capi", "python: ctypes/cffi bindings to the LoOS API", "python", "norm", ["python", "sdk-api"], ""),
    ("python-embed", "python: embeddable libpython for LoOS scripting", "python", "low", ["python"], ""),
    ("tboot", "tests: boot harness (headless QEMU + serial assertions)", "tests", "high", ["M0"], ""),
    ("tkernel", "tests: kernel unit harness (ctest)", "tests", "norm", ["M1"], ""),
    ("tapiset", "tests: apiset resolution test", "tests", "norm", ["apisetsF"], ""),
    ("tcorpus", "tests: compat corpus runner (7/10/11 + loos apps)", "tests", "norm", ["M7"], ""),
    ("peexec", "tests: run a real PE32+ exe through the loader (pe_exec)", "tests", "norm", ["M5"], "tests/win32/exec/"),
    ("corpus", "compat: assemble stock corpus (7-Zip, Notepad++, VLC, .NET app)", "porting", "norm", ["M7"], "docs/COMPAT-APPS.md"),
    ("ci", "ci: build matrix + QEMU boot + serial artifact", "ci", "high", ["tboot"], ""),
    ("cipri", "ci: privacy lint (outbound-socket gate)", "ci", "norm", ["ci"], ""),
    ("genapisets", "apisets: generate api-ms-win-* forwarder tables", "apisets", "norm", ["apisetsF"], ""),
    ("genshims", "compat: per-era shim tables", "compat", "norm", ["cshim"], ""),
    ("portnotepad", "port: Notepad to a LoOS app (apps/notepad)", "porting", "norm", ["M6"], "docs/COMPAT-APPS.md"),
    ("portcalc", "port: Calculator to a LoOS app (apps/calc)", "porting", "norm", ["M6"], ""),
    ("brand", "branding: apply LoOS brand kit to shell + apps", "design", "norm", ["U1"], "docs/BRANDING.md"),
    ("buildvm", "env: stand up Win10 IoT build VM + SSH pipeline", "env", "norm", [], "remote/"),
    ("docs", "docs: keep docs current (ongoing)", "docs", "low", [], ""),

    # ---- defects found in the audit (verified by build/test) ----
    ("bug-crypto", "security: fix constant-time length boundary (crypto_block_test fails)", "crypto", "high", ["crypto"], "ctest: 1/16 fail"),
    ("bug-btrfs-mk", "build: fix Makefile btrfs harness (-Wtype-limits under -Werror)", "build", "norm", [], "make test fails; CMake btrfs passes"),

    # ---- Lin: native Linux app compatibility (docs/LINUX-COMPAT.md) ----
    ("L0", "lin L0: native ELF loader in the LoOS kernel", "lin-core", "high", [], "docs/LINUX-COMPAT.md"),
    ("L1", "lin L1: x86-64 Linux syscall table + core syscalls", "lin-core", "high", ["L0"], ""),
    ("L2", "lin L2: run unmodified static Linux binaries (busybox echo/ls)", "lin-core", "high", ["L1"], ""),
    ("L3", "lin L3: run unmodified dynamic Debian/Ubuntu binary (bundled ld.so+glibc)", "lin-core", "high", ["L2"], ""),
    ("linfs", "lin: Linux VFS + /proc /sys /dev /etc", "lin-fs", "norm", ["L2"], ""),
    ("L4", "lin L4: .deb base install + rootfs pack tool", "lin-base", "norm", ["linfs"], ""),
    ("L5", "lin L5: threads, futex, clone, signals (bash, threaded app)", "lin-core", "high", ["L3"], ""),
    ("linnet", "lin: Linux sockets -> LoOS net stack", "lin-core", "norm", ["L3", "net"], ""),
    ("L6", "lin L6: liblooswin bridge (call LoOS/Win32 API from Linux apps)", "lin-bridge", "high", ["L3"], ""),
    ("L7", "lin L7: GUI - X11/Wayland on the LoOS compositor", "lin-gui", "low", ["L6", "comp"], ""),
    ("linsysgen", "lin: syscall table generator + coverage test", "lin-tooling", "norm", ["L1"], ""),
    ("lincorpus", "lin: compat corpus (Ubuntu/Debian binaries) + CI", "lin-corpus", "norm", ["L2"], ""),
    ("linpreload", "lin: LD_PRELOAD injection so unmodified apps reach liblooswin", "lin-bridge", "norm", ["L6"], ""),

    # =====================================================================
    # M11+ — make LoOS a FULL operating system (parity with Windows).
    # Sections: kernel/HAL, drivers, filesystems, networking, security,
    # Win32 breadth, package managers, installer, updates, power, virt,
    # printing, system apps, accessibility, diagnostics.
    # =====================================================================

    # ---- kernel / HAL / memory completeness ----
    ("hal", "kernel: x86-64 HAL (IDT, GDT, MSRs, IPI, local APIC timers)", "kernel", "high", ["M1"], "src/kernel/"),
    ("mmphys", "mm: physical page allocator + kernel heap (slab/KMALLOC)", "kernel", "high", ["M1"], ""),
    ("mmvad", "mm: virtual address descriptors + working sets", "kernel", "high", ["mmphys"], ""),
    ("mmpagefile", "mm: paging file + page eviction + standby list", "kernel", "norm", ["mmvad"], ""),
    ("sync", "kernel: synchronization primitives (mutex/event/semaphore/wait)", "kernel", "high", ["M2"], ""),
    ("apcdpc", "kernel: APC/DPC + work queues + waitable timers", "kernel", "norm", ["sync"], ""),
    ("seh", "kernel: structured exception handling (SEH/VEH)", "kernel", "norm", ["M2"], ""),
    ("kd", "kernel: kernel debugger (KD) + in-memory crash capture", "kernel", "low", ["M1"], ""),
    ("mitigations", "security: kernel mitigations (DEP/ASLR/CFG/KASLR/W^X)", "security", "high", ["mmvad"], ""),
    ("bugcheck", "kernel: bugcheck + safe-mode boot", "kernel", "norm", ["M0"], ""),
    ("smss", "exec: session manager + subsystem init (smss)", "kernel", "high", ["M2"], ""),
    ("csrss", "exec: console subsystem / server (csrss)", "kernel", "norm", ["smss"], ""),
    ("pnp", "exec: PnP manager + device enumeration", "drivers", "norm", ["iomgr"], ""),
    ("kmdf", "drivers: kernel-mode driver framework + load/unload", "drivers", "norm", ["iomgr"], ""),

    # ---- drivers ----
    ("pci", "drivers: PCI bus + MSI/MSI-X", "drivers", "norm", ["acpi"], ""),
    ("stornvme", "drivers: NVMe + AHCI storage stack", "drivers", "high", ["io"], ""),
    ("partmgr", "drivers: partition manager (MBR/GPT)", "drivers", "norm", ["io"], ""),
    ("usb", "drivers: USB stack (xHCI hub/HID/mass-storage)", "drivers", "high", ["io"], ""),
    ("hiddrv", "drivers: input (PS/2 + HID keyboard/mouse)", "drivers", "norm", ["usb"], ""),
    ("dispdrv", "drivers: display (framebuffer + GPU modesetting)", "drivers", "high", ["comp"], ""),
    ("audiodrv", "drivers: audio (HDA) + WASAPI engine", "audio", "norm", ["acpi"], ""),
    ("nicdrv", "drivers: NIC drivers (e1000/virtio-net) + NDIS", "net", "norm", ["net"], ""),
    ("virtio", "drivers: virtio family (blk/net/gpu/input) for QEMU", "drivers", "norm", ["io"], ""),
    ("battery", "drivers: ACPI battery/thermal/fan", "drivers", "low", ["acpi"], ""),
    ("nouveau", "drivers: port Linux nouveau (GPL) + GSP firmware", "drivers", "norm", ["kms", "gfxfw"], "docs/DRIVERS.md"),
    ("nvk", "lin: Mesa nvk (Vulkan) on nouveau -> D3D via d3dvk", "lin-bridge", "low", ["mesa", "nouveau"], ""),
    ("nvwddm", "drivers: WDDM/DXGK miniport host for NVIDIA Windows driver", "drivers", "low", ["amdwddm"], "proprietary; deprioritised"),
    ("nvd3d", "graphics: D3D UMD host + runtime for NVIDIA Windows driver", "graphics", "low", ["nvwddm"], ""),
    ("cuda", "lin: open compute (Vulkan compute/rusticl); proprietary CUDA optional", "lin-bridge", "low", ["nvk"], ""),

    # ---- graphics: default display path + vendor GPU (docs/DRIVERS.md) ----
    ("gfxgop", "boot: pass UEFI GOP / legacy VBE framebuffer to the kernel", "boot", "high", ["boot"], "docs/DRIVERS.md"),
    ("gfxbdd", "graphics: basic display driver (firmware framebuffer -> compositor)", "graphics", "high", ["gfxgop", "comp"], "docs/DRIVERS.md"),
    ("virtgpu", "drivers: virtio-gpu 2D + virgl/venus 3D for the dev VM", "drivers", "high", ["virtio", "comp"], ""),
    ("kms", "graphics: DRM/KMS-compatible display class", "graphics", "high", ["dispdrv"], "docs/DRIVERS.md"),
    ("gfxfw", "drivers: GPU firmware (microcode) loading, no telemetry", "drivers", "norm", ["kms"], ""),
    ("amdgpu", "drivers: port Linux amdgpu (GPL) onto the KMS display class", "drivers", "norm", ["kms", "gfxfw"], "docs/DRIVERS.md"),
    ("mesa", "lin: Mesa (radeonsi/radv) on the Lin layer -> D3D via d3dvk", "lin-bridge", "norm", ["L6", "d3dvk"], ""),
    ("amdwddm", "drivers: WDDM/DXGK miniport host (dxgkrnl) for Windows AMD drivers", "drivers", "low", ["dispdrv", "d2d"], "proprietary; deprioritised"),
    ("amdd3d", "graphics: D3D UMD host + runtime for Windows AMD drivers", "graphics", "low", ["amdwddm"], ""),

    # ---- filesystems ----
    ("mountmgr", "fs: mount manager + volume GUID + drive letters", "fs", "norm", ["vfs"], ""),
    ("fsd", "fs: filesystem driver model (IRP_MJ_* + mini-filter hooks)", "fs", "norm", ["vfs"], ""),
    ("btrfssnap", "fs: btrfs subvolumes/snapshots umbrella (see granular btrfs-* tasks)", "fs", "norm", ["btrfsw"], "docs/BTRFS.md"),
    # full btrfs feature set + 100% stability (docs/BTRFS.md)
    ("btrfs-cow", "fs: btrfs full copy-on-write for data + metadata", "fs", "high", ["btrfsw"], "docs/BTRFS.md"),
    ("btrfs-csum", "fs: btrfs checksum tree (crc32c/xxhash/sha256/blake2) + verify + repair from mirror", "fs", "high", ["btrfsw"], ""),
    ("btrfs-snap", "fs: btrfs snapshots (read-only + writable) + rollback", "fs", "high", ["btrfssnap"], ""),
    ("btrfs-subvol", "fs: btrfs subvolumes (nested, default, readonly/nodatacow flags)", "fs", "high", ["btrfssnap"], ""),
    ("btrfs-sendreceive", "fs: btrfs send/receive incremental streams", "fs", "norm", ["btrfs-snap"], ""),
    ("btrfs-compress", "fs: btrfs compression (zstd/zlib/lzo/lz4) + mount heuristics", "fs", "norm", ["btrfs-cow"], ""),
    ("btrfs-raid", "fs: btrfs RAID0/1/1c3/1c4/10/5/6 (write-hole-correct)", "fs", "high", ["btrfs-cow"], ""),
    ("btrfs-device", "fs: btrfs multi-device add/remove/replace/seed", "fs", "norm", ["btrfs-raid"], ""),
    ("btrfs-balance", "fs: btrfs balance (data/metadata/system chunks)", "fs", "norm", ["btrfs-device"], ""),
    ("btrfs-scrub", "fs: btrfs scrub + auto-repair from mirrors", "fs", "norm", ["btrfs-csum"], ""),
    ("btrfs-freestree", "fs: btrfs free-space tree v2 / space cache", "fs", "norm", ["btrfs-cow"], ""),
    ("btrfs-qgroup", "fs: btrfs qgroups + subvolume quotas", "fs", "norm", ["btrfs-subvol"], ""),
    ("btrfs-defrag", "fs: btrfs defrag + autodefrag (compression-aware)", "fs", "low", ["btrfs-compress"], ""),
    ("btrfs-dedupe", "fs: btrfs block/ref deduplication", "fs", "low", ["btrfs-csum"], ""),
    ("btrfs-reflink", "fs: btrfs reflink (FICLONE/clone_range, cp --reflink)", "fs", "norm", ["btrfs-cow"], ""),
    ("btrfs-prealloc", "fs: btrfs fallocate/preallocate/sparse + inline extents", "fs", "norm", ["btrfs-cow"], ""),
    ("btrfs-xattr", "fs: btrfs xattrs + POSIX ACLs + capabilities", "fs", "norm", ["btrfsw"], ""),
    ("btrfs-trim", "fs: btrfs discard/trim + zoned-device support", "fs", "low", ["btrfs-device"], ""),
    ("btrfs-swap", "fs: btrfs swapfile support (CoW-safe)", "fs", "low", ["btrfs-cow"], ""),
    ("btrfs-resize", "fs: btrfs online grow/shrink", "fs", "low", ["btrfs-device"], ""),
    ("btrfs-ioctl", "fs: btrfs full ioctl surface", "fs", "norm", ["btrfs-subvol"], ""),
    ("btrfs-check", "fs: btrfs tools - check/restore/rescue (btrfs-progs equivalent)", "fs", "norm", ["btrfs-csum"], ""),
    ("btrfs-boot", "fs: boot the kernel from btrfs (loader reader or FAT boot)", "fs", "high", ["btrfsw"], ""),
    ("btrfs-stab", "fs: btrfs 100% stability program (fuzz + power-loss + soak + crash consistency, check-clean)", "fs", "high", ["btrfsw"], ""),
    ("instfs", "install: installer FS is btrfs-only (no NTFS/ext4/exFAT choice)", "installer", "norm", ["instcore"], "docs/BTRFS.md"),
    ("ntfs", "fs: NTFS read-only (legacy volumes; never a system FS)", "fs", "low", ["fsd"], "docs/BTRFS.md"),
    ("fatfs", "fs: FAT (ESP/boot only; not a user FS choice)", "fs", "low", ["fsd"], "docs/BTRFS.md"),
    ("iso9660", "fs: ISO9660/UDF mount", "fs", "low", ["fsd"], ""),
    ("reghivetx", "registry: transaction log + REGAPI completeness", "fs", "norm", ["reghive"], ""),
    ("shadow", "fs: shadow copies + restore points", "fs", "low", ["btrfssnap"], ""),

    # ---- networking ----
    ("tcpip", "net: full TCP/IP stack (IPv4/IPv6, routing, ICMP, loopback)", "net", "high", ["net"], ""),
    ("dnsdhcp", "net: DNS + DHCP client + IP helper", "net", "norm", ["net"], ""),
    ("wfp", "net: firewall + packet-filter engine (WFP-like)", "net", "norm", ["net"], ""),
    ("http", "net: HTTP client/server (winhttp/http.sys)", "net", "norm", ["net"], ""),
    ("schannel", "security: TLS/SSL (schannel) + root certificate store", "security", "norm", ["crypto"], ""),
    ("smb", "net: SMB client/server", "net", "low", ["tcpip"], ""),
    ("rdp", "net: Remote Desktop (RDP server)", "net", "low", ["comp"], ""),
    ("vpn", "net: VPN/PPP", "net", "low", ["tcpip"], ""),

    # ---- security / system ----
    ("sam", "security: local accounts + SAM/LSA", "security", "high", ["se"], ""),
    ("logonui", "security: LogonUI + credential providers + secure desktop", "security", "norm", ["sam"], ""),
    ("uac", "security: UAC + elevation broker", "security", "norm", ["se"], ""),
    ("bitlocker", "security: full-disk encryption (BitLocker-like)", "crypto", "norm", ["crypto"], ""),
    ("codesign", "security: code signing + driver signature enforcement + secure-boot chain", "security", "norm", ["boot"], ""),
    ("appcontainer", "security: AppContainer/sandbox + capability ACLs", "security", "low", ["se"], ""),
    ("eventlog", "security: event log (EVTX) + ETW-like tracing", "backend", "norm", ["M2"], ""),
    ("efs", "security: per-file encryption (EFS-like)", "crypto", "low", ["bitlocker"], ""),

    # ---- Win32 breadth ----
    ("kernelbase", "win32: kernelbase split + forwarder completeness", "win32", "norm", ["k32"], ""),
    ("ole32f", "win32: OLE/COM full (ole32/oleaut32) + type libraries", "win32", "norm", ["ole32"], ""),
    ("rpc", "win32: RPC runtime + local DCOM", "win32", "low", ["ole32f"], ""),
    ("ucrt", "win32: UCRT full (ucrtbase) + locale/multibyte/NLS", "win32", "high", ["M7"], ""),
    ("msvcrt", "win32: MSVC runtime (vcruntime/msvcp) + C++ stdlib", "win32", "norm", ["ucrt"], ""),
    ("dotnet", "win32: .NET runtime hosting (CLR) for managed apps", "win32", "low", ["ucrt"], ""),
    ("u32full", "win32: user32 full surface parity", "win32", "norm", ["u32"], ""),
    ("gdi32full", "win32: gdi32 full surface parity", "win32", "norm", ["gdi32"], ""),
    ("dwrite", "graphics: DirectWrite + font stack (Fonts)", "graphics", "norm", ["gdi32"], ""),
    ("d2d", "graphics: Direct2D/Direct3D11 full + shader compiler", "graphics", "norm", ["d3dvk"], ""),
    ("mf", "win32: Media Foundation/DirectShow codecs", "graphics", "low", ["d2d"], ""),
    ("directsound", "audio: DirectSound/WASAPI + media codecs (WMF)", "audio", "low", ["audiodrv"], ""),
    ("conhost", "win32: console host (conhost) + VT sequences", "win32", "norm", ["k32"], ""),
    ("wow64", "win32: WOW64 (run 32-bit PE)", "win32", "norm", ["M7"], ""),
    ("uia", "win32: UI Automation (UIA) + IME/TSF", "win32", "low", ["u32full"], ""),
    ("ws2full", "win32: winsock full (WSA*/ioctl completeness)", "win32", "norm", ["ws2"], ""),
    ("tasksched", "backend: Task Scheduler service", "backend", "norm", ["scm"], ""),
    ("wmi", "backend: WMI/CIM provider + performance counters", "backend", "low", ["tasksched"], ""),

    # ---- package managers (winget + apt, native) ----
    ("pkgd", "pkg: unified package service daemon (winget + apt + MSIX + MSI)", "pkg", "high", ["M7"], "docs/PACKAGING.md"),
    ("pkgapsolver", "pkg: dependency/version SAT solver shared by all formats", "pkg", "high", ["pkgd"], ""),
    ("pkgwinget", "pkg: winget-compatible CLI (install/search/upgrade/source)", "pkg", "high", ["pkgd"], "docs/PACKAGING.md"),
    ("pkgwingetman", "pkg: winget manifest (YAML) parser + repository index", "pkg", "norm", ["pkgwinget"], ""),
    ("pkgmsix", "pkg: MSIX/APPX model + install/register/provision", "pkg", "norm", ["pkgd"], ""),
    ("pkgmsi", "pkg: MSI installer engine (msiexec) + custom actions", "pkg", "norm", ["pkgd"], ""),
    ("pkgapt", "pkg: native apt/dpkg engine (install/remove/upgrade .deb, no chroot)", "pkg", "high", ["pkgd"], "docs/PACKAGING.md"),
    ("pkgaptrepo", "pkg: apt repository client + GPG verify + sources.list", "pkg", "norm", ["pkgapt"], ""),
    ("pkgdpkg", "pkg: dpkg status db (/var/lib/dpkg) + dependency resolver", "pkg", "norm", ["pkgapt"], ""),
    ("pkgtrust", "pkg: package trust/signing + allowlist + privacy gate", "pkg", "norm", ["pkgd"], ""),
    ("pkgrollback", "pkg: uninstall/rollback + repair + file ownership tracking", "pkg", "norm", ["pkgd"], ""),
    ("pkgoffline", "pkg: sideload offline .msix/.deb/.msi", "pkg", "low", ["pkgd"], ""),
    ("pkgcache", "pkg: repo mirrors + local cache + delta downloads", "pkg", "low", ["pkgd"], ""),
    ("pkgcli", "pkg: CLI 'loos pkg' (install/remove/update/search/list)", "pkg", "high", ["pkgd"], ""),
    ("pkgbase", "pkg: distro base bootstrap (Ubuntu/Debian rootfs into a LoOS namespace)", "lin-base", "norm", ["L4"], ""),
    ("pkgwsl", "pkg: winget 'msstore' source + custom REST source", "pkg", "low", ["pkgwinget"], ""),

    # ---- system apps / services ----
    ("svchost", "sysapp: service host (svchost) + services management", "backend", "norm", ["scm"], ""),
    ("explorer", "sysapp: File Explorer (tabs, preview, thumbnails)", "ui", "norm", ["U1", "bFs"], ""),
    ("startmenu", "sysapp: Start menu + search index", "ui", "norm", ["U1"], ""),
    ("searchidx", "backend: content search indexer", "backend", "low", ["explorer"], ""),
    ("devmgr", "sysapp: Device Manager + Control Panel", "ui", "norm", ["pnp"], ""),
    ("diskmgmt", "sysapp: Disk Management + optimize/trim", "ui", "low", ["partmgr"], ""),
    ("eventvwr", "sysapp: Event Viewer + Reliability Monitor", "ui", "low", ["eventlog"], ""),
    ("controlpanel", "sysapp: Settings app (full)", "ui", "norm", ["U3"], ""),
    ("updateui", "sysapp: LoOS Update (Windows Update-like UI)", "ui", "norm", ["updatesvc"], ""),
    ("lostore", "sysapp: LoOS Store (package-manager UI)", "ui", "norm", ["pkgd"], ""),
    ("loterm", "sysapp: Terminal (multi-tab, LoCmd + Lin shells)", "ui", "norm", ["conhost"], ""),
    ("accessories", "sysapp: WordPad/Paint/Photos/Media Player set", "ui", "low", ["u32full", "gdi32full"], ""),
    ("taskmgrui", "sysapp: Task Manager UI wired to Ps.snapshot", "ui", "norm", ["bPs"], ""),
    ("clipboard", "sysapp: clipboard + clipboard history + sharing", "ui", "low", ["U1"], ""),
    ("notifcenter", "sysapp: Action Center / notification center", "ui", "norm", ["unotif"], ""),
    ("themes", "sysapp: themes/dark mode + skinning", "design", "low", ["U1"], ""),
    ("remoteassist", "sysapp: Remote assistance / Quick Assist", "net", "low", ["rdp"], ""),

    # ---- installer / deployment / recovery ----
    ("instcore", "install: OS installer (partition, format btrfs, deploy image, bootloader)", "installer", "high", ["boot", "btrfsw"], "docs/INSTALL.md"),
    ("instunattend", "install: unattended/answer files + OEM provisioning", "installer", "norm", ["instcore"], ""),
    ("instmedia", "install: USB/DVD media builder", "installer", "norm", ["instcore"], ""),
    ("instupgrade", "install: upgrade-in-place + migration", "installer", "norm", ["instcore"], ""),
    ("instdrv", "install: offline driver injection + WinPE-like boot image", "installer", "low", ["instcore"], ""),
    ("instdual", "install: dual-boot manager (LoOS + Windows + Linux)", "installer", "norm", ["boot"], ""),
    ("winre", "recovery: recovery environment + repair/rollback", "recovery", "norm", ["instcore"], ""),
    ("factoryreset", "recovery: factory reset + refresh", "recovery", "norm", ["winre"], ""),

    # ---- updates ----
    ("updatesvc", "update: OTA update service (delta + atomic) + signing", "update", "high", ["M10"], ""),
    ("updateab", "update: snapshot/A-B rollback updates", "update", "norm", ["updatesvc"], ""),
    ("updatestage", "update: staged rollout + update servers (telemetry-free)", "update", "norm", ["updatesvc"], ""),
    ("updatefw", "update: in-place driver/firmware updates", "update", "low", ["updatesvc"], ""),

    # ---- time / power ----
    ("time", "time: NTP client + timezone + RTC calendar", "time", "norm", ["M2"], ""),
    ("power", "power: power manager (sleep/hibernate/shutdown) + ACPI S3/S4", "power", "norm", ["acpi"], ""),
    ("energysaver", "power: battery/thermal/energy saver", "power", "low", ["power"], ""),

    # ---- virtualization / containers ----
    ("hyperv", "virt: type-1 hypervisor (Hyper-V-like) [optional]", "virt", "low", ["hal"], ""),
    ("wsl", "virt: WSL-like Linux subsystem integration into the shell", "lin-bridge", "norm", ["L6"], ""),
    ("containers", "virt: application container namespace (isolation)", "security", "low", ["appcontainer"], ""),

    # ---- printing ----
    ("printspool", "print: spooler + GDI printing + IPP", "print", "low", ["gdi32full"], ""),
    ("printdrv", "print: printer drivers + PnP print", "print", "low", ["printspool"], ""),

    # ---- compat extras ----
    ("compatdb", "compat: app compatibility database (shims) + verifier", "compat", "low", ["cshim"], ""),
    ("dx9", "compat: legacy DirectX 9 + old GDI for games", "graphics", "low", ["d3dvk"], ""),
    ("win16", "compat: 16-bit/DOS subsystem [optional]", "compat", "low", ["u32full"], ""),

    # ---- diagnostics ----
    ("kdump", "diag: kernel crash dump + local-only crash reporting", "backend", "low", ["kd"], ""),
]

# Tasks from an earlier seed that the board above supersedes; cancel if present.
RETIRE = [
    "api-set resolver + schema (apisets)",
    "PE parser + import binder + reloc (M5)",
    "expand API-set contract map",
]


def print_plan(tasks, owners):
    from collections import Counter
    counts = Counter(owners.values())
    print("owner distribution (computed):")
    for m in MODELS:
        print(f"  {m:8} {counts.get(m, 0)}")
    by_cat = {}
    for key, title, cat, *_ in tasks:
        by_cat.setdefault(cat, []).append(owners[key])
    print("by category:")
    for cat in sorted(by_cat):
        c = Counter(by_cat[cat])
        span = " ".join(f"{m}={c[m]}" for m in MODELS if c[m])
        print(f"  {cat:12} {span}")


def main() -> int:
    owners = plan_owners(TASKS)
    if "--print-plan" in sys.argv:
        print_plan(TASKS, owners)
        return 0

    b = Bus("user")
    existing_title = {t["title"]: t for t in b.task_list()}
    ids: dict[str, int] = {}
    added = skipped = reassigned = 0
    for key, title, cat, prio, deps, notes in TASKS:
        owner = owners[key]
        if title in existing_title:
            t = existing_title[title]
            ids[key] = t["id"]
            if t.get("owner") != owner:
                b.task_update(t["id"], owner=owner)
                reassigned += 1
            skipped += 1
            continue
        dep_ids = [ids[d] for d in deps if d in ids]
        tid = b.task_add(title, owner=owner, priority=prio,
                         notes=notes, deps=dep_ids)
        ids[key] = tid
        added += 1

    total = len(b.task_list())
    print(f"added {added}, skipped {skipped} existing "
          f"(reassigned {reassigned}); board now has {total}")

    retired = 0
    by_title = {t["title"]: t for t in b.task_list()}
    for title in RETIRE:
        t = by_title.get(title)
        if t and t["state"] not in ("done", "cancelled"):
            b.task_update(t["id"], state="cancelled",
                          notes=(t.get("notes") or "") + " [superseded by reseed]")
            retired += 1
    if retired:
        print(f"cancelled {retired} superseded task(s)")

    cur: dict[str, int] = {}
    for t in b.task_list():
        cur[t["owner"]] = cur.get(t["owner"], 0) + 1
    for o, n in sorted(cur.items()):
        print(f"  {o or '(none)':8} {n}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
