#!/usr/bin/env python3
"""
resolve_patchkit_conflicts.py — resolve overlay conflicts in favour of patchkit.

The patchkit is the openxp x64 patchset: it adds AMD64 source/tool additions.
Where the base tree (anika-style repack) already had a file at the same path,
02-overlay-patchkit.sh skipped it by default. This resolves those conflicts by
taking the patchkit version, after:

  * backing up the base version (backups/patchkit-resolve-<stamp>/<rel>)
  * normalising line endings of text files to the base tree's convention
    (so .cmd files stay CRLF and the tree stays consistent)
  * leaving binaries byte-for-byte

Driven by env: NT_SRC, PATCHKIT, BACKUP_DIR, LOGS_DIR.
Usage: resolve_patchkit_conflicts.py [--dry-run]
"""
from __future__ import annotations
import datetime as _dt
import os
import shutil
import sys
from pathlib import Path


def env(name: str) -> str:
    v = os.environ.get(name)
    if not v:
        sys.exit(f"missing env {name} (source env.sh first)")
    return v


def is_binary(b: bytes) -> bool:
    return b"\0" in b[:8192]


def main() -> int:
    NT = Path(env("NT_SRC"))
    PK = Path(env("PATCHKIT"))
    BACKUP_DIR = Path(env("BACKUP_DIR"))
    LOGS_DIR = Path(env("LOGS_DIR"))
    dry = "--dry-run" in sys.argv[1:]

    stamp = _dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    bkdir = BACKUP_DIR / f"patchkit-resolve-{stamp}"
    LOGS_DIR.mkdir(parents=True, exist_ok=True)
    report = LOGS_DIR / f"patchkit-resolve-{stamp}.tsv"

    rows = []
    n_eol = n_text = n_bin = n_skip = 0
    for p in sorted(PK.rglob("*")):
        if not p.is_file() or ".git" in p.parts:
            continue
        rel = p.relative_to(PK).as_posix()
        dst = NT / rel
        if not dst.exists():
            continue  # additive — handled by the overlay script
        sb = p.read_bytes()
        db = dst.read_bytes()
        if sb == db:
            n_skip += 1
            continue

        if is_binary(sb) or is_binary(db):
            new = sb
            kind = "binary"
            n_bin += 1
        else:
            crlf = db.count(b"\r\n") * 2 >= max(1, db.count(b"\n"))
            t = sb.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
            if crlf:
                t = t.replace(b"\n", b"\r\n")
            new = t
            if new == db:
                kind = "eol-only"
                n_eol += 1
            else:
                kind = "text-crlf" if crlf else "text-lf"
                n_text += 1

        if kind != "eol-only" and not dry:
            bkp = bkdir / rel
            bkp.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(dst, bkp)
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(new)

        rows.append((kind, len(db), len(new), rel))

    with report.open("w") as f:
        f.write("kind\tsize_before\tsize_after\trel\n")
        for r in rows:
            f.write(f"{r[0]}\t{r[1]}\t{r[2]}\t{r[3]}\n")

    print(f"{'DRY-RUN ' if dry else ''}resolved conflicts: {len(rows)}")
    print(f"  text rewritten : {n_text}")
    print(f"  binary copied  : {n_bin}")
    print(f"  eol-only (noop): {n_eol}")
    print(f"  already equal  : {n_skip}")
    if not dry and (n_text or n_bin):
        print(f"  backups        : {bkdir}")
    print(f"  report         : {report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
