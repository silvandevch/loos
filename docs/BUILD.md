# LoOS — building

Two ways to build. Start with **Wine** (no VM). Use the **Windows VM** if Wine
misbehaves on an old tool. Both end with a bootable ISO.

One command checks readiness first:

```bash
./loos verify      # tools + paths + wine prefix
./loos status      # where things live + disk
```

---

## A. Build under Wine (no VM)

```bash
./loos setup       # once: fetch source + patchkit, verify layout
./loos prep        # once: overlay patches, OOBE fix, certs, tags, missing bins
./loos build       # prebuild + build (long — hours)
./loos iso         # make the bootable ISO with the tree's oscdimg
./loos qemu        # boot it
```

Or the original scripts directly:
`remote/build-wine.sh` (build+ISO in one), `scripts/09-build-all.sh` (build only).

**What happens:** razzle starts, `prebuild.cmd` generates makefiles, then
`build -bcZP -M 4` compiles. Wine reports `PROCESSOR_ARCHITECTURE=AMD64`, so
razzle builds **x64 natively** (single pass). Threads are capped at 4 because the
tree is unstable above that. A full build is hours.

**First-time Wine setup** (if `./loos verify` complains):
```bash
./loos fixwine      # make .cmd/.bat run via cmd.exe, not notepad
```

---

## B. Build on a Windows VM (dependable)

Do this if Wine fails on an old tool, or you want the "real" toolchain.

1. Stand up the VM — follow **`remote/win10-iot-vm-setup.md`** (Windows 10 IoT
   LTSC + OpenSSH; XP/2000 fallback included).
2. Configure and run:
   ```bash
   cp remote/config.example.env remote/config.env   # edit host/user
   ./loos vm-build          # sync → build → fetch ISO → qemu
   # or step by step:
   ./loos vm sync
   ./loos vm build
   ./loos vm iso            # re-make ISO only, no rebuild
   ./loos vm fetch          # pull ISO back to OUT_ROOT
   ./loos vm qemu
   ```

`loos-build.py` (behind `loos vm`) pushes the tree over SSH, runs
`build-in-vm.cmd` inside razzle on the guest (prebuild + build + oscdimg), and
scps the ISO back. Everything is `ssh … "cmd /c …"`, so `.cmd` runs as batch.

---

## Prerequisites

| Need | For | Check |
|---|---|---|
| `wine` (staging ok) | Wine build | `./loos verify` |
| `git rsync openssl perl python3 7z` | prep + transport | `./loos verify` |
| `mkisofs`/`xorriso` | Linux ISO fallback | `command -v mkisofs` |
| `qemu-system-x86_64`, `/dev/kvm` | boot the ISO | `ls /dev/kvm` |
| Windows VM + OpenSSH | VM build | `remote/win10-iot-vm-setup.md` |

## Missing binaries

The tree cannot build a few files (activation/crypto + third-party were
stripped). `./loos prep` parses `tools/missing.cmd` and either pulls them from
retail media or writes placeholders:

```bash
RETAIL_ISO=/path/to/srv2003_rtm.iso scripts/06-stubs.sh    # preferred
```

Placeholders produce a non-bootable image — fine for compile testing.

## ISO without a full build

If you already have a postbuild layout directory on Linux:

```bash
remote/make-iso.sh <layout-dir> "$OUT_ROOT/loos.iso"
```

## Variants

`VARIANT` picks the SKU layout `tools\oscdimg.cmd` uses (`srv`, `srv_usa`,
`wks`, `per`, `blad`). Default `srv`. Pass it where the tool accepts it, e.g.
`VARIANT=wks ./loos iso`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `.cmd` opens in notepad | `./loos fixwine` |
| `razzle … Unknown argument: x86` | don't pass `x86`; razzle has no x86 arg (see BUILD notes) |
| Wine reports wrong arch | it should say `AMD64`; `./loos verify` |
| ISO missing after build | postbuild layout incomplete; check the variant / build log in `logs/` |
| Out of space | outputs are on DATA; check `./loos status` |
