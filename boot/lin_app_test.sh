#!/usr/bin/env bash
# Runs a real host GUI application in the LoOS Linux layer (Wayland, software rendering) and proves it
# draws and reacts: once its window is up, take a screenshot, type "loos", take another; the window must
# change (apps that ignore letters list their own keys). Screenshots land in out/lin-app/<app>/.
#   lin_app_test.sh <app>          apps: see the table below (lin_app_test.sh list)
#   STRACE=1 / STRACE=46,47        syscall tracing, as in lin_dyn_test.sh
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
app=${1:-}
# Data every desktop toolkit needs: fonts, icons, schemas, keyboard layouts, MIME database.
common="/etc/fonts /usr/share/fonts/adwaita-sans-fonts /usr/share/fonts/google-noto-vf /usr/share/glib-2.0/schemas /usr/share/icons/Adwaita /usr/share/icons/hicolor /usr/share/xkeyboard-config-2 /usr/share/mime /usr/bin/bash /usr/bin/sh /usr/bin/uname /usr/bin/getconf"
qt="/usr/lib64/qt6/plugins/platforms /usr/lib64/qt6/plugins/wayland-shell-integration /usr/lib64/qt6/plugins/wayland-decoration-client /usr/lib64/qt6/plugins/wayland-graphics-integration-client /usr/lib64/qt6/plugins/imageformats /usr/lib64/qt6/plugins/iconengines /usr/lib64/qt6/plugins/xcbglintegrations /usr/lib64/qt6/plugins/platformthemes /usr/lib64/qt6/plugins/styles /usr/share/icons/breeze"
# Discord installs itself per user; only the program folder is shipped (never the user's profile).
discord=$(ls -d "$HOME"/.config/discord/app-* 2>/dev/null | sort -V | tail -1)
# Chromium-based apps dlopen NSS's crypto modules (not visible to ldd).
nss="/usr/lib64/libsoftokn3.so /usr/lib64/libfreeblpriv3.so /usr/lib64/libfreebl3.so /usr/lib64/libnssckbi.so"
electron="--no-sandbox --ozone-platform=wayland --disable-gpu --disable-gpu-compositing --no-zygote --disable-dev-shm-usage --in-process-gpu"
# name | rootfs paths (besides $common) | environment | command line | keys to type (QEMU names; default l o o s)
table() { cat <<EOF
firefox|/usr/lib64/firefox /usr/lib64/libgtk-3.so.0 /usr/lib64/gdk-pixbuf-2.0|MOZ_DISABLE_CONTENT_SANDBOX=1 MOZ_CRASHREPORTER_DISABLE=1|/usr/lib64/firefox/firefox --no-remote --profile /tmp/ffprofile about:blank
kcalc|$qt /usr/bin/kcalc|QT_QPA_PLATFORM=wayland QT_QUICK_BACKEND=software|/usr/bin/kcalc|1 2 3
kwrite|$qt /usr/bin/kwrite /usr/lib64/qt6/plugins/kf6 /usr/share/katepart5 /usr/share/kf6|QT_QPA_PLATFORM=wayland|/usr/bin/kwrite
vscode|$nss /usr/share/code /usr/bin/code|ELECTRON_DISABLE_SANDBOX=1|/usr/share/code/code $electron --user-data-dir=/tmp/code --disable-extensions
discord|$nss $discord|ELECTRON_DISABLE_SANDBOX=1|$discord/Discord $electron --user-data-dir=/tmp/discord
EOF
}
if [ -z "$app" ] || [ "$app" = list ]; then table | cut -d'|' -f1; exit 0; fi
line=$(table | grep "^$app|") || { echo "unknown app $app (lin_app_test.sh list)"; exit 2; }
IFS='|' read -r _ files envs cmd keys <<< "$line"
files=$(for f in $common $files; do [ -e "$f" ] && echo "$f"; done)   # skip what this host lacks
out=${LOOS_BOOT_OUT:-$root/out/lin-app/$app}; mkdir -p "$out"; out=$(cd "$out" && pwd)
args=$out/app.args              # too long for the kernel command line: loos.lin=@file reads it from the rootfs
{
    case ${STRACE:-} in '') ;; 1) echo strace ;; *) echo "strace:$STRACE" ;; esac
    printf '%s\n' $envs ${LIN_ENV:-} $cmd
} > "$args"
sockdir=$(mktemp -d); trap 'rm -rf "$sockdir"' EXIT   # the data drive cannot hold Unix sockets
mon=$sockdir/monitor.sock; log=$out/lin-dyn.log; rm -f "$log" "$out"/shot-*.ppm
hmp() { echo "$1" | socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1; sleep 0.3; }
drive() {
    until grep -aq "wayland window" "$log" 2>/dev/null; do sleep 0.5; done
    sleep ${SETTLE:-15}                            # let the app finish its first paints
    hmp "screendump $out/shot-before.ppm"
    for k in ${keys:-l o o s}; do hmp "sendkey $k"; done
    sleep 5
    hmp "screendump $out/shot-typed.ppm"
    hmp "quit"
}
drive & driver=$!
LIN_BOOT=${LIN_BOOT:-uefi} LIN_DISK_IF=${LIN_DISK_IF:-virtio} LOOS_BOOT_OUT=$out LOOS_KVM=${LOOS_KVM:-auto} \
QEMU_DISPLAY=${QEMU_DISPLAY:-gtk} QEMU_MEM=${QEMU_MEM:-6G} QEMU_EXTRA="-monitor unix:$mon,server,nowait" TIMEOUT=${TIMEOUT:-150} \
ROOTFS_BINS="$args $files" "$here/lin_dyn_test.sh" "@$args" > "$out/build.log" 2>&1
kill $driver 2>/dev/null; wait   # QEMU is gone: stop waiting for a window that never came
grep -a "^lin: wayland window\|^lin: .*running," "$log" | tail -2
ok=1
for s in before typed; do [ -s "$out/shot-$s.ppm" ] || { echo "no screenshot $s (window never came up)"; ok=0; }; done
[ $ok = 1 ] && ! cmp -s "$out/shot-before.ppm" "$out/shot-typed.ppm" || { [ $ok = 1 ] && echo "window did not change after typing"; ok=0; }
[ $ok = 1 ] && echo "lin app test ok: $app" || { echo "lin app test FAILED: $app"; exit 1; }
