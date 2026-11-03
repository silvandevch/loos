#!/usr/bin/env python3
"""Run the Linux compatibility corpus through a supplied LoOS launcher."""

import argparse
import json
import subprocess
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", type=Path, default=root / "tests/lin/corpus.json")
    ap.add_argument("--launcher", help="override launcher command; defaults to each case")
    args = ap.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if manifest.get("version") != 1 or not isinstance(manifest.get("cases"), list):
        raise SystemExit("lin corpus: invalid manifest")
    for case in manifest["cases"]:
        ident = case.get("id")
        command = args.launcher or case.get("launcher")
        if not isinstance(ident, str) or not ident or not isinstance(command, str):
            raise SystemExit(f"lin corpus: malformed case {case!r}")
        argv = command.split() + [str(x) for x in case.get("args", [])]
        result = subprocess.run(argv, cwd=root, text=True, capture_output=True)
        output = result.stdout + result.stderr
        if result.returncode != 0:
            print(output, end="")
            raise SystemExit(f"lin corpus: {ident} exited {result.returncode}")
        expected = case.get("expect")
        if expected and expected not in output:
            print(output, end="")
            raise SystemExit(f"lin corpus: {ident} missing marker {expected!r}")
        print(f"lin corpus: {ident}: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
