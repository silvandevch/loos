#!/usr/bin/env bash
# Runs a real host GUI application in the LoOS Linux layer (Wayland, software rendering) and proves it
# draws and reacts: once its window is up, take a screenshot, type "loos", take another; the window must
# change (apps that ignore letters list their own keys). Screenshots land in out/lin-app/<app>/.
#   lin_app_test.sh <app>          apps: see the table below (lin_app_test.sh list)
#   STRACE=1 / STRACE=46,47 / STRACE=46,47@name   syscall tracing (@name: first process whose argv has name; @147: that pid)
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
app=${1:-}
# Data every desktop toolkit needs: fonts, icons, schemas, keyboard layouts, MIME database, and glycin's image
# loaders (gdk-pixbuf decodes PNG/SVG icons through them; GTK aborts when none is configured).
common="/usr/share/glycin-loaders /usr/libexec/glycin-loaders /etc/fonts /usr/share/fonts/adwaita-sans-fonts /usr/share/fonts/google-noto-vf /usr/share/glib-2.0/schemas /usr/share/icons/Adwaita /usr/share/icons/hicolor /usr/share/xkeyboard-config-2 /usr/share/mime /usr/bin/bash /usr/bin/sh /usr/bin/uname /usr/bin/getconf"
qt="/usr/lib64/qt6/plugins/platforms /usr/lib64/qt6/plugins/wayland-shell-integration /usr/lib64/qt6/plugins/wayland-decoration-client /usr/lib64/qt6/plugins/wayland-graphics-integration-client /usr/lib64/qt6/plugins/imageformats /usr/lib64/qt6/plugins/iconengines /usr/lib64/qt6/plugins/xcbglintegrations /usr/lib64/qt6/plugins/platformthemes /usr/lib64/qt6/plugins/styles /usr/share/icons/breeze"
# Discord installs itself per user; only the program folder is shipped (never the user's profile).
discord=$(ls -d "$HOME"/.config/discord/app-* 2>/dev/null | sort -V | tail -1)
# Chromium-based apps dlopen NSS's crypto modules (not visible to ldd).
nss="/usr/lib64/libsoftokn3.so /usr/lib64/libfreeblpriv3.so /usr/lib64/libfreebl3.so /usr/lib64/libnssckbi.so"
electron="--no-sandbox --ozone-platform=wayland --disable-gpu --disable-gpu-compositing --no-zygote --disable-dev-shm-usage --in-process-gpu"
# macOS Electron apps (Prism runs them on Linux Electron): slim copies of the .app bundles from a Mac
# (Contents/Info.plist, MacOS, Resources and the Electron Framework's Info.plist) in out/mac-apps/;
# Linux Electron installs on this host stand in as the runtimes in /usr/lib/prism/electron/<major>.
mac=${MAC_APPS:-$root/out/mac-apps}; pe=/usr/lib/prism/electron
# Windows programs run on Wine in the Linux layer (wine-runtime.sh stages it once per Wine version) with the
# prepared prefix in /home/.wine, Wine's Wayland driver, and DXVK/vkd3d on Mesa's software Vulkan.
mem=6G   # Wine's runtime and prefix live in the in-memory rootfs: Windows programs need more
case $app in win-*) mem=12G; wine=$("$root/tools/wine-runtime.sh" "$root/out/wine" | tr '\n' ' ') || exit 1 ;; *) wine= ;; esac
# Windows apps from a Windows install (extracted, e.g. 7z x "Firefox Setup.exe" core) live in out/win-apps/<name>.
winapps=${WIN_APPS:-$root/out/win-apps}
# Firefox runs with its defaults under Wine (forcing its sandboxes or launcher off makes this build crash).
ffwin="MOZ_CRASHREPORTER_DISABLE=1"
wenv="WINEPREFIX=/home/.wine HOME=/home USER=user WINEDLLOVERRIDES=mscoree,mshtml= WINEDEBUG=${WINEDEBUG:-err+all,fixme-all} VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json"
# name | rootfs paths (besides $common) | environment | command line | keys to type (QEMU names; default l o o s)
table() { cat <<EOF
firefox|/usr/lib64/firefox /usr/lib64/libgtk-3.so.0 /usr/lib64/gdk-pixbuf-2.0|MOZ_DISABLE_CONTENT_SANDBOX=1 MOZ_CRASHREPORTER_DISABLE=1 HOME=/tmp|/usr/lib64/firefox/firefox --no-remote about:blank|ctrl-l l o o s
kcalc|$qt /usr/bin/kcalc|QT_QPA_PLATFORM=wayland QT_QUICK_BACKEND=software|/usr/bin/kcalc|1 2 3
kwrite|$qt /usr/bin/kwrite /usr/lib64/qt6/plugins/kf6 /usr/share/katepart5 /usr/share/kf6|QT_QPA_PLATFORM=wayland|/usr/bin/kwrite|ctrl-n l o o s
vscode|$nss /usr/share/code /usr/bin/code /usr/bin/bash|ELECTRON_DISABLE_SANDBOX=1 SHELL=/usr/bin/bash|/usr/share/code/code $electron --user-data-dir=/tmp/code --disable-extensions|ctrl-n l o o s
discord|$nss $discord|ELECTRON_DISABLE_SANDBOX=1|$discord/Discord $electron --user-data-dir=/tmp/discord
mac-discord|$nss $discord=$pe/44 ln:$pe/44/electron=Discord $discord/updater.node=$pe/apps/com.hnc.Discord/updater.node $mac/Discord.app=/Applications/Discord.app||/Applications/Discord.app|tab tab
mac-claude|$nss $discord=$pe/44 ln:$pe/44/electron=Discord $mac/Claude.app=/Applications/Claude.app||/Applications/Claude.app
mac-vscode|$nss /usr/share/code=$pe/42 ln:$pe/42/electron=code $mac/VSCode.app=/Applications/VSCode.app||/Applications/VSCode.app --disable-extensions
win-notepad|$wine|$wenv|/usr/bin/wine64 notepad|l o o s
win-firefox|$wine $winapps/firefox=/home/.wine/drive_c/firefox|$wenv $ffwin|/usr/bin/wine64 /home/.wine/drive_c/firefox/firefox.exe -no-remote -wait-for-browser about:blank|ctrl-l l o o s
win-winecfg|$wine|$wenv|/usr/bin/wine64 winecfg|tab tab
EOF
}
if [ -z "$app" ] || [ "$app" = list ]; then table | cut -d'|' -f1; exit 0; fi
line=$(table | grep "^$app|") || { echo "unknown app $app (lin_app_test.sh list)"; exit 2; }
IFS='|' read -r _ files envs cmd keys <<< "$line"
files=$(for f in $common $files; do s=${f%%=/*}; case $f in ln:*) echo "$f" ;; *) [ -e "$s" ] && echo "$f" ;; esac; done)   # skip what this host lacks
# Multi-GB rootfs and disk images: built on the fast local disk (the repo drive is slow exFAT).
out=${LOOS_BOOT_OUT:-${LOOS_BUILD:-$HOME/.cache/loos-build}/lin-app/$app}; mkdir -p "$out"; out=$(cd "$out" && pwd)
args=$out/app.args              # too long for the kernel command line: loos.lin=@file reads it from the rootfs
{
    case ${STRACE:-} in '') ;; 1) echo strace ;; @*) echo "strace$STRACE" ;; *) echo "strace:$STRACE" ;; esac   # STRACE=@explorer: one process
    printf '%s\n' $envs ${LIN_ENV:-} $cmd
} > "$args.new"
cmp -s "$args.new" "$args" && rm -f "$args.new" || mv "$args.new" "$args"   # unchanged: keeps the cached rootfs valid
sockdir=$(mktemp -d); trap 'rm -rf "$sockdir"' EXIT   # the data drive cannot hold Unix sockets
mon=$sockdir/monitor.sock; log=$out/lin-dyn.log; rm -f "$log" "$out"/shot-*.ppm
hmp() { echo "$1" | socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1; sleep 0.3; }
dead="Rust panic\|kernel panic\|^lin: program finished\|^lin: deadlock\|^lin: out of memory\|^lin: unrecoverable fault\|^lin: cannot execute\|^lin: argument file not"
drive() {
    until grep -aq "wayland window" "$log" 2>/dev/null; do
        # the kernel or the program ended (crash, exit, deadlock): no window is coming
        if grep -aq "$dead" "$log" 2>/dev/null; then echo "stopped early: $(grep -a "$dead" "$log" | head -1)"; hmp "quit"; return; fi
        sleep 0.5
    done
    sleep ${SETTLE:-15}                            # let the app finish its first paints
    hmp "screendump $out/shot-before.ppm"
    for k in ${keys:-l o o s}; do hmp "sendkey $k"; done
    sleep 5
    hmp "screendump $out/shot-typed.ppm"
    hmp "quit"
}
drive & driver=$!
LIN_BOOT=${LIN_BOOT:-uefi} LIN_DISK_IF=${LIN_DISK_IF:-virtio} LOOS_BOOT_OUT=$out LOOS_KVM=${LOOS_KVM:-auto} \
QEMU_MEM=${QEMU_MEM:-$mem} QEMU_EXTRA="-monitor unix:$mon,server,nowait" TIMEOUT=${TIMEOUT:-60} \
ROOTFS_BINS="$args $files" "$here/lin_dyn_test.sh" "@$args" > "$out/build.log" 2>&1
kill $driver 2>/dev/null; wait   # QEMU is gone: stop waiting for a window that never came
grep -a "^lin: wayland window\|^lin: .*running," "$log" | tail -2
ok=1
for s in before typed; do [ -s "$out/shot-$s.ppm" ] || { echo "no screenshot $s (window never came up)"; ok=0; }; done
[ $ok = 1 ] && ! cmp -s "$out/shot-before.ppm" "$out/shot-typed.ppm" || { [ $ok = 1 ] && echo "window did not change after typing"; ok=0; }
[ $ok = 1 ] && echo "lin app test ok: $app (screenshots in $out)" || { echo "lin app test FAILED: $app"; exit 1; }
