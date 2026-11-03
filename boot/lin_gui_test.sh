#!/usr/bin/env bash
# Build and boot a real Linux GUI app through liblooswin/Win32k, then save a QEMU screenshot.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_GUI_OUT:-/tmp/opencode/loos-lin-gui}}
png=${2:-/tmp/opencode/linux-gui-proof.png}
out_parent=$(dirname "$out")
if [ -d "$out_parent" ] && [ "$(stat -f -c %T "$out_parent" 2>/dev/null || true)" = exfat ]; then
    out=/tmp/opencode/loos-lin-gui-$$
fi
png_parent=$(dirname "$png")
if [ -d "$png_parent" ] && [ "$(stat -f -c %T "$png_parent" 2>/dev/null || true)" = exfat ]; then
    png=/tmp/opencode/linux-gui-proof-$$.png
fi
mkdir -p "$out" "$(dirname "$png")"
# Unix-domain sockets are not supported on the exFAT workspace mount. Keep
# QEMU's control socket on the local Linux filesystem, while leaving build
# outputs and the screenshot in the requested workspace paths.
runtime=${LOOS_QEMU_RUNTIME:-/tmp/opencode}
mkdir -p "$runtime"
if [ "$(stat -f -c %T "$runtime" 2>/dev/null || true)" = exfat ]; then
    runtime=/tmp/opencode
    mkdir -p "$runtime"
fi
sock=$runtime/loos-lin-gui-qmp-$$.sock
ppm=$runtime/loos-lin-gui-$$.ppm
log=$out/lin-gui.log
q=''
cleanup() {
    if [ -n "$q" ]; then kill "$q" 2>/dev/null || true; wait "$q" 2>/dev/null || true; fi
    rm -f "$sock" "$ppm"
}
trap cleanup EXIT INT TERM
clang --target=x86_64-linux-gnu -static -nostdlib -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    -I"$root/src/lin" "$root/tests/lin/gui.c" "$root/tests/lin/looswin_client.c" -o "$out/gui"
python3 - "$out/gui" "$out/lin-gui-initrd.tar" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[2], "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("bin", "etc"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    data = open(sys.argv[1], "rb").read()
    i = tarfile.TarInfo("bin/gui"); i.size = len(data); i.mode = 0o755; t.addfile(i, io.BytesIO(data))
PY
img=$("$here/bios/build.sh" "$out" --module initrd="$out/lin-gui-initrd.tar" --cmdline "console=ttyS0 loos.test=1 loos.lin=/bin/gui loos.lin.stop=1")
qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 512M \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -monitor "unix:$sock,server,nowait" -no-reboot &
q=$!
for _ in $(seq 1 180); do
    if grep -q "lin: gui proof complete\|fault\|deadlock" "$log" 2>/dev/null; then break; fi
    sleep 1
done
grep -q "LIN_GUI: window painted" "$log"
grep -q "LIN_GUI: quit observed" "$log"
grep -Eq "lin: gui composed green pixels [1-9][0-9]*" "$log"
for _ in $(seq 1 50); do test -S "$sock" && break; sleep 0.1; done
test -S "$sock"
python3 - "$sock" "$ppm" <<'PY'
import socket
import sys
import time

sock_path, ppm = sys.argv[1:]
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
    s.connect(sock_path)
    s.sendall(f"screendump {ppm}\n".encode())
    s.settimeout(2)
    try:
        while s.recv(4096):
            pass
    except (TimeoutError, ConnectionResetError):
        pass
time.sleep(0.5)
PY
test -s "$ppm"
kill "$q" 2>/dev/null || true; wait "$q" 2>/dev/null || true
python3 - "$ppm" "$png" <<'PY'
from PIL import Image
import sys
image = Image.open(sys.argv[1]).convert("RGB")
green = sum(1 for r, g, b in image.getdata() if (r, g, b) == (0, 200, 0))
if green == 0:
    raise SystemExit("screenshot proof failed: no rendered green Linux GUI pixels")
image.save(sys.argv[2])
print(f"screenshot green pixels: {green}")
PY
grep -E "^(LIN_GUI|lin: gui composed|lin: program finished)" "$log"
echo "screenshot: $png"
