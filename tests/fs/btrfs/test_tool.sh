#!/usr/bin/env bash
set -eu

check_tool=$1
mkfs_tool=$2
work=$3
mkdir -p "$work"
image="$work/check.img"

"$mkfs_tool" "$image" 200 LoOS-check >/dev/null

python3 - "$check_tool" "$image" <<'PY'
import json
import subprocess
import sys

tool, image = sys.argv[1:]

def run(*args, expected):
    result = subprocess.run([tool, "--json", *args], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode != expected:
        raise SystemExit(f"unexpected exit {result.returncode}: {result.stdout} {result.stderr}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid JSON: {result.stdout!r}: {error}")
    return value

clean = run("check", image, expected=0)
assert clean["status"] == 0 and clean["status_name"] == "ok"
assert clean["superblocks_valid"] == 2
assert clean["chunks_checked"] == 3

with open(image, "r+b") as stream:
    stream.seek(0x10000 + 299)
    byte = stream.read(1)
    stream.seek(0x10000 + 299)
    stream.write(bytes([byte[0] ^ 1]))

damaged = run("check", image, expected=1)
assert damaged["status"] == -8 and damaged["status_name"] == "corrupt"
assert damaged["first_bad_offset"] == 0x10000

restored = run("restore-primary", image, "0x4000000", expected=0)
assert restored["status"] == 0 and restored["status_name"] == "ok"
assert run("check", image, expected=0)["status"] == 0

invalid = run("restore-primary", image, "123", expected=1)
assert invalid["status"] == -1 and invalid["status_name"] == "invalid_argument"
PY

echo "btrfs_check: all passed"
