# Remote build, ISO, and QEMU boot

Turns the prepared source tree into a bootable ISO and boots it — either on a
**Windows VM over SSH** (dependable) or **locally under Wine** (no VM).

```
remote/
  loos-build.py         the "simple program": ssh → copy → compile → iso → fetch → qemu
  config.example.env    copy to config.env; holds host/user/paths
  build-in-vm.cmd       runs on the VM inside razzle (prebuild + build + oscdimg)
  build-wine.sh         build + ISO entirely local, under Wine
  make-iso.sh           Linux-only ISO fallback (mkisofs/xorriso, etfsboot.com)
  run-qemu.sh           boot an ISO in QEMU with an install disk
  win10-iot-vm-setup.md how to stand up the Windows 10 IoT LTSC VM
```

## Path A — Windows 10 IoT LTSC VM over SSH (recommended)

```bash
cp remote/config.example.env remote/config.env   # edit host/user/paths
python3 remote/loos-build.py all                 # sync → build → fetch → qemu
# or step by step:
python3 remote/loos-build.py sync
python3 remote/loos-build.py build
python3 remote/loos-build.py fetch
python3 remote/loos-build.py qemu
python3 remote/loos-build.py iso                 # re-make ISO only, no rebuild
```

What `all` does:
1. **sync** — pushes the source tree to `REMOTE_DIR` (rsync if the VM has it,
   else `tar` → `scp` → Windows `tar.exe` extract) and drops `build-in-vm.cmd`
   next to it.
2. **build** — `ssh` runs, on the VM:
   `tools\razzle.cmd free offline no_certcheck exec ...\build-in-vm.cmd`
   which does `prebuild` + `build -bcZP -M 4` + `tools\oscdimg.cmd <variant>`.
3. **fetch** — `scp` the finished ISO back to `ISO_OUT` on this host.
4. **qemu** — boots it with a fresh `loos.qcow2` install disk.

Everything is `ssh … "cmd /c …"`, so `.cmd` files run as batch — never via the
Windows shell file association.

## Path B — Wine only (no VM)

```bash
remote/build-wine.sh srv          # builds, then makes the ISO with oscdimg.exe
ISO=out/loos.iso remote/run-qemu.sh
```

Wine runs `oscdimg.exe` directly, so the ISO is identical to the VM path. Wine
gets the RAZZLE environment up (verified) — the only caveat is that a full
compile is long and some very old 16-bit tools may misbehave; if so, fall back
to the VM.

## Path C — build elsewhere, burn here

If you built on a VM and only want the ISO on Linux from a layout dir:

```bash
remote/make-iso.sh /path/to/layout/srv out/loos.iso
```

## Notes

- `VARIANT` selects the SKU CD layout under `%_NTPOSTBLD%` (`srv`, `srv_usa`,
  `wks`, `per`, `blad`, …). Default `srv`. `tools\oscdimg.cmd` uses it.
- The postbuild layout only exists **inside a razzle session** (`%_NTPOSTBLD%`),
  which is why the ISO step runs as a razzle `exec`, not a standalone command.
- `run-qemu.sh` needs `/dev/kvm` for speed; otherwise it uses TCG (slow).
- Server 2003 x64 guests want a BIOS (i440fx) machine — the script uses
  `-machine pc` and an IDE disk on purpose.
