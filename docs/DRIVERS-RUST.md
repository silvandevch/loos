# Rust subsystems brought up by the boot kernel

All live in `crates/` and are linked into the kernel through `crates/kffi/src/sys.rs`; `src/kernel/boot/kmain.c`
calls them after the executive is up and prints one serial line each.

| Subsystem | Crate / file | Boot line | Hardware proof |
|---|---|---|---|
| Session manager (smss) | `loos-kernel/src/smss.rs` | `smss: ok (0xf)` | policy only; process creation waits for the PE loader (`KLauncher` allocates pids) |
| Compositor + D3D11/DXGI bridge | `loos-compositor` | `compositor: ok (0xf)` | software path |
| TCP/IP | `loos-net` | `net: ok (0xf)` | loopback; NIC glue pending |
| xHCI + HID + USB mass storage | `loos-drivers/src/{xhci,usb}.rs` | `usb: xHCI ..., devices 3 (keyboards 1, mice 1, storage 1, sectors 16384)` | **QEMU `-device qemu-xhci -device usb-kbd -device usb-mouse -device usb-storage`** |
| AHCI / NVMe | `loos-drivers/src/{ahci,nvme}.rs` | exported as `loos_ahci_probe` / `loos_nvme_probe`, not auto-called: the C drivers (`ahci.c`, `nvme.c`) still own those controllers | host-tested against device models |
| Display modesetting | `loos-drivers/src/gpu.rs` (EDID, Bochs VGA) | not yet called by `kmain` | host-tested |

DMA memory is `kalloc`'d (identity-mapped, so bus address = pointer). Reproduce the USB line:

    boot/bios/build.sh out/boot-int --cmdline "console=ttyS0 loos.test=1"
    qemu-system-x86_64 -display none -serial file:log -drive file=out/boot-int/bios.img,format=raw,if=ide \
      -device qemu-xhci -device usb-kbd -device usb-mouse -drive if=none,id=u,file=usb.img,format=raw -device usb-storage,drive=u
