#!/usr/bin/env python3
"""Builds a real btrfs image with mkfs.btrfs -r and runs ffi_btrfs_test on it (skips when mkfs.btrfs is absent)."""
import pathlib, shutil, subprocess, sys, tempfile

SKIP = 77

def main() -> int:
    if len(sys.argv) != 2:
        return 2
    mkfs = shutil.which("mkfs.btrfs")
    if not mkfs:
        print("SKIP: mkfs.btrfs is not installed")
        return SKIP
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d) / "root"
        (root / "sub").mkdir(parents=True)
        (root / "hello.txt").write_bytes(b"hello btrfs\n")
        (root / "sub" / "big.bin").write_bytes(bytes(i * 7 % 251 for i in range(300000)))
        img = pathlib.Path(d) / "image.img"
        with open(img, "wb") as f:
            f.truncate(256 * 1024 * 1024)
        subprocess.run([mkfs, "-q", "-f", "--nodiscard", "-r", str(root), str(img)], check=True)
        return subprocess.run([sys.argv[1], str(img)]).returncode

sys.exit(main())
