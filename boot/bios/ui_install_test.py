#!/usr/bin/env python3
"""Interactive installer test: drives the real setup wizard with keystrokes through the QEMU monitor (no unattended
shortcut), installs onto a second disk, checks the disk with btrfs check, then boots the installed disk and signs in at the
login screen with the password typed in the wizard. Run with QEMU_ACCEL="-enable-kvm -cpu host" for speed.
usage: ui_install_test.py OUTDIR"""
import os
import shutil
import struct
import subprocess
import sys
import tempfile

here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(here, "..", "tools"))
from qemu_drive import Vm  # noqa: E402

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "..", "..", "out", "boot")
if not shutil.which("btrfs"):
    print("missing btrfs: skipping"); sys.exit(77)
work = tempfile.mkdtemp(prefix="loos-ui-install.")
img = subprocess.check_output([os.path.join(here, "build.sh"), out, "--cmdline", "installer"], text=True).strip().splitlines()[-1]
target = os.path.join(work, "target.img")
with open(target, "wb") as f:
    f.truncate(2 << 30)
PASSWORD = "horse9staple"
fails = []

def check(cond, msg):
    if not cond:
        fails.append(msg); print("FAIL:", msg)

vm = Vm(["-vga", "std", "-smp", "2", "-m", "512M", "-drive", "file=%s,format=raw,if=ide,index=0" % img,
         "-drive", "file=%s,format=raw,if=ide,index=1" % target, "-no-reboot"])
try:
    check(vm.wait("installer: 1 windows", 90), "installer UI did not start")
    vm.shot(os.path.join(work, "welcome.ppm"))
    vm.keys("tab ret")                                   # Install LoOS
    vm.keys("tab tab tab ret")                           # disk page: list, Back, Continue
    vm.keys("tab"); vm.text("ada")                       # name; the user name follows
    vm.keys("tab tab tab"); vm.text(PASSWORD)            # username, computer name, password
    vm.keys("tab"); vm.text(PASSWORD)                    # confirm
    vm.shot(os.path.join(work, "account.ppm"))
    vm.keys("tab tab ret")                               # Back, Continue
    vm.keys("tab tab tab tab ret")                       # privacy: two toggles, Back, Continue
    vm.keys("tab tab ret")                               # review: Back, Erase disk and install
    check(vm.wait("install: ok (verified)", 240), "installation did not finish")
    vm.shot(os.path.join(work, "done.ppm"))
finally:
    log1 = vm.serial(); vm.close()
check("install: media is installer" in log1, "not booted as installer")

with open(target, "rb") as t:
    def sector(n): t.seek(n * 512); return t.read(512)
    mbr = sector(0)
    check(mbr[510:512] == b"\x55\xaa" and mbr[450] == 0x83, "partition table")
    start, size = struct.unpack("<I", mbr[454:458])[0], struct.unpack("<I", mbr[458:462])[0]
    cfg = b"".join(sector(67 + i) for i in range(8)).split(b"\0")[0].decode()
    check("hostname=loos-pc" in cfg and "\naccount ada " in cfg and PASSWORD not in cfg, "stored configuration")
    t.seek(start * 512)
    with open(os.path.join(work, "data.bin"), "wb") as d:
        d.write(t.read(size * 512))
r = subprocess.run(["btrfs", "check", "--readonly", os.path.join(work, "data.bin")], capture_output=True, text=True)
check(r.returncode == 0, "btrfs check: " + r.stdout[-300:] + r.stderr[-300:])

vm = Vm(["-vga", "std", "-smp", "2", "-m", "512M", "-drive", "file=%s,format=raw,if=ide,index=0" % target, "-no-reboot"])
try:
    check(vm.wait("login: 1 windows", 90), "login screen did not appear")
    vm.shot(os.path.join(work, "login.ppm"))
    vm.text(PASSWORD)
    vm.keys("ret")
    check(vm.wait("login: signed in as ada", 90), "sign-in with the wizard password failed")
    check(vm.wait("desktop: running", 90), "desktop did not start after sign-in")
    vm.shot(os.path.join(work, "desktop.ppm"))
finally:
    vm.close()
for n in ("welcome", "account", "done", "login", "desktop"):
    src = os.path.join(work, n + ".ppm")
    if os.path.exists(src):
        shutil.copy(src, os.path.join(out, "ui_install_%s.ppm" % n))
shutil.rmtree(work, ignore_errors=True)
print("ui install", "FAILED" if fails else "ok")
sys.exit(1 if fails else 0)
