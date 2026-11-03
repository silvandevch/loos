#!/usr/bin/env python3
"""Pack a Linux root tree or unpack a Debian data archive without scripts."""

import argparse
import io
import pathlib
import tarfile


def pack(src: pathlib.Path, out: pathlib.Path) -> None:
    with tarfile.open(out, "w", format=tarfile.USTAR_FORMAT) as tar:
        for path in sorted(src.rglob("*")):
            rel = path.relative_to(src).as_posix()
            info = tar.gettarinfo(str(path), arcname=rel)
            if info.isreg():
                with path.open("rb") as f:
                    tar.addfile(info, f)
            else:
                tar.addfile(info)


def data_tar(deb: pathlib.Path) -> bytes:
    raw = deb.read_bytes()
    if not raw.startswith(b"!<arch>\n"):
        raise SystemExit("not a Debian ar archive")
    off = 8
    while off < len(raw):
        hdr = raw[off:off + 60]
        if len(hdr) != 60 or hdr[58:60] != b"`\n":
            raise SystemExit("malformed ar header")
        size = int(hdr[48:58].strip() or b"0")
        body = off + 60
        payload = raw[body:body + size]
        name = hdr[:16].decode("ascii", "strict").strip()
        if name == "data.tar":
            return payload
        if name.startswith("data.tar."):
            raise SystemExit(f"compressed {name} requires an offline decoder")
        off = body + size + (size & 1)
    raise SystemExit("data.tar member not found")


def unpack(deb: pathlib.Path, dst: pathlib.Path) -> None:
    dst.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data_tar(deb)), mode="r:") as tar:
        for member in tar:
            name = pathlib.PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts or any(not p for p in name.parts):
                raise SystemExit(f"unsafe archive path: {member.name!r}")
            target = dst.joinpath(*name.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.issym():
                link = pathlib.PurePosixPath(member.linkname)
                if link.is_absolute() or ".." in link.parts:
                    raise SystemExit(f"unsafe symlink: {member.name!r}")
                target.parent.mkdir(parents=True, exist_ok=True)
                target.unlink(missing_ok=True)
                target.symlink_to(member.linkname)
            elif member.isreg():
                target.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as source, target.open("wb") as out:
                    out.write(source.read())
                target.chmod(member.mode & 0o7777)
            else:
                raise SystemExit(f"unsupported archive member: {member.name!r}")


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="mode", required=True)
    p = sub.add_parser("pack")
    p.add_argument("root", type=pathlib.Path)
    p.add_argument("output", type=pathlib.Path)
    u = sub.add_parser("unpack-deb")
    u.add_argument("package", type=pathlib.Path)
    u.add_argument("root", type=pathlib.Path)
    args = ap.parse_args()
    if args.mode == "pack":
        pack(args.root, args.output)
    else:
        unpack(args.package, args.root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
