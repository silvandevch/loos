#!/usr/bin/env bash
# Boots LoOS and starts the host's real Firefox (GTK3, Wayland, software rendering) in the Linux layer.
#   lin_firefox_test.sh [url]        STRACE=1 logs every syscall, STRACE=46,47 only those
set -u
here=$(cd "$(dirname "$0")" && pwd)
out=${LOOS_BOOT_OUT:-$here/../out/linfox}; mkdir -p "$out"; out=$(cd "$out" && pwd)
url=${1:-about:blank}
prof=$out/profile; rm -rf "$prof"; mkdir -p "$prof"   # fresh profile, shipped in the rootfs
# LIN_PREFS="name=value;name=value" become user.js prefs (values are JS literals: true, 1, "s").
# Default: vsync from a timer instead of Wayland frame callbacks. With frame-callback vsync Firefox
# composites its first frames and then stops (paint transactions never complete); still open.
IFS=';' read -ra prefs <<< "${LIN_PREFS-widget.wayland.vsync.enabled=false}"
for kv in "${prefs[@]}"; do [ -n "$kv" ] && printf 'user_pref("%s", %s);\n' "${kv%%=*}" "${kv#*=}" >> "$prof/user.js"; done
touch "$prof/user.js"
args=$out/firefox.args          # too long for the kernel command line: loos.lin=@file reads it from the rootfs
{
    case ${STRACE:-} in '') ;; 1) echo strace ;; *) echo "strace:$STRACE" ;; esac   # STRACE=1 all, STRACE=46,47 only those
    # No seccomp/namespaces in the Linux layer yet: Firefox's child processes run unsandboxed.
    printf '%s\n' MOZ_DISABLE_CONTENT_SANDBOX=1 MOZ_DISABLE_RDD_SANDBOX=1 MOZ_DISABLE_SOCKET_PROCESS_SANDBOX=1 \
        MOZ_DISABLE_GMP_SANDBOX=1 MOZ_DISABLE_UTILITY_SANDBOX=1 MOZ_CRASHREPORTER_DISABLE=1 ${LIN_ENV:-} \
        /usr/lib64/firefox/firefox --no-remote --profile "$prof" "$url"
} > "$args"
# virtio: the BIOS loader reads the ~450 MB rootfs far faster than through emulated IDE PIO
LIN_BOOT=${LIN_BOOT:-uefi} LIN_DISK_IF=${LIN_DISK_IF:-virtio} LOOS_BOOT_OUT=$out LOOS_KVM=${LOOS_KVM:-auto} TIMEOUT=${TIMEOUT:-600} \
ROOTFS_BINS="$args $prof $prof/user.js /usr/lib64/firefox /usr/lib64/firefox/firefox /etc/fonts /usr/share/fonts/adwaita-sans-fonts /usr/share/fonts/google-noto-vf /usr/share/glib-2.0/schemas /usr/share/icons/Adwaita /usr/lib64/gdk-pixbuf-2.0 /usr/libexec/glycin-loaders /usr/share/glycin-loaders /usr/share/mime /usr/bin/bash /usr/share/xkeyboard-config-2 /usr/lib64/libgtk-3.so.0" \
    "$here/lin_dyn_test.sh" "@$args"
