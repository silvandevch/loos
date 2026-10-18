#!/usr/bin/env python3
"""
loos-build.py — build NT 5.2 x64 on a remote Windows VM over SSH, then pull the
bootable ISO back to this host and (optionally) boot it in QEMU.

Why a VM: RAZZLE is native Windows PE tooling.  Wine gets the environment up
(verified) but a real Windows 10 IoT LTSC VM is the dependable build host, and
an XP/2000 VM is the fallback for the oldest tools.

Subcommands
  sync     push source tree + scripts to the VM
  build    run prebuild + build + oscdimg on the VM (inside razzle)
  iso      re-make only the ISO from an existing build (no rebuild)
  fetch    copy the ISO from the VM back to this host
  qemu     boot the fetched ISO in QEMU (attaches a data disk for install)
  all      sync -> build -> fetch -> qemu

Config: remote/config.env (see config.example.env).  Any value can be
overridden with an environment variable of the same name.

No third-party Python packages required.
"""
from __future__ import annotations
import argparse, os, shlex, shutil, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def load_config() -> dict:
    cfg: dict[str, str] = {}
    for name in ("config.env", "config.example.env"):
        p = HERE / name
        if p.is_file():
            for line in p.read_text().splitlines():
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                cfg[k.strip()] = v.strip().strip("'").strip('"')
            break
    # environment overrides
    for k in list(cfg) + ["REMOTE_HOST", "REMOTE_USER", "REMOTE_PORT", "REMOTE_DIR",
                          "REMOTE_ISO_DIR", "VARIANT", "SSH_KEY", "SSH_OPTS",
                          "ISO_OUT", "LOCAL_SRC", "QEMU_MEM", "QEMU_CPUS",
                          "QEMU_DISK", "QEMU_DISK_SIZE", "QEMU_ACCEL"]:
        if k in os.environ:
            cfg[k] = os.environ[k]
    return cfg


class Ctx:
    def __init__(self, cfg: dict):
        self.c = cfg
        self.host = cfg.get("REMOTE_HOST", "")
        self.user = cfg.get("REMOTE_USER", "")
        self.port = cfg.get("REMOTE_PORT", "22")
        self.remote_dir = cfg.get("REMOTE_DIR", r"C:\loos\srv03rtm")
        self.remote_iso_dir = cfg.get("REMOTE_ISO_DIR", r"C:\loos\out")
        self.variant = cfg.get("VARIANT", "srv")
        self.iso_out = Path(cfg.get("ISO_OUT", str(ROOT / "out" / "loos.iso")))
        self.local_src = Path(cfg.get("LOCAL_SRC", "/home/silvan/LoOS-build/srv03rtm"))
        self.key = cfg.get("SSH_KEY", "")
        self.opts = shlex.split(cfg.get("SSH_OPTS", ""))
        self.dest = f"{self.user}@{self.host}" if self.user else self.host

    # ---- ssh plumbing -------------------------------------------------------
    def _base(self, prog: str) -> list[str]:
        cmd = [prog]
        if self.key:
            cmd += ["-i", self.key]
        if prog == "ssh":
            cmd += ["-p", self.port]
        elif prog == "scp":
            cmd += ["-P", self.port]
        cmd += self.opts
        return cmd

    def ssh(self, win_cmd: str, check=True, capture=False):
        # Run through cmd.exe so .cmd executes (never the shell file assoc).
        full = self._base("ssh") + [self.dest, f'cmd /c "{win_cmd}"']
        return self._run(full, check, capture)

    def scp_to(self, local: Path, remote: str):
        return self._run(self._base("scp") + [str(local), f"{self.dest}:{remote}"])

    def scp_from(self, remote: str, local: Path):
        return self._run(self._base("scp") + [f"{self.dest}:{remote}", str(local)])

    def _run(self, cmd, check, capture):
        print("  $", " ".join(shlex.quote(x) for x in cmd))
        r = subprocess.run(cmd, text=True, capture_output=capture)
        if check and r.returncode != 0:
            sys.exit(f"[loos-build] command failed ({r.returncode}): {' '.join(cmd)}")
        return r


