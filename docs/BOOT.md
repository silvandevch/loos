# Boot

LoOS boots from **BIOS** or **UEFI** into the same kernel image through one shared loader core.

```
 BIOS:  MBR ─ stage 2 (real mode: E820, VBE, disk → RAM) ─ frontend.c ┐
                                                                         ├─ boot/common loader core ─ kernel (higher half)
 UEFI:  BOOTX64.EFI (ESP: BOOT.CFG, kernel, modules, GOP, ACPI) ───────┘
```

## Components

| Path | Role |
|---|---|
| `boot/common/` | Firmware-independent loader core (C, freestanding, host-tested): `mmap` (memory-map normalisation), `elf` (ELF64 validation + RELATIVE relocation), `paging` (4-level tables), `loader` (`loader_prepare` / `loader_main`) |
| `boot/bios/` | `mbr.S` (stage 1), `stage2.S` (real-mode work, unreal-mode copy to high RAM, long-mode entry), `frontend.c`, `build.sh`, `boot_test.sh` |
| `boot/uefi/` | `efi.h` (UEFI interfaces, from the spec), `main.c` (BOOTX64.EFI), `build.sh` (GPT + FAT32 ESP image), `boot_test.sh` |
| `boot/tools/mkbios.py` | BIOS image assembler (boot table + files) |
| `boot/build-kernel.sh` | builds the stage-0 kernel (`src/kernel/boot/`) as a static PIE ELF |
| `src/kernel/boot/bootinfo.h` | the boot protocol (handoff block) |

## What the loader guarantees at kernel entry

* Long mode, paging on, `CR0.WP` and `EFER.NXE` set, interrupts off, `RDI` = physical address of `struct loos_bootinfo`.
* The kernel is a **static PIE ELF64** (`ET_DYN`). Any size that fits RAM (no fixed staging area). The loader validates every
  header field (untrusted input), applies `R_X86_64_RELATIVE` relocations, and maps it in the top 1 GiB of the address space
  (`0xffffffff80000000..`) at a **KASLR**-slid, 2 MiB-aligned base (`nokaslr` on the command line pins it) with **per-segment W^X**
  (4 KiB pages: text r-x, rodata r--, data rw-). Segments that are both writable and executable are rejected.
* Identity map of the first 4 GiB (RAM RW+NX cacheable; non-RAM uncached), all RAM above 4 GiB (up to 512 GiB, best effort) and
  the framebuffer. The loader's own image stays executable until the kernel reclaims it (`LOOS_MEM_BOOT`).
* A single sorted, non-overlapping **memory map** (`LOOS_MEM_*`): firmware ranges plus the loader's reservations; the most
  restrictive type wins on overlap; page 0 is reserved; unused scratch is returned as usable.
* **Modules** (initrd, boot drivers, hive, ...): up to 8 files loaded next to the kernel (type `LOOS_MEM_MODULE`), plus a
  **command line**, the **ACPI RSDP**, and a 32-bpp **framebuffer** (VBE on BIOS, GOP on UEFI; BGRX/RGBX reported).
* **SMP** is the kernel's job, as on Linux/Windows: the stage-0 kernel parses the MADT and starts the other CPUs with
  INIT-SIPI-SIPI using `crates/khal/src/ap_tramp.S` and a private trampoline page table.

## Disk formats

* **BIOS**: `LBA 0` stage 1 · `1..64` stage 2 · `65..66` boot table (`LOOSBTAB`, command line, up to 9 file entries) · files,
  sector aligned. Files are read through a 32 KiB bounce buffer and copied above 1 MiB in *unreal mode*, landing at 16 MiB.
* **UEFI**: GPT, FAT32 ESP with `\EFI\BOOT\BOOTX64.EFI`, `\LOOS\KERNEL.ELF`, modules and `\LOOS\BOOT.CFG`:
  ```
  kernel=\LOOS\KERNEL.ELF
  module=initrd:\LOOS\INITRD.BIN
  cmdline=console=ttyS0 quiet
  ```

## Resolution, framebuffer, boot screen

* The loader picks the 32-bpp linear mode closest to the requested size: BIOS reads the VBE mode list in stage 2
  (`mkbios.py --video WxH`, default 1920x1080), UEFI walks the GOP modes (`video=WxH` in `BOOT.CFG`). Tested 800x600 to 2560x1440.
* The framebuffer is mapped write-combining (PAT entry 4, set by the loader) when the CPU has a PAT, uncached otherwise.
* The kernel allocates its back buffer and app surfaces from the memory map (`kalloc.c`), so any size up to 8192x8192 works.
* The boot screen (`splash.c`) shows the wordmark and a progress bar that follows the real init steps, then cross-fades into
  the desktop.

## Installer (BIOS and UEFI media)

Boot a media image built with `--cmdline "installer"` (BIOS: `mkbios.py` adds a `bootimg` module holding the media itself;
UEFI: the loader passes `efi`, `kernel.elf` and `boot.cfg` as modules), so the install source never depends on the boot
medium. The kernel enumerates disks through the blkdev registry (`blk.c`: legacy ATA/PIO, AHCI, NVMe, found via `pci.c`) and
shows the wizard. `install.c` then installs for the firmware the media booted with:

* **BIOS**: MBR + stage2 + kernel/boot files cloned to the disk, partition entry for the data partition, boot table without
  `installer`, `LOOSCFG1` config + account hash at LBA 67..74.
* **UEFI**: GPT (`diskfmt/gpt.c`), FAT32 ESP with `EFI/BOOT/BOOTX64.EFI`, `kernel.elf` and `BOOT.CFG` (`diskfmt/fat32.c`),
  config stored on the ESP.
* **Both**: the data partition is formatted as btrfs (`src/fs/btrfs/mkfs.c`: single profile, 16 KiB nodes), the account is a
  salted PBKDF2 hash, and everything is read back and verified. The installed disk boots to the sign-in screen, then the desktop.

`installer.auto=N` (with `installer.user/pass`) installs unattended. Tests: `boot.install` (BIOS) and `boot.install_uefi`
run IDE, AHCI and NVMe targets and check the result with host `btrfs check`, `sgdisk -v`, `fsck.fat` and `mtools`;
`boot.install_ui` drives the real wizard through the QEMU monitor. All boot tests honour `QEMU_ACCEL="-enable-kvm -cpu host"`
and pass under both KVM and TCG.

## Known limits

* Storage: no USB mass storage / xHCI (cannot install to or from USB sticks), NVMe 4Kn and non-512-byte sectors unsupported,
  AHCI/NVMe are polled single-queue drivers.
* x86-64 only; BIOS needs ≥ 32 MiB RAM (boot files land at 16 MiB), scratch/CR3 must sit below 4 GiB.
* Kernel window is 1 GiB; a kernel that does not fit is rejected.
* UEFI runtime services are left at their firmware addresses (no `SetVirtualAddressMap` yet); no Secure Boot / measured boot,
  no network boot, no multiboot handoff, no compressed kernels.
* The stage-0 kernel still runs on the loader's identity map; the real VM (crates/kmem) takes over later.
