#!/usr/bin/env python3
"""Run the reader against a filesystem made by btrfs-progs when available."""

from __future__ import annotations

import pathlib
import shutil
import subprocess
import sys
import tempfile


SKIP = 77
IMAGE_SIZE = 256 * 1024 * 1024


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} TEST_BINARY", file=sys.stderr)
        return 2
    mkfs = shutil.which("mkfs.btrfs")
    if mkfs is None:
        print("SKIP: mkfs.btrfs is not installed")
        return SKIP

    with tempfile.TemporaryDirectory(prefix="loos-btrfs-root-") as root_name, \
        tempfile.TemporaryDirectory(prefix="loos-btrfs-image-") as image_name:
        root = pathlib.Path(root_name)
        image = pathlib.Path(image_name) / "real-btrfs.img"
        (root / "subdir").mkdir()
        (root / "hello.txt").write_bytes(b"hello from LoOS\n")
        (root / "regular.bin").write_bytes(
            bytes((index * 13 + 7) & 0xFF for index in range(0x2000))
        )
        (root / "subdir" / "nested.txt").write_bytes(b"nested from LoOS\n")
        with image.open("wb") as handle:
            handle.truncate(IMAGE_SIZE)
        subprocess.run(
            [mkfs, "-q", "-f", "--rootdir", str(root), str(image)],
            check=True,
        )
        return subprocess.run([sys.argv[1], str(image)]).returncode


if __name__ == "__main__":
    raise SystemExit(main())
