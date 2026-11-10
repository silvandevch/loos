#!/usr/bin/env bash
# GUI input proof for the Linux layer: boots LoOS with zenity (GTK4 over the LoOS Wayland compositor), moves the
# QEMU PS/2 mouse onto the dialog's OK button and clicks. Passes when zenity exits 0, i.e. the click travelled
# PS/2 -> window manager -> wl_pointer -> GTK.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${LOOS_BOOT_OUT:-$root/out/lingui}; mkdir -p "$out"
sockdir=$(mktemp -d); trap 'rm -rf "$sockdir"' EXIT   # the data drive cannot hold Unix sockets
mon=$sockdir/monitor.sock; log=$out/lin-dyn.log; rm -f "$log"
hmp() { echo "$1" | socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1; sleep 0.15; }
click() {
    for _ in $(seq 1 600); do grep -aq "wayland window" "$log" 2>/dev/null && break; sleep 0.5; done
    sleep 3
    read -r cx cy < <(grep -a "wayland window" "$log" | head -1 | sed -E 's/.*client at ([0-9]+),([0-9]+).*/\1 \2/')
    local tx=$((cx + 150)) ty=$((cy + 177)) x=0 y=0 dx dy
    for _ in $(seq 1 30); do hmp "mouse_move -100 -100"; done     # pin the cursor to 0,0
    while [ $x -lt $tx ] || [ $y -lt $ty ]; do
        dx=$(( tx - x > 50 ? 50 : tx - x )); dy=$(( ty - y > 50 ? 50 : ty - y ))
        hmp "mouse_move $dx $dy"; x=$((x + dx)); y=$((y + dy))
    done
    hmp "mouse_button 1"; hmp "mouse_button 0"
}
click &
LOOS_BOOT_OUT=$out QEMU_EXTRA="-monitor unix:$mon,server,nowait" TIMEOUT=${TIMEOUT:-200} \
ROOTFS_BINS="/usr/bin/zenity /etc/fonts /usr/share/fonts/adwaita-sans-fonts /usr/share/glib-2.0/schemas /usr/share/icons/Adwaita /usr/lib64/gdk-pixbuf-2.0 /usr/share/xkeyboard-config-2" \
    "$here/lin_dyn_test.sh" "${LIN_ENV:-}/usr/bin/zenity,--info,--text=Click%20OK"
st=$?; wait
exit $st
