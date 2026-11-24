#!/usr/bin/env bash
# Boots LoOS under QEMU with one Windows GUI program from a tar module and saves a screenshot.
# usage: gui_test.sh <exe> <out.png> [keys]    (e.g. apps/loos-notepad/notepad.exe shots/notepad.png Hello_LoOS)
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
exe=$(realpath "$1"); png=$(realpath -m "$2"); keys=${3:-}
# EXTRA="host/file=Windows/System32/name host2=Users/x.txt": more files for the guest volume; GUI_OPEN/GUI_SAVE: answers of the file dialogs
out=${OUT:-${LOOS_GUI_OUT:-/tmp/opencode/loos-gui-$$}}; mkdir -p "$out" "$(dirname "$png")"
name=$(basename "$exe")
tarf=$out/guiroot.tar
python3 - "$exe" "$tarf" "$name" "${EXTRA:-}" <<'PY'
import sys, tarfile
with tarfile.open(sys.argv[2], "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("Windows", "Windows/System32"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    t.add(sys.argv[1], arcname=f"Windows/System32/{sys.argv[3]}")
    for e in sys.argv[4].split():
        src, dst = e.split("=", 1)
        t.add(src, arcname=dst)
PY
img=$("$here/bios/build.sh" "$out" --module winroot="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.gui=$name loos.net=1 loos.gui.keys=$keys loos.gui.open=${GUI_OPEN:-} loos.gui.save=${GUI_SAVE:-}") || exit 1
log=$out/gui.log; sd=$(mktemp -d); sock=$sd/qmp.sock
qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-auto} "$root/scripts/qemu-accel.sh" args)} -display ${QEMU_DISPLAY:-gtk} -vga std -smp 1 -m ${QEMU_MEM:-512M} \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -monitor "unix:$sock,server,nowait" -no-reboot &
q=$!
# GUI_WAIT seconds for the program to settle (default 240). On a timeout the guest's instruction pointer is sampled
# through the monitor ("hang: RIP=..." lines) so a spin can be located.
done=0
for _ in $(seq 1 "${GUI_WAIT:-240}"); do grep -q "final frame presented\|exited\|FAILED\|fault" "$log" 2>/dev/null && { done=1; break; }; sleep 1; done
if [ $done = 0 ]; then
    python3 -c "
import re,socket,time
for _ in range(4):
    s=socket.socket(socket.AF_UNIX); s.connect('$sock'); time.sleep(.2); s.recv(65536)
    s.send(b'info registers\n'); time.sleep(.5); r=s.recv(65536).decode(errors='replace'); s.close()
    m=re.search(r'RIP=([0-9a-f]+)', r); n=re.search(r'RSP=([0-9a-f]+)', r)
    print('hang: RIP=%s RSP=%s' % (m and m.group(1), n and n.group(1))); time.sleep(.7)" | tee -a "$log"
fi
sleep 1
printf 'screendump %s.ppm\n' "$png" | python3 -c "
import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.connect('$sock'); time.sleep(.3); s.send(sys.stdin.read().encode()); time.sleep(1.5)"
kill $q 2>/dev/null; wait $q 2>/dev/null
grep -E "^(win|\[winrt\]|hang)" "$log"
python3 -c "
from PIL import Image; Image.open('$png.ppm').save('$png')" && rm -f "$png.ppm" && echo "screenshot: $png"
