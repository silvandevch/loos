#!/usr/bin/env bash
# Builds the compatibility-layer file systems the boot media carries (shared by ./loos qemu and ./loos install-iso):
#   compat-modules.sh OUTDIR   -> prints "winroot=OUTDIR/winroot.tar" and "initrd=OUTDIR/initrd.tar" (one per line)
# winroot: Windows/System32 PE images for the Windows layer (loos-winrt).
# initrd:  the compat runtime's root (Programs volume in Files): Linux root (/bin, /etc), macOS programs and .app
#          bundles for Prism, Windows programs, and with LOOS_WINE=1 Wine itself. Prism samples are skipped with a
#          warning when the Mach-O toolchain is missing.
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
# The initrd is the compat runtime's root: the Programs volume in Files, where a double-click starts Linux,
# Windows (Wine) and macOS (Prism) programs. Built by lin-rootfs.sh (shared libraries, links, /etc).
# Toolchain output goes to a local temporary folder: linkers wrote broken files on the exFAT data drive.
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
items=("$out/lbox=/bin/lbox" "$out/gui=/bin/gui")
printf 'LoOS says hello from the initrd\n' > "$work/motd"; items+=("$work/motd=/etc/motd")
if "$root/tests/prism/build.sh" "$work/prism" >"$out/prism-build.log" 2>&1; then
    for a in x86_64 arm64; do
        for p in hello objc cf; do items+=("$work/prism/${p}_$a=/bin/mac_${p}_$a"); done
        items+=("$work/prism/Found_$a.app=/Applications/Found_$a.app")
    done
else
    echo "compat-modules: macOS samples skipped (Mach-O toolchain missing, see $out/prism-build.log)" >&2
fi
# Windows programs (double-click them in Files > Programs > home > Windows Programs). They run on Wine, which
# is ~0.8 GB staged: LOOS_WINE=1 ships it (and needs that much more guest memory); without it they cannot start.
for src in apps/loos-notepad/notepad.exe apps/loos-cmd/cmd.exe apps/calc/LoCalc.exe apps/loos-taskmgr/taskmgr.exe tests/win/hello_gui.exe; do
    [ -e "$root/$src" ] && items+=("$root/$src=/home/Windows Programs/$(basename "$src")")
done
if [ "${LOOS_WINE:-0}" = 1 ]; then
    while IFS= read -r w; do items+=("$w"); done < <("$root/tools/wine-runtime.sh" "$root/out/wine")
fi
"$root/tools/lin-rootfs.sh" "$linrt" "${items[@]}" >/dev/null
echo "winroot=$winrt"
echo "initrd=$linrt"