# ---- commands --------------------------------------------------------------
def cmd_sync(ctx: Ctx, args):
    ctx.ssh(f'mkdir "{ctx.remote_dir}" 2>nul & mkdir "{ctx.remote_iso_dir}" 2>nul', check=False)
    # rsync if the VM has it (Linux VM / cwrsync), else tar|scp|untar.
    if shutil.which("rsync") and args.transport == "rsync":
        print("[sync] rsync ->", ctx.dest)
        ctx._run(ctx._base("rsync") + ["-az", "--delete", "-e",
                 " ".join(["ssh"] + (["-i", ctx.key] if ctx.key else []) +
                          ["-p", ctx.port] + ctx.opts),
                 str(ctx.local_src) + "/", f"{ctx.dest}:{ctx.remote_dir.replace(chr(92), '/')}/"])
    else:
        print("[sync] tar -> scp -> untar (Windows tar.exe)")
        with tempfile.NamedTemporaryFile(suffix=".tgz", delete=False) as tf:
            tgz = Path(tf.name)
        try:
            ctx._run(["tar", "czf", str(tgz), "-C", str(ctx.local_src), "."])
            remote_tgz = r"C:\loos\src.tgz"
            ctx.ssh(r'mkdir "C:\loos" 2>nul', check=False)
            ctx.scp_to(tgz, remote_tgz)
            ctx.ssh(f'cd /d "{ctx.remote_dir}" && tar -xzf "{remote_tgz}" && del "{remote_tgz}"')
        finally:
            tgz.unlink(missing_ok=True)
    # push our scripts (build-in-vm.cmd etc.)
    for f in (HERE / "build-in-vm.cmd",):
        ctx.scp_to(f, rf"{ctx.remote_dir}\..\scripts\{f.name}")
    print("[sync] done")


def _remote_build_cmd(ctx: Ctx, iso_only: bool) -> str:
    # build-in-vm.cmd is copied next to the tree (C:\loos\scripts).
    scripts = str(Path(ctx.remote_dir).parent) + r"\scripts"
    flag = " iso" if iso_only else ""
    return (f'set VARIANT={ctx.variant}&& set ISO_OUT={ctx.remote_iso_dir}\\loos.iso&& '
            f'call "{ctx.remote_dir}\\tools\\razzle.cmd" free offline no_certcheck '
            f'exec "{scripts}\\build-in-vm.cmd"{flag}')


def cmd_build(ctx: Ctx, args):
    print(f"[build] VM build (variant={ctx.variant})")
    ctx.ssh(_remote_build_cmd(ctx, iso_only=False))


def cmd_iso(ctx: Ctx, args):
    print("[iso] re-making ISO on VM (no rebuild)")
    ctx.ssh(_remote_build_cmd(ctx, iso_only=True))


def cmd_fetch(ctx: Ctx, args):
    ctx.iso_out.parent.mkdir(parents=True, exist_ok=True)
    remote = ctx.remote_iso_dir + r"\loos.iso"
    print(f"[fetch] {ctx.dest}:{remote} -> {ctx.iso_out}")
    ctx.scp_from(remote, ctx.iso_out)
    print(f"[fetch] {ctx.iso_out} ({ctx.iso_out.stat().st_size/2**20:.1f} MiB)")


def cmd_qemu(ctx: Ctx, args):
    script = HERE / "run-qemu.sh"
    env = os.environ.copy()
    env["ISO"] = str(ctx.iso_out)
    env["QEMU_MEM"] = ctx.c.get("QEMU_MEM", "4096")
    env["QEMU_CPUS"] = ctx.c.get("QEMU_CPUS", "4")
    env["QEMU_DISK"] = ctx.c.get("QEMU_DISK", "")
    env["QEMU_DISK_SIZE"] = ctx.c.get("QEMU_DISK_SIZE", "20G")
    env["QEMU_ACCEL"] = ctx.c.get("QEMU_ACCEL", "kvm")
    print("[qemu]", script)
    sys.exit(subprocess.run(["bash", str(script)], env=env).returncode)


def cmd_all(ctx: Ctx, args):
    cmd_sync(ctx, args)
    cmd_build(ctx, args)
    cmd_fetch(ctx, args)
    cmd_qemu(ctx, args)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("sync"); p.add_argument("--transport", choices=["rsync", "tar"], default="tar")
    sub.add_parser("build")
    sub.add_parser("iso")
    sub.add_parser("fetch")
    sub.add_parser("qemu")
    sub.add_parser("all"); 
    args = ap.parse_args(argv)
    ctx = Ctx(load_config())
    if not ctx.host and args.cmd in ("sync", "build", "iso", "fetch", "all"):
        sys.exit("no REMOTE_HOST set — edit remote/config.env")
    {"sync": cmd_sync, "build": cmd_build, "iso": cmd_iso,
     "fetch": cmd_fetch, "qemu": cmd_qemu, "all": cmd_all}[args.cmd](ctx, args)


if __name__ == "__main__":
    main()
