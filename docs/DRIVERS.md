# LoOS — drivers and graphics

How LoOS gets hardware working: the driver model, the **default drivers** it
ships, the **default graphics driver**, and how **vendor GPU drivers** fit in —
including the two AMD sources (a Windows WDDM driver and the Linux `amdgpu`
driver). This is the plan the `drivers/` tasks implement (see §Tasks).

## Driver model

LoOS uses an NT-style model, one device class per directory under
`src/drivers/`:

- **Kernel-mode driver framework** (`kmdf`): load/unload, dispatch tables,
  `DriverEntry`, IRP/IOCTL handling.
- **I/O manager** (`iomgr`) owns the device stack and IRPs; **PnP** (`pnp`)
  enumerates devices (ACPI + PCI + USB) and matches them to drivers via their
  hardware IDs.
- **Device classes**: a driver registers a class interface (display, storage,
  net, input, audio) that the rest of the OS binds to. The display class is the
  one the compositor talks to.
- Drivers are built freestanding and signed (`codesign`); unsigned kernel
  modules are refused at load.

## The default drivers LoOS ships

The minimum set to boot to a usable desktop on QEMU **and** common x86-64
hardware. Everything below is a `drivers/` task.

| Class | Driver | Task |
|---|---|---|
| Bus | ACPI, PCI(+MSI), virtio bus | `acpi`, `pci`, `virtio` |
| Interrupt/timer | local APIC, IOAPIC, HPET | `hal` |
| Storage | AHCI + NVMe, partition manager (MBR/GPT) | `stornvme`, `partmgr` |
| USB | xHCI + HID (kbd/mouse) + mass-storage | `usb`, `hiddrv` |
| Display | **generic framebuffer** (default) + virtio-gpu | `dispdrv`, `virtio` |
| Input | PS/2 + HID | `hiddrv` |
| Audio | Intel HDA + WASAPI engine | `audiodrv` |
| Network | e1000 + virtio-net, NDIS shim | `nicdrv` |
| Power | ACPI battery/thermal | `battery`, `power` |

QEMU-first: `virtio` covers blk/net/gpu/input so LoOS boots in the dev VM before
any real-hardware driver exists.

## Default graphics driver

The default display path needs **no vendor code**:

```
firmware (UEFI GOP / legacy VBE) sets a linear framebuffer
   -> bootloader passes {address, width, height, stride, pixel format} to kernel
      -> kernel "basic display driver" (BDD) exposes it as a display-class surface
         -> compositor (`comp`) renders windows, cursors, the shell
```

- Modeset at boot comes from the firmware; LoOS does not chain-load a vendor
  GPU firmware in the default path.
- The BDD is format-agnostic (BGRA/BGRX/RGBA 32bpp, RGB565, RGB888) and does the
  fill/blit the compositor needs. Hardware cursor is optional.
- Acceleration is added **above** the BDD, never required by it, so a machine
  with only a framebuffer still reaches the desktop: `virtio-gpu` in QEMU, then
  vendor GPUs.

### Acceleration tiers

1. **Tier 0 (default):** firmware framebuffer + software rendering in `comp`.
2. **Tier 1 (VM):** `virtio-gpu` (2D) with a `virgl`/`venus` 3D path for
   `d3dvk` (D3D11-on-Vulkan) — this is how Windows apps get D3D in QEMU.
3. **Tier 2 (real GPU):** a vendor KMS/miniport driver (below).

## Vendor GPU compatibility (AMD)

Two very different sources, two different plans.

### A. Linux `amdgpu` (recommended primary path)

`amdgpu` is the in-kernel DRM/KMS driver (GPL/MIT), and Mesa (`radeonsi`,
`radv`) is the userspace half. This is fully open, so it is the realistic AMDon-
LoOS plan:

- LoOS provides a **DRM/KMS-compatible display class** (`kms` task) — the kernel
  interface `amdgpu` expects (connectors, CRTCs, planes, fences, GEM).
- Port/build the **`amdgpu` kernel module** against that class (source reuse
  under GPL, logged in `THIRD-PARTY.md` per [SOURCE-POLICY.md](SOURCE-POLICY.md)).
- Userspace **Mesa** (`radeonsi` for GL, `radv` for Vulkan) runs in the **Lin**
  layer ([LINUX-COMPAT.md](LINUX-COMPAT.md)); the `d3dvk` bridge maps Windows
  D3D to Vulkan so Windows apps land on `radv`.
