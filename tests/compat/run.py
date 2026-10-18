#!/usr/bin/env python3
"""Validate and run LoOS app compatibility corpus entries."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
from typing import Any

ERAS = {"7", "10", "11", "loos"}


def load_cases(path: Path) -> list[dict[str, Any]]:
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read manifest {path}: {exc}") from exc
    if not isinstance(doc, dict) or doc.get("version") != 1 or not isinstance(doc.get("cases"), list):
        raise ValueError("manifest must have version=1 and a cases array")

    seen: set[str] = set()
    cases = []
    for i, case in enumerate(doc["cases"]):
        where = f"cases[{i}]"
        if not isinstance(case, dict):
            raise ValueError(f"{where} must be an object")
        case_id, era, exe = case.get("id"), case.get("era"), case.get("exe")
        if not isinstance(case_id, str) or not case_id or case_id in seen:
            raise ValueError(f"{where}.id must be a unique nonempty string")
        seen.add(case_id)
        if era not in ERAS:
            raise ValueError(f"{where}.era must be one of {', '.join(sorted(ERAS))}")
        if not isinstance(exe, str) or not exe:
            raise ValueError(f"{where}.exe must be a nonempty path")
        exe_path = Path(exe)
        if not exe_path.is_absolute():
            exe_path = path.parent / exe_path
        if not exe_path.is_file():
            raise ValueError(f"{where}.exe does not exist: {exe_path}")
        argv = case.get("argv", [])
        if not isinstance(argv, list) or any(not isinstance(arg, str) for arg in argv):
            raise ValueError(f"{where}.argv must be an array of strings")
        cwd = case.get("cwd")
        if cwd is not None and not isinstance(cwd, str):
            raise ValueError(f"{where}.cwd must be a path string")
        cwd_path = Path(cwd) if cwd else exe_path.parent
        if not cwd_path.is_absolute():
            cwd_path = path.parent / cwd_path
        if not cwd_path.is_dir():
            raise ValueError(f"{where}.cwd does not exist: {cwd_path}")
        cases.append({"id": case_id, "era": era, "exe": exe_path, "argv": argv, "cwd": cwd_path})
    return cases


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(os.environ.get("LOOS_COMPAT_MANIFEST", "tests/compat/corpus.json")))
    parser.add_argument("--era", choices=sorted(ERAS), help="select one Windows era or LoOS apps")
    parser.add_argument("--list", action="store_true", help="validate and list selected cases without launching")
    parser.add_argument("--launcher", help="launcher command; executable, args, and cwd are appended as argv")
    args = parser.parse_args()

    try:
        cases = load_cases(args.manifest.resolve())
    except ValueError as exc:
        print(f"compat: {exc}", file=sys.stderr)
        return 2
    selected = [case for case in cases if args.era is None or case["era"] == args.era]
    if not selected:
        if args.list:
            print(f"no cases registered for selection in {args.manifest}")
            return 0
        print(f"compat: no cases selected from {args.manifest}", file=sys.stderr)
        return 2
    if not args.list and not args.launcher:
        print("compat: --launcher is required to run cases (use --list to validate only)", file=sys.stderr)
        return 2

    for case in selected:
        print(f"[{case['era']}] {case['id']}: {case['exe']}", flush=True)
        if args.list:
            continue
        command = shlex.split(args.launcher)
        if not command:
            print("compat: launcher command is empty", file=sys.stderr)
            return 2
        try:
            result = subprocess.run(command + [str(case["exe"]), *case["argv"]], cwd=case["cwd"], check=False)
        except OSError as exc:
            print(f"compat: {case['id']}: cannot start launcher: {exc}", file=sys.stderr)
            return 2
        if result.returncode:
            print(f"compat: {case['id']} failed with status {result.returncode}", file=sys.stderr)
            return result.returncode if result.returncode > 0 else 1
    if args.list:
        print(f"validated {len(selected)} case(s); no apps launched")
    else:
        print(f"passed {len(selected)} launched case(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
