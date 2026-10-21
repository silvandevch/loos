#!/usr/bin/env python3
"""Scriptable QEMU session for end-to-end UI tests: boots a VM with a QMP-less HMP monitor, sends keystrokes, waits for text
on the serial console, takes screenshots.

usage in tests:  from qemu_drive import Vm; vm = Vm([...qemu args...]); vm.wait("text", 60); vm.keys("tab ret"); vm.shot("x.ppm")"""
import os
import socket
import subprocess
import tempfile
import time


class Vm:
    def __init__(self, args, accel=None):
        self.dir = tempfile.mkdtemp(prefix="loos-vm.")
        self.log = os.path.join(self.dir, "serial.log")
        self.sock = os.path.join(self.dir, "mon.sock")
        cmd = ["qemu-system-x86_64", "-display", "none", "-serial", "file:" + self.log, "-monitor", "unix:%s,server,nowait" % self.sock]
        cmd += (accel or os.environ.get("QEMU_ACCEL", "")).split() + args
        self.p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        for _ in range(100):
            if os.path.exists(self.sock):
                break
            time.sleep(0.1)
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(self.sock)
        time.sleep(0.3)
        self.s.recv(65536)

    def hmp(self, line, delay=0.05):
        self.s.send((line + "\n").encode())
        time.sleep(delay)
        self.s.settimeout(0.2)
        try:
            self.s.recv(65536)
        except socket.timeout:
            pass

    def keys(self, seq, gap=0.3):
        for k in seq.split():
            self.hmp("sendkey " + k)
            time.sleep(gap)

    def text(self, s, gap=0.15):
        names = {" ": "spc", "-": "minus", ".": "dot"}
        for ch in s:
            self.hmp("sendkey " + names.get(ch, ch))
            time.sleep(gap)

    def serial(self):
        try:
            return open(self.log, errors="replace").read()
        except FileNotFoundError:
            return ""

    def wait(self, needle, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if needle in self.serial():
                return True
            if self.p.poll() is not None:
                return needle in self.serial()
            time.sleep(0.25)
        return False

    def shot(self, path):
        self.hmp("screendump " + path, delay=1.0)

    def close(self):
        try:
            self.p.kill()
        except Exception:
            pass
        self.p.wait()
