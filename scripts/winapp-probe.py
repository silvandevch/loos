#!/usr/bin/env python3
"""winapp-probe — download real Windows CLI apps, run them on the LoOS compat
layer, and log every API that is missing so we know exactly what to implement.

It never stops on a failure: each app is analysed, the gaps are recorded, and it
moves on. For every executable it produces, per imported DLL:

  * how many imports resolve against the LoOS implementations we ship
  * the exact list of missing symbols (and whole DLLs we do not provide yet)

Two sources of truth are combined:
  1. static   — parse each PE's import table (pure-Python, no deps) and diff it
                against the set of APIs implemented under src/win32/.
  2. runtime  — actually load the exe through the LoOS loader harness
                (tests/win32/app) when it has been built, and record the first
                import the loader cannot satisfy.

    python3 scripts/winapp-probe.py [--out DIR] [--cache DIR] [--apps a,b] [--list]

Exit code is 0 even when apps fail — the point is the report, not a green run.
"""
from __future__ import annotations
import argparse, json, os, re, struct, subprocess, sys, tempfile, urllib.request, zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SRC = REPO / "src" / "win32"

# ---- real Windows command-line apps (name -> {url, kind, exes}) ----------------
# kind: "zip" (unzip) or "file" (single exe). exes: substrings to pick out.
APPS = {
    "git": {
        "url": "https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.1/MinGit-2.47.1-64-bit.zip",
        "kind": "zip", "exes": ["mingw64/bin/git.exe", "cmd/git.exe"],
    },
    "jq": {
        "url": "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe",
        "kind": "file", "exes": ["jq-windows-amd64.exe"],
    },
    "ripgrep": {
        "url": "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-pc-windows-msvc.zip",
        "kind": "zip", "exes": ["rg.exe"],
    },
    "busybox": {
        "url": "https://frippery.org/files/busybox/busybox64.exe",
        "kind": "file", "exes": ["busybox64.exe"],
    },
    "7zip": {
        "url": "https://www.7-zip.org/a/7zr.exe",
        "kind": "file", "exes": ["7zr.exe"],
    },
}

# ---- implemented-API scan -----------------------------------------------------
def scan_implemented():
    """Return {dll_lower: set(symbol)} for the APIs LoOS implements in src/win32."""
    impl: dict[str, set] = {"kernel32.dll": set(), "ntdll.dll": set()}
    api_re = re.compile(r"\b(?:WINAPI|NTAPI)\s+([A-Za-z_]\w*)\s*\(")
    for c in (SRC / "kernel32").glob("*.c"):
        impl["kernel32.dll"].update(api_re.findall(c.read_text(errors="ignore")))
    nd = SRC / "ntdll"
    for c in nd.glob("*.c"):
        impl["ntdll.dll"].update(api_re.findall(c.read_text(errors="ignore")))
    tbl = nd / "syscalls.tbl"
    if tbl.exists():
        for line in tbl.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                impl["ntdll.dll"].add(line.split()[0])
    extra = nd / "extra_exports.txt"
    if extra.exists():
        for line in extra.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                impl["ntdll.dll"].add(line.split()[0])
    return impl


# ---- PE import parsing (PE32 and PE32+) --------------------------------------
def pe_imports(data: bytes):
    if len(data) < 0x40 or data[:2] != b"MZ":
        return None
    e_lfanew = struct.unpack_from("<I", data, 0x3C)[0]
    if data[e_lfanew:e_lfanew + 4] != b"PE\x00\x00":
        return None
    coff = e_lfanew + 4
    machine, nsec, _ts, _ps, _ns, sizeopt, _ch = struct.unpack_from("<HHIIIHH", data, coff)
    opt = coff + 20
    magic = struct.unpack_from("<H", data, opt)[0]
    plus = magic == 0x20B
    dd = opt + (112 if plus else 96)
    import_rva = struct.unpack_from("<I", data, dd + 1 * 8)[0]
    secs = []
    so = opt + sizeopt
    for i in range(nsec):
        s = so + 40 * i
        vsize, vaddr, rawsize, rawptr = struct.unpack_from("<IIII", data, s + 8)
        secs.append((vaddr, max(vsize, rawsize), rawptr, rawsize))

    def r2o(rva):
        for vaddr, vsz, rawptr, rawsz in secs:
            if vaddr <= rva < vaddr + vsz and rva - vaddr < rawsz:
                return rawptr + (rva - vaddr)
        return None

    def cstr(off):
        end = data.find(b"\0", off)
        return data[off:end].decode("latin1") if end > 0 else ""

    out = {}
    off = r2o(import_rva) if import_rva else None
    if off is None:
        return out
    while True:
        oft, _ts, _fc, name_rva, ft = struct.unpack_from("<IIIII", data, off)
        if oft == 0 and name_rva == 0 and ft == 0:
            break
        no = r2o(name_rva)
        dll = cstr(no).lower() if no is not None else "?"
        thunk_rva = oft or ft
        to = r2o(thunk_rva)
        syms = []
        if to is not None:
            step = 8 if plus else 4
            fmt = "<Q" if plus else "<I"
            hi = (1 << 63) if plus else (1 << 31)
            j = 0
            while True:
                v = struct.unpack_from(fmt, data, to + j * step)[0]
                if v == 0:
                    break
                if v & hi:
                    syms.append("#%d" % (v & 0xFFFF))
                else:
                    ho = r2o(v & 0x7FFFFFFF)
                    syms.append(cstr(ho + 2) if ho is not None else "?")
                j += 1
        out.setdefault(dll, []).extend(syms)
        off += 20
    return out


