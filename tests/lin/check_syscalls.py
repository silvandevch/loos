#!/usr/bin/env python3
"""Check the generated Linux syscall diagnostic table against its source map."""

from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
TAB = ROOT / "tools/linux_syscalls.tab"
OUT = ROOT / "crates/loos-lin/src/syscalls_generated.rs"

subprocess.run(["python3", str(ROOT / "tools/gen_linux_syscalls.py")], check=True)
rows = []
for line in TAB.read_text().splitlines():
    line = line.split("#", 1)[0].strip()
    if line:
        number, name = line.split()
        rows.append((int(number), name))

generated = OUT.read_text()
for number, name in rows:
    needle = f'{number} => "{name}"'
    if needle not in generated:
        raise SystemExit(f"missing generated mapping: {needle}")

if len(re.findall(r'^\s+\d+ => "', generated, re.MULTILINE)) != len(rows):
    raise SystemExit("generated syscall mapping count differs from source map")

print(f"linux syscall table ok ({len(rows)} entries)")