- Needs **GPU firmware** (microcode) loading (`gfxfw`), with the same
  no-telemetry rule as everything else.

### B. Windows AMD driver (WDDM)

The Windows AMD driver is a **WDDM miniport** (`DXGK` kernel interface, a `.sys`)
plus a proprietary **user-mode driver** (UMD) and Direct3D runtime. To load it:

- implement a **`DXGK`/`dxgkrnl` miniport host** (`amdwddm`) so the AMD `.sys`
  miniport can bind to the display class, and
- provide the **D3D UMD host + runtime** (`amdd3d`) the driver's UMD expects.

This is a long, reverse-engineering-heavy project, is governed by AMD's licence
(no redistribution), and depends on the whole WDDM/D3D stack existing first. It
is tracked but **deprioritised** behind the Linux path, which reaches working
AMD acceleration sooner with licensable code.

### Decision

| Path | Licence | Effort | Priority |
|---|---|---|---|
| Firmware framebuffer + `comp` software | ours | low | **do first** |
| `virtio-gpu` + virgl/venus (VM, D3D via Vulkan) | MIT/OSS | medium | **do second** |
| Linux `amdgpu` + Mesa on Lin layer | GPL/MIT | high | **primary AMD** |
| Windows AMD WDDM miniport + UMD | proprietary | very high | deprioritised |

## Vendor GPU compatibility (NVIDIA)

Same two-source structure as AMD, with one extra wrinkle: recent NVIDIA GPUs
need signed **GSP firmware** to run, so even the open driver depends on
firmware loaded at runtime.

### A. Linux `nouveau` + `nvk` (recommended primary path)

Open (GPL/MIT) and the realistic NVIDIA-on-LoOS plan:

- the same **KMS display class** (`kms`) that serves `amdgpu` also hosts
  `nouveau`;
- port/build the **`nouveau` kernel module** against it, including **GSP**
  firmware loading (`gfxfw`, `nv-fw` blob, no telemetry);
- userspace **Mesa** with the **`nvk`** Vulkan driver (and `zink`/`nv50` GL) runs
  in the **Lin** layer; the `d3dvk` bridge maps Windows D3D onto Vulkan, so
  Windows apps land on `nvk`.

### B. Windows NVIDIA driver (WDDM)

The Windows driver is a **WDDM `DXGK` miniport** (`.sys`) plus a proprietary UMD
and D3D runtime — same shape as the AMD Windows case. It needs the **`dxgkrnl`
miniport host** (`nvwddm`) and the **D3D UMD host** (`nvd3d`). Proprietary, no
redistribution, deprioritised behind the open `nouveau` path.

### C. Compute / CUDA

Open compute is reachable through **Vulkan compute** and **`vk`/Level-Zero-style`
paths on `nvk`; running the **proprietary CUDA driver** (`cudadev`) is the same
WDDM/kernel problem as B and is optional. OpenCL via Mesa's **rusticl** runs in
the Lin layer.

### NVIDIA decision

| Path | Licence | Effort | Priority |
|---|---|---|---|
| Firmware framebuffer + `comp` software | ours | low | shared with AMD |
| `virtio-gpu` + virgl/venus (VM) | MIT/OSS | medium | shared with AMD |
| Linux `nouveau` + `nvk` (GSP firmware) | GPL/MIT | high | **primary NVIDIA** |
| Windows NVIDIA WDDM miniport + UMD | proprietary | very high | deprioritised |
| proprietary CUDA driver | proprietary | very high | optional |

## Tasks

Default drivers: `acpi`, `pci`, `virtio`, `stornvme`, `partmgr`, `usb`,
`hiddrv`, `dispdrv`, `audiodrv`, `nicdrv`, `battery`, `hal`, `iomgr`, `pnp`,
`kmdf`, `codesign`.
Graphics: `gfxgop`, `gfxbdd`, `comp`, `virtgpu` (under `virtio`), `d3dvk`,
`d2d`, `dwm`, `dxgi`, `dx9`.
AMD/vendor GPU: `kms`, `amdgpu`, `mesa`, `gfxfw`, `amdwddm`, `amdd3d`.
NVIDIA/vendor GPU: `nouveau`, `nvk`, `nvwddm`, `nvd3d`, `cuda`.
Both vendor paths share `kms`, `gfxfw`, `mesa`, `d3dvk`, and the D3D UMD host.
