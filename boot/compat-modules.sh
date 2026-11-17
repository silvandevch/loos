#!/usr/bin/env bash
# Builds the compatibility-layer file systems the boot media carries (shared by ./loos qemu and ./loos install-iso):
#   compat-modules.sh OUTDIR   -> prints "winroot=OUTDIR/winroot.tar" and "initrd=OUTDIR/initrd.tar" (one per line)
# winroot: Windows/System32 PE images for the Windows layer (loos-winrt).
# initrd:  the Linux layer's root (/bin, /etc) plus macOS programs and .app bundles for Prism, which runs Mach-O
#          inside the same Linux process model. Prism samples are skipped with a warning when the Mach-O toolchain is missing.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:?output dir}; mkdir -p "$out"
winrt=$out/winroot.tar; linrt=$out/initrd.tar
python3 - "$root" "$winrt" <<'PY'
import os, sys, tarfile
root, dst = sys.argv[1:3]
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("Windows", "Windows/System32", "Windows/Fonts", "Users", "Users/user"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    for src in ("tests/win/hello_gui.exe", "apps/loos-notepad/notepad.exe", "apps/loos-cmd/cmd.exe", "tests/win/nettest.exe",
                "tests/win/selftest.exe", "tests/win/threads.exe", "tests/win/seh.exe", "tests/win/testdll.dll",
                "apps/calc/LoCalc.exe", "apps/loos-taskmgr/taskmgr.exe"):
        if os.path.exists(f"{root}/{src}"):
            t.add(f"{root}/{src}", arcname=f"Windows/System32/{os.path.basename(src)}")
    fonts = f"{root}/branding/assets/fonts"   # open fonts for DirectWrite/GDI (Inter, Liberation)
    for f in sorted(os.listdir(fonts)):
        if f.endswith(".ttf"):
            t.add(f"{fonts}/{f}", arcname=f"Windows/Fonts/{f}")
PY
cc=(clang --target=x86_64-linux-gnu -static -nostdlib -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone)
"${cc[@]}" "$root/tests/lin/lbox.c" -o "$out/lbox"
"${cc[@]}" -I"$root/src/lin" "$root/tests/lin/gui.c" "$root/tests/lin/looswin_client.c" -o "$out/gui"
prism=$out/prism
if ! "$root/tests/prism/build.sh" "$prism" >"$out/prism-build.log" 2>&1; then
    echo "compat-modules: macOS samples skipped (Mach-O toolchain missing, see $out/prism-build.log)" >&2; prism=""
fi
python3 - "$out" "$linrt" "$prism" <<'PY'
import io, os, sys, tarfile
out, dst, prism = sys.argv[1:4]
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    def mkdir(name):
        i = tarfile.TarInfo(name); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    for d in ("bin", "etc", "home", "tmp", "Applications"):
        mkdir(d)
    for n in ("lbox", "gui"):
        t.add(f"{out}/{n}", arcname=f"bin/{n}")
    m = b"LoOS says hello from the initrd\n"
    i = tarfile.TarInfo("etc/motd"); i.size = len(m); i.mode = 0o644; t.addfile(i, io.BytesIO(m))
    if prism:
        for a in ("x86_64", "arm64"):
            for p in ("hello", "objc", "cf"):
                t.add(f"{prism}/{p}_{a}", arcname=f"bin/mac_{p}_{a}")
            t.add(f"{prism}/Found_{a}.app", arcname=f"Applications/Found_{a}.app")   # recursive: MacOS, Info.plist, Resources
PY
echo "winroot=$winrt"
echo "initrd=$linrt"