def which_exes(root: Path, patterns):
    found = []
    for pat in patterns:
        p = root / pat
        if p.exists():
            found.append(p)
    if not found:  # fall back: any exe whose name matches the basenames
        want = {os.path.basename(p).lower() for p in patterns}
        for p in root.rglob("*.exe"):
            if p.name.lower() in want:
                found.append(p)
    return found


def analyze(path: Path, impl):
    data = path.read_bytes()
    imports = pe_imports(data)
    if imports is None:
        return {"error": "not a PE"}
    report = {}
    for dll, syms in imports.items():
        have = impl.get(dll, set())
        missing = sorted({s for s in syms if s not in have})
        report[dll] = {
            "imports": sorted(set(syms)),
            "implemented": sorted({s for s in syms if s in have}),
            "missing": missing,
            "dll_known": dll in impl,
        }
    return {"imports": report}


def runtime_probe(harness: Path, workdir: Path, exe_name: str):
    if not harness.exists():
        return {"skipped": "harness not built"}
    try:
        r = subprocess.run([str(harness), str(workdir), exe_name],
                           capture_output=True, text=True, timeout=30)
    except Exception as e:  # noqa: BLE001
        return {"error": str(e)}
    out = (r.stdout + r.stderr).strip()
    m = re.search(r"load failed: module=(\S+) symbol=(\S*)", out)
    first = {"dll": m.group(1), "symbol": m.group(2)} if m else None
    return {"rc": r.returncode, "first_failure": first, "tail": out.splitlines()[-4:]}


def fetch(url: str, dest: Path):
    if dest.exists() and dest.stat().st_size > 0:
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": "loos-winapp-probe"})
    with urllib.request.urlopen(req, timeout=120) as r, open(dest, "wb") as f:
        f.write(r.read())
    return dest


def prepare(name, spec, cache: Path, tmp: Path):
    url = spec["url"]
    fn = url.split("/")[-1]
    got = fetch(url, cache / fn)
    out = tmp / name
    if spec["kind"] == "zip":
        out.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(got) as z:
            z.extractall(out)
        return out
    out.mkdir(parents=True, exist_ok=True)
    (out / fn).write_bytes(got.read_bytes())
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(REPO / "out" / "winapp"))
    ap.add_argument("--cache", default=str(REPO / "out" / "winapp" / "cache"))
    ap.add_argument("--apps", default="", help="comma list; default all")
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.list:
        for k, v in APPS.items():
            print(f"{k:10} {v['url']}")
        return 0

    out_root = Path(args.out); out_root.mkdir(parents=True, exist_ok=True)
    cache = Path(args.cache)
    impl = scan_implemented()
    print(f"LoOS implements: kernel32.dll={len(impl['kernel32.dll'])} ntdll.dll={len(impl['ntdll.dll'])} exports\n")

    harness = None
    for cand in [Path(os.environ.get("OUT_ROOT", "/nonexistent")),
                 REPO / "out",
                 Path("/run/media/silvan/DATA/loos/out")]:
        p = cand / "cmake" / "host-clang" / "tests" / "win32" / "app_compat"
        if p.exists():
            harness = p
            break
    if harness is None:
        harness = REPO / "out" / "cmake" / "host-clang" / "tests" / "win32" / "app_compat"
    print(f"harness: {harness}{'' if harness.exists() else '  (not built - runtime probe skipped)'}")
    names = [a.strip() for a in args.apps.split(",") if a.strip()] or list(APPS)
    result = {}

    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for name in names:
            spec = APPS.get(name)
            if not spec:
                print(f"[{name}] unknown app, skipping"); continue
            print(f"=== {name} ===")
            try:
                root = prepare(name, spec, cache, tmp)
            except Exception as e:  # noqa: BLE001
                print(f"  download/extract failed: {e}"); result[name] = {"error": str(e)}; continue
            exes = which_exes(root, spec["exes"])
            result[name] = {}
            for exe in exes:
                rel = str(exe.relative_to(root))
                info = analyze(exe, impl)
                if "error" in info:
                    print(f"  {rel}: {info['error']}"); result[name][rel] = info; continue
                total = have = 0
                miss_apis = []
                for dll, d in info["imports"].items():
                    total += len(d["imports"]); have += len(d["implemented"])
                    if d["missing"]:
                        miss_apis.append((dll, d["missing"]))
                pct = (100 * have // total) if total else 0
                print(f"  {rel}: {have}/{total} imports resolve ({pct}%)")
                for dll, miss in miss_apis:
                    if not impl.get(dll, None):
                        print(f"     {dll}: {len(miss)} imports, DLL NOT IMPLEMENTED")
                    else:
                        shown = ", ".join(miss[:12]) + (" ..." if len(miss) > 12 else "")
                        print(f"     {dll}: missing {len(miss)}: {shown}")
                info["coverage_pct"] = pct
                info["runtime"] = runtime_probe(harness, root, rel)
                if info["runtime"].get("first_failure"):
                    ff = info["runtime"]["first_failure"]
                    print(f"     runtime: first unsatisfied import -> {ff['dll']}!{ff['symbol']}")
                result[name][rel] = info

    rep = out_root / "winapp-coverage.json"
    rep.write_text(json.dumps({"implemented": {k: sorted(v) for k, v in impl.items()},
                               "apps": result}, indent=2))
    print(f"\nreport: {rep}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
