#!/usr/bin/env bash
# Keyboard proof for the Linux layer: zenity's text-entry dialog (GTK4 over the LoOS Wayland compositor)
# gets "loos" typed on the QEMU PS/2 keyboard plus Enter; zenity then prints the text and exits 0.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${LOOS_BOOT_OUT:-$root/out/linkbd}; mkdir -p "$out"
sockdir=$(mktemp -d); trap 'rm -rf "$sockdir"' EXIT   # the data drive cannot hold Unix sockets
mon=$sockdir/monitor.sock; log=$out/lin-dyn.log; rm -f "$log"
hmp() { echo "$1" | socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1; sleep 0.2; }
type_text() {
    for _ in $(seq 1 600); do grep -aq "wayland window" "$log" 2>/dev/null && break; sleep 0.5; done
    sleep 4
    for k in l o o s ret; do hmp "sendkey $k"; done
}
type_text &
LIN_BOOT=${LIN_BOOT:-uefi} LIN_DISK_IF=${LIN_DISK_IF:-virtio} LOOS_KVM=${LOOS_KVM:-auto} LOOS_BOOT_OUT=$out \
QEMU_EXTRA="-monitor unix:$mon,server,nowait" TIMEOUT=${TIMEOUT:-120} \
ROOTFS_BINS="/usr/bin/zenity /etc/fonts /usr/share/fonts/adwaita-sans-fonts /usr/share/glib-2.0/schemas /usr/share/icons/Adwaita /usr/lib64/gdk-pixbuf-2.0 /usr/share/xkeyboard-config-2" \
    "$here/lin_dyn_test.sh" "/usr/bin/zenity,--entry,--text=Type" >/dev/null
st=$?; wait
if grep -aq "^loos" "$log" && [ $st = 0 ]; then echo "lin keyboard test ok"; else echo "lin keyboard test FAILED"; grep -a "^lin:" "$log" | tail -5; exit 1; fi
