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

## Try it: QEMU and VMware (Windows 11 Pro host)

Build the UEFI install ISO on the Linux box (`./loos install-iso`, or `boot/iso/build.sh`) → `out/boot/loos.iso`, and copy it to
the Windows machine. `boot/iso/iso_test.sh` checks the full flow (CD boot → install → boot the installed disk) in QEMU+OVMF.
Quick QEMU run: `boot/run-desktop.sh uefi`.

VMware Workstation (Pro/Player) on Windows 11:

1. New Virtual Machine → Custom → *Other* / *Other 64-bit* (or Other Linux 6.x 64-bit) → firmware **UEFI**.
2. **Disable Secure Boot**: VM Settings → Options → Advanced → untick *Enable secure boot* (or Esc/F2 into the VM firmware →
   Device Manager → Secure Boot Configuration). `BOOTX64.EFI` is unsigned. No TPM/encryption is needed.
3. Memory ≥ 2 GB, 2+ CPUs, disk **≥ 4 GB** (installer minimum is 2 GiB) as **NVMe** or **SATA**. Do not use SCSI/LSI (no driver).
   Leave the keyboard/mouse on the default (PS/2); there is no USB HID or xHCI driver yet.
4. CD/DVD → *Use ISO image file* → `loos.iso`, *Connect at power on*. Power on; the wizard starts. Pick the disk, create an
   account, install.
5. When it finishes, power off, then set the CD/DVD to *Use physical drive* or untick *Connected* (otherwise the firmware boots the
   ISO again), and power on: sign in with your account.

If the screen stays black, add `video=1280x720` to the `cmdline=` line in `\LOOS\BOOT.CFG` of the ISO's `efiboot.img` or tell
me what the VM's serial/debug output says (VM Settings → Add → Serial Port → output to file; the kernel logs to COM1).

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
