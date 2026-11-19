#!/usr/bin/env bash
# Stages LoOS's Windows runtime for the Linux layer: the host's Wine (WoW64 build, so 32-bit programs need no
# 32-bit Linux libraries) with DXVK/vkd3d and Mesa's software Vulkan (lavapipe), stripped of debug info, plus a
# ready-made Wine prefix (C: drive and registry) so the first start does not spend minutes in wineboot.
#   wine-runtime.sh OUTDIR       -> prints the lin-rootfs.sh arguments (one per line) that ship it
# The stage is cached per Wine version in OUTDIR; WINE_I386=0 leaves out 32-bit Windows DLLs (~200 MB).
set -euo pipefail
out=${1:?output dir}
lib=/usr/lib64/wine-wow64/wine
[ -d "$lib/x86_64-windows" ] || { echo "wine-runtime: host Wine (WoW64 build) not found in $lib" >&2; exit 1; }
ver=$(wine64 --version 2>/dev/null || echo unknown)
ver=${ver//[^A-Za-z0-9.]/_}; stage=$out/wine-rt-$ver-i386${WINE_I386:-1}
if [ ! -e "$stage/.done" ]; then
    rm -rf "$stage"; mkdir -p "$stage$lib" "$stage/usr/share/wine"
    archs="x86_64-windows x86_64-unix"; [ "${WINE_I386:-1}" = 1 ] && archs="$archs i386-windows"
    for a in $archs; do
        mkdir -p "$stage$lib/$a"
        for f in "$lib/$a"/*; do
            d=$stage$lib/$a/$(basename "$f")
            case $a in
                *-windows) x86_64-w64-mingw32-strip --strip-debug -o "$d" "$f" 2>/dev/null || cp "$f" "$d" ;;  # keeps the "Wine builtin DLL" stub
                *) cp "$f" "$d"; strip --strip-debug "$d" 2>/dev/null || true ;;
            esac
        done
    done
    cp -r /usr/share/wine/nls /usr/share/wine/fonts /usr/share/wine/wine.inf "$stage/usr/share/wine/"
    # The prefix belongs to the guest's only user (uid 1000, HOME=/home): created here with that name so the
    # profile folder is C:\users\user. Mono and Gecko are left out (.NET / built-in browser control are optional).
    # Built in a temporary folder (OUTDIR may sit on a file system without symbolic links, like exFAT); its links
    # (dosdevices/c: -> ../drive_c, z: -> /) are recorded in links.txt and recreated in the rootfs.
    tmp=$(mktemp -d); pfx=$tmp/.wine
    WINEPREFIX=$pfx USER=user WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml=" DISPLAY= WAYLAND_DISPLAY= \
        wine64 wineboot -i >/dev/null 2>&1 || { echo "wine-runtime: wineboot failed" >&2; rm -rf "$tmp"; exit 1; }
    WINEPREFIX=$pfx wineserver -w
    rm -rf "$pfx/drive_c/users/user/AppData/Local/Temp"/* 2>/dev/null || true
    # Wine copies its builtin DLLs into system32/syswow64: strip those too (Wine recognises them by the stub).
    find "$pfx/drive_c/windows" -type f \( -iname '*.dll' -o -iname '*.exe' -o -iname '*.drv' -o -iname '*.sys' \) \
        -exec x86_64-w64-mingw32-strip --strip-debug {} + 2>/dev/null || true
    # Only the prefix's own links ship (c: and z:); links into this host (its home folders, disks) become
    # plain folders or are dropped.
    (cd "$tmp" && find .wine -type l | while read -r l; do
        t=$(readlink "$l")
        case $t in
            /) echo "ln:/home/$l=/" ;;
            /*) case $l in */dosdevices/*) ;; *) mkdir -p "$stage/home/$l" ;; esac ;;
            *) echo "ln:/home/$l=$t" ;;
        esac
    done) > "$stage/links.txt"
    mkdir -p "$stage/home"; (cd "$tmp" && tar --exclude-from=<(find .wine -type l) -cf - .wine) | tar -C "$stage/home" -xf -
    rm -rf "$tmp"
    touch "$stage/.done"
fi
cat <<EOF
/usr/bin/wine64
/usr/bin/wineserver
ln:/usr/bin/wine=wine64
$stage$lib=$lib
$stage/usr/share/wine=/usr/share/wine
$stage/home/.wine=/home/.wine
/usr/lib64/libvulkan.so.1
/usr/lib64/libvulkan_lvp.so
/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
EOF
cat "$stage/links.txt"
