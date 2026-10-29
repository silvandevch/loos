#!/usr/bin/env python3
"""End-to-end QEMU browser test: boot the desktop, open LoOS Browser from Start, fetch
https://example.com through the COM2 web bridge, and save a screenshot.

Needs qemu-system-x86_64 and host network. Returns 77 (CTest skip) when the bridge cannot
reach the network or QEMU is unavailable. Run: python3 tests/boot/browser_qemu.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "boot", "tools"))
from qemu_drive import Vm  # noqa: E402


def main() -> int:
    url = sys.argv[1] if len(sys.argv) > 1 else "example.com"
    if not shutil.which("qemu-system-x86_64"):
        print("qemu-system-x86_64 not found: skipping")
        return 77
    out = os.path.join(ROOT, "out", "boot")
    os.makedirs(out, exist_ok=True)
    img = subprocess.check_output(
        [os.path.join(ROOT, "boot", "bios", "build.sh"), out, "--cmdline", "console=ttyS0"],
        text=True,
    ).strip().splitlines()[-1]

    work = tempfile.mkdtemp(prefix="loos-browser.")
    sock = os.path.join(work, "web.sock")
    blog = os.path.join(work, "bridge.log")
    helper = subprocess.Popen(
        [sys.executable, os.path.join(ROOT, "boot", "web-bridge.py"), sock],
        stdout=open(blog, "w"), stderr=subprocess.STDOUT,
    )
    for _ in range(100):
        if os.path.exists(sock):
            break
        time.sleep(0.05)

    accel = ""
    if os.path.exists("/dev/kvm") and os.environ.get("LOOS_KVM", "auto") != "off":
        accel = "-enable-kvm -cpu host"
    vm = Vm(
        [
            "-vga", "std", "-smp", "2", "-m", "512M", "-snapshot",
            "-drive", "file=%s,format=raw,if=ide,index=0" % img,
            "-chardev", "socket,id=web,path=%s,server=off" % sock,
            "-device", "isa-serial,chardev=web,index=1",
            "-no-reboot",
        ],
        accel=accel,
    )
    try:
        if not vm.wait("desktop: running", 90):
            print("desktop did not start")
            return 1
        vm.keys("meta_l", gap=0.7)
        vm.text("browser", gap=0.15)
        vm.keys("ret", gap=0.7)
        time.sleep(1.5)
        vm.keys("ctrl-l", gap=0.4)
        vm.text(url, gap=0.1)
        vm.keys("ret", gap=0.4)

        deadline = time.time() + 40
        ok = False
        while time.time() < deadline:
            log = open(blog, errors="replace").read()
            if any("web-bridge:" in line and "failed" not in line and " -> " in line and url in line for line in log.splitlines()):
                ok = True
                break
            time.sleep(0.5)
        if ok:
            # The guest is still receiving/parsing over the serial bridge; give it time to paint.
            time.sleep(30)
        shot = os.path.join(out, "browser_qemu.ppm")
        vm.shot(shot)
        if not ok:
            print("web bridge did not return " + url)
            print(log[-800:])
            return 77
        if not os.path.exists(shot) or os.path.getsize(shot) < 1000:
            print("screenshot was not written")
            return 1
        print("browser qemu ok: " + url + " via web bridge -> " + shot)
        return 0
    finally:
        vm.close()
        helper.terminate()
        helper.wait()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
