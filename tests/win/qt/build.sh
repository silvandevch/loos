#!/usr/bin/env bash
# Builds the Qt Widgets compatibility target and its runtime bundle without installing anything system-wide:
# Fedora's MinGW Qt 6 and g++ packages are downloaded (dnf download, no root) and unpacked into QT_SDK (default
# ~/.cache/loos-qt: Qt's headers are symlinks, so the SDK needs a local file system, not exFAT).
#   tests/win/qt/build.sh [OUT]   (default out/qt) -> OUT/app/qtwidgets.exe, its DLLs, platforms/qwindows.dll,
#                                                     and OUT/extra.txt for boot/gui_test.sh's EXTRA=
# Run in QEMU:  EXTRA="$(cat out/qt/extra.txt)" QEMU_MEM=2G LOOS_KVM=on boot/gui_test.sh out/qt/app/qtwidgets.exe shots/qt.png
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/../../..
out=$(realpath -m "${1:-$root/out/qt}"); sdk=${QT_SDK:-${XDG_CACHE_HOME:-$HOME/.cache}/loos-qt}
mkdir -p "$out/app/platforms" "$sdk/rpm" "$sdk/root"
if [ ! -x "$sdk/root/usr/bin/x86_64-w64-mingw32-g++" ]; then
    (cd "$sdk/rpm" && dnf download --skip-unavailable --resolve --alldeps mingw64-qt6-qtbase mingw64-gcc-c++ >/dev/null 2>&1)
    for f in "$sdk"/rpm/mingw64-*.rpm; do (cd "$sdk/root" && rpm2cpio "$f" | cpio -idm --quiet 2>/dev/null); done
fi
q=$sdk/root; sys=$q/usr/x86_64-w64-mingw32/sys-root; R=$sys/mingw
gcc_dir=$(ls -d "$q"/usr/libexec/gcc/x86_64-w64-mingw32/*/ | head -1)
"$q/usr/bin/x86_64-w64-mingw32-g++" -B"$gcc_dir" --sysroot="$sys" -I"$R/include/qt6" -I"$R/include/qt6/QtCore" -I"$R/include/qt6/QtGui" \
    -I"$R/include/qt6/QtWidgets" -O2 -std=c++17 -mwindows "$here/qtwidgets.cpp" -L"$R/lib" -lQt6Widgets -lQt6Gui -lQt6Core -o "$out/app/qtwidgets.exe"
python3 - "$R" "$out/app" <<'PY'
import os, re, shutil, subprocess, sys
R, D = sys.argv[1:]
look = {}
for d in (f"{R}/bin", f"{R}/lib/qt6/plugins/platforms"):
    for f in os.listdir(d):
        if f.lower().endswith(".dll"): look.setdefault(f.lower(), f"{d}/{f}")
deps = lambda p: [m.lower() for m in re.findall(r"DLL Name: (\S+)", subprocess.run(["x86_64-w64-mingw32-objdump", "-p", p], capture_output=True, text=True).stdout)]
todo, seen = [f"{D}/qtwidgets.exe", look["qwindows.dll"]], set()
while todo:
    for d in deps(todo.pop()):
        if d in look and d not in seen: seen.add(d); todo.append(look[d])
for d in seen: shutil.copy(look[d], D)
shutil.copy(look["qwindows.dll"], f"{D}/platforms/")
PY
for f in "$out"/app/*.dll; do printf '%s=Windows/System32/%s ' "$f" "$(basename "$f")"; done > "$out/extra.txt"
printf '%s=Windows/System32/platforms/qwindows.dll ' "$out/app/platforms/qwindows.dll" >> "$out/extra.txt"
for f in "$root"/branding/assets/fonts/*.ttf; do printf '%s=Windows/Fonts/%s ' "$f" "$(basename "$f")"; done >> "$out/extra.txt"
echo >> "$out/extra.txt"
echo "$out/app/qtwidgets.exe"
