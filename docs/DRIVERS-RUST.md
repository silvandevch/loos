# Rust subsystems brought up by the boot kernel

All live in `crates/` and are linked into the kernel through `crates/kffi/src/sys.rs`; `src/kernel/boot/kmain.c`
calls them after the executive is up and prints one serial line each.

| Subsystem | Crate / file | Boot line | Hardware proof |
|---|---|---|---|
| Session manager (smss) | `loos-kernel/src/smss.rs`, `loos-winrt`, `kffi/src/win.rs` | `smss: ok (0xf)`; with `loos.win=1` + a `winroot` tar: `win: smss ok (0x1f)` | **real PE processes in ring 3** (`boot/win_test.sh`): autochk, csrss/wininit/services/lsass (session 0), csrss/winlogon (session 1) |
| Compositor + D3D11/DXGI bridge | `loos-compositor` | `compositor: ok (0xf)` | **the desktop is presented through it**: `video_present` -> `loos_comp_present` (BGRX scan-out). Screenshot: `out/desktop-compositor.png` |
| TCP/IP | `loos-net` | `net: ok (0xf)`, `net: e1000 10.0.2.15/24, gateway 10.0.2.2, dns 10.0.2.3, ping gateway reply` (address from DHCP; VMware 82545EM `0x100f` accepted too) | **QEMU e1000: ARP + ICMP echo to the SLIRP gateway** (pcap-verified). `shellhost_loop` polls the NIC every frame |
| xHCI + HID + USB mass storage | `loos-drivers/src/{xhci,usb}.rs` | `usb: xHCI ..., devices 3 (keyboards 1, mice 1, storage 1, sectors 16384)` | **QEMU `-device qemu-xhci -device usb-kbd -device usb-mouse -device usb-storage`** |
| AHCI / NVMe | `loos-drivers/src/{ahci,nvme}.rs`, glue `src/kernel/boot/rustblk.c` | disks register as `sata%d` / `nvme%d` | **these are now the kernel's only AHCI/NVMe drivers** (`ahci.c`/`nvme.c` removed); `boot/bios/install_test.sh` passes on IDE, AHCI and NVMe (install, `btrfs check`, boot installed disk). Writes end with FLUSH CACHE EXT / NVMe Flush |
| Display modesetting | `loos-drivers/src/gpu.rs` (EDID, Bochs VGA) | `gpu: bochs vga, modes 8` | probed on QEMU std VGA; `loos_gpu_set_mode` exported but unused (the boot framebuffer mode is kept) |

DMA memory is `kalloc`'d (identity-mapped, so bus address = pointer). Reproduce the USB line:

    boot/bios/build.sh out/boot-int --cmdline "console=ttyS0 loos.test=1"
    qemu-system-x86_64 -display none -serial file:log -drive file=out/boot-int/bios.img,format=raw,if=ide \
      -device qemu-xhci -device usb-kbd -device usb-mouse -drive if=none,id=u,file=usb.img,format=raw -device usb-storage,drive=u

Stack clock: call `Stack::poll(now)` (or `loos_net_ping(.., now)`) before queuing traffic; a stale clock expired the first ARP-pending packet.

## Windows process runtime (`loos-winrt`)
x64 PE images only, loaded at their preferred base; imports bind to `mov r10,rcx; mov eax,N; syscall; ret` stubs. Implemented:
ExitProcess/TerminateProcess, GetStdHandle, WriteFile/WriteConsoleA, GetCommandLineA, GetCurrentProcessId, ProcessIdToSessionId,
Sleep (INFINITE parks a resident service), GetTickCount, VirtualAlloc, Heap*, Get/SetLastError, GetModuleHandleA. Unimplemented imports
are reported by name when called and end the process with STATUS_NOT_IMPLEMENTED. Not yet: CRT/other DLLs, threads, relocations, files, registry.
