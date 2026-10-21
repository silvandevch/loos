# LoOS — installing the OS

How a machine gets LoOS onto it: the boot path, the installer, deployment,
recovery, and the media builder. The bootloader itself is defined by the `boot`
tasks; this doc is the user-facing install story.

## Boot path

- **UEFI**: LoOS ships an EFI application (`boot`) on the ESP; the firmware
  launches it directly. Optional **Secure Boot** signing chain (`codesign`).
- **Legacy BIOS**: MBR → stage-2 (`bootlegacy`) with El Torito for
  CD/DVD boot, plus a boot menu and a **safe mode** (`bugcheck`).
- **Dual-boot**: `instdual` manages a menu for LoOS + Windows + Linux and never
  silently seizes the existing boot order.

## Installer (`instcore`)

A WinPE-style pre-installation environment boots from USB/DVD and runs the
installer:

1. pick the target disk and a free region,
2. create/format **btrfs** — the **only** filesystem LoOS offers ([BTRFS.md](BTRFS.md));
   the installer creates the standard subvolumes (`@`, `@home`, snapshot subvol)
   plus a FAT32 ESP. No NTFS/ext4/exFAT choice is presented,
3. deploy the OS image,
4. install the bootloader (UEFI or legacy; dual-boot aware),
5. first-boot: create a local account, stamp **machine identity** (hostname, OEM/
   product fields, splash — see [BRANDING.md](BRANDING.md); app branding is
   already in the sources), enable the privacy defaults.

Supporting pieces: `instunattend` (answer files / OEM provisioning),
`instmedia` (build the USB/DVD), `instdrv` (offline driver injection),
`instupgrade` (upgrade-in-place + migration from an earlier LoOS install).

## Recovery (`winre`, `factoryreset`)

A separate recovery partition holds the LoOS Recovery Environment: repair boot,
roll back a bad update (`updateab`), restore from a btrfs snapshot
(`btrfssnap`), or refresh/factory-reset the machine.

## Updates (`updatesvc`)

Post-install, LoOS updates itself over the air: signed, delta, and
**atomic/rollback** (snapshot or A/B) so a failed update reverts cleanly — no
telemetry in the update channel. The `LoOS Update` app (`updateui`) is the
Windows-Update-style front end.

## Reproducible media

`reproiso` makes the install ISO byte-for-byte reproducible and signed, so a
build can be independently verified. CI builds and boots the ISO in QEMU
(`ci`, `tboot`) on every push.

## Disk layout (default)

| Partition | FS | Content |
|---|---|---|
| ESP | FAT32 | UEFI loader + secure-boot chain |
| Recovery | btrfs | Recovery Environment + snapshots |
| Root | btrfs subvol `@` | OS + packages |
| Home | btrfs subvol `@home` | user data |
| Swap | — | paging file / swap (optional, `mmpagefile`) |

## Tasks

`boot`, `bootlegacy`, `codesign`, `instcore`, `instunattend`, `instmedia`,
`instupgrade`, `instdrv`, `instdual`, `winre`, `factoryreset`, `updatesvc`,
`updateab`, `updatestage`, `updatefw`, `updateui`, `reproiso`.
