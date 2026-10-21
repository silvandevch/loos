#!/usr/bin/env python3
"""Assemble a BIOS boot image: MBR | stage 2 (64 sectors) | boot table (2 sectors) | boot files.

usage: mkbios.py OUT mbr.bin stage2.bin kernel.elf [--module NAME=PATH ...] [--cmdline TEXT] [--size-mib N]
The table layout is shared with boot/bios/stage2.S and frontend.c."""
import argparse
import struct

SECT = 512
STAGE2_SECTORS = 64
TABLE_LBA = 1 + STAGE2_SECTORS
TABLE_SECTORS = 2
CONFIG_LBA = TABLE_LBA + TABLE_SECTORS      # machine configuration written by the installer
CONFIG_SECTORS = 8
ENTRY = struct.Struct("<16sQIIQ")           # name, lba, sectors, bytes, dest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("mbr")
    ap.add_argument("stage2")
    ap.add_argument("kernel")
    ap.add_argument("--module", action="append", default=[])
    ap.add_argument("--cmdline", default="")
    ap.add_argument("--size-mib", type=int, default=0)
    ap.add_argument("--installer", action="store_true", help="embed the whole image as the \"bootimg\" module (implied by the installer command-line word)")
    ap.add_argument("--video", default="", help="preferred video mode WxH (default 1920x1080)")
    a = ap.parse_args()

    mbr = open(a.mbr, "rb").read()
    st = open(a.stage2, "rb").read()
    assert len(mbr) == 512, "stage 1 must be exactly one sector"
    assert len(st) <= STAGE2_SECTORS * SECT, f"stage 2 is {len(st)} bytes, limit {STAGE2_SECTORS * SECT}"
    files = [("kernel", open(a.kernel, "rb").read())]
    for m in a.module:
        name, path = m.split("=", 1)
        assert len(name) < 16, "module names are at most 15 characters"
        files.append((name, open(path, "rb").read()))
    assert len(files) <= 7, "at most 6 modules"
    cmd = a.cmdline.encode()
    assert len(cmd) < 232, "command line too long"

    installer = "installer" in a.cmdline.split() or a.installer
    first_lba = CONFIG_LBA + CONFIG_SECTORS
    lba = first_lba
    entries, payload, placed = [], b"", []
    for name, data in files:                       # kernel and modules, sector aligned after the config area
        sectors = (len(data) + SECT - 1) // SECT
        placed.append((name, lba, sectors, len(data)))
        payload += data.ljust(sectors * SECT, b"\0")
        lba += sectors
    end_lba = lba
    # table order: kernel first, the (always present) config area, the installer's copy of the whole media, then modules
    order = [placed[0], ("config", CONFIG_LBA, CONFIG_SECTORS, CONFIG_SECTORS * SECT)]
    if installer:
        order.append(("bootimg", 0, end_lba, end_lba * SECT))
    order += placed[1:]
    assert len(order) <= 9, "too many boot files"
    for name, l, sectors, nbytes in order:
        entries.append(ENTRY.pack(name.encode(), l, sectors, nbytes, 0))
    files_count = len(order)
    video = 0
    if a.video:
        vw, vh = (int(x) for x in a.video.lower().split("x"))
        video = vw | vh << 16
    table = b"LOOSBTAB" + struct.pack("<II", files_count, video) + cmd.ljust(232, b"\0") + b"".join(entries)
    assert len(table) <= TABLE_SECTORS * SECT
    img = mbr + st.ljust(STAGE2_SECTORS * SECT, b"\0") + table.ljust(TABLE_SECTORS * SECT, b"\0") + bytes(CONFIG_SECTORS * SECT) + payload
    size = max(len(img), a.size_mib << 20, 1 << 20)
    open(a.out, "wb").write(img.ljust((size + SECT - 1) // SECT * SECT, b"\0"))


if __name__ == "__main__":
    main()
