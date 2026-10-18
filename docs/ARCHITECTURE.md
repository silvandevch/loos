# LoOS — architecture

The detailed backend spec lives in [`../prompts/BACKEND-SPEC.md`](../prompts/BACKEND-SPEC.md);
this is the map. If the two ever disagree, fix this doc and the spec together.

```
+---------------------------------------------------------------+
|  Shell / UI          LoOS desktop (own)      -> UI-SPEC.md      |
+---------------------------------------------------------------+
|  Rebranded apps      your decompiled+rebranded apps  (COMPAT-APPS.md)
+---------------------------------------------------------------+
|  Win32 subsystem     user32 gdi32 shell32 comctl32 advapi32     |
|                      ole32 ws2_32 dxgi/d3d  (own code)          |
+---------------------------------------------------------------+
|  API sets + compat   api-ms-win-*, per-app version profiles     |
+---------------------------------------------------------------+
|  Executive           Ob  Mm  Io  Ps  Fs  Se  (NT-compatible)    |
+---------------------------------------------------------------+
|  Kernel              sched, VM, IRQL, syscall entry, HAL        |
+---------------------------------------------------------------+
|  Boot                UEFI loader / legacy chainload             |
+---------------------------------------------------------------+
        Filesystems:  btrfs (native)  +  FAT32 (esp/tools)          -> BTRFS.md
        No telemetry. No network I/O that the user didn't start.    -> OVERVIEW.md
```

## Language

Every layer is **Rust by default**; C is used only where Rust is a worse fit
(vendor C-only interfaces, assembly, or modules that would be almost entirely
`unsafe`). See [LANGUAGE.md](LANGUAGE.md). The C ABI is the boundary between
them, so a Rust module and a C module link seamlessly.

## Layer notes

- **Boot** — one kernel image boots via UEFI (`BOOTX64.EFI`) and legacy BIOS;
  passes a memory map + ACPI/SMBIOS table to the kernel.
- **Kernel** — scheduler, virtual memory, IRQL discipline, syscall entry/return,
  timers, SMP. The syscall ABI is *ours*; a translation table maps the documented
  NT contract so compatibility shims work. (See SOURCE-POLICY for how we may
  read the original to learn that contract.)
- **Executive** — object manager, memory manager, I/O manager (IRP-like model),
  process/thread, security tokens, registry with real hive layout.
- **Filesystems** — **btrfs is native and read/write**; FAT32 for ESP/tooling.
  Legacy NTFS volumes may be mounted **read-only** for convenience, but are not
  the system FS. All of this is in [BTRFS.md](BTRFS.md).
- **Win32 subsystem** — the big compatibility surface: kernel32/user32/gdi32,
  shell32/comctl32/shlwapi, advapi32, ole32/oleaut32, ws2_32, and a
  DXGI/D3D11 (vulkan-backed) path for games.
- **API sets & compat** — modern apps import `api-ms-win-*`, not raw DLLs. A
  forwarder resolves those names to our implementations, and a per-app
  *compatibility profile* decides which Windows version we report and which
  down-level behaviors to emulate. This is what makes Win7/10/11 apps run.
- **Shell/UI** — our own desktop; see [`../prompts/UI-SPEC.md`](../prompts/UI-SPEC.md).

## Running apps from Windows 7 / 10 / 11

| Era | Needs | We cover with |
|---|---|---|
| Win7 | old imports, `GetVersion` ≤ 6.1, GDI-era shell assumptions | classic entry points + version profile + DPI virtualization |
| Win10 | API sets, UCRT, bcrypt, d3d11, modern shell | API-set forwarders, UCRT, D3D bridge, modern compositor |
| Win11 | newer API sets, dwmapi additions | superset API sets + report Win11 levels where required |

A **compatibility corpus** of real apps per era is a hard gate in CI — see
[DEVELOPMENT.md](DEVELOPMENT.md). Add every rebranded app from
[COMPAT-APPS.md](COMPAT-APPS.md) to it.

## Privacy architecture (first-class, not a checkbox)

- No outbound connection unless a user action caused it.
- No IDs, no analytics, no activation callbacks, no "compatibility telemetry".
- A **network firewall + live egress view** in Settings, per process.
- Updates/search/DNS are local or opt-in and self-hostable.
- Build is reproducible from source; you sign with your own key.

## Milestones

M0 boot → M1 kernel core → M2 kernel objects/syscalls → M3 btrfs + registry →
M4 native test binary → M5 PE loading → M6 Win32 core → M7 API sets + UCRT →
M8 graphics/compositor → M9 Win7/11 shims + corpus in CI → M10 privacy
hardening + reproducible ISO. Owner split in [`../prompts/coordination.md`](../prompts/coordination.md).
