# LoOS — overview

**LoOS** = **Lo**cal **O**nly **OS**. A desktop operating system that runs
Windows applications, built to be private by construction.

## What it is

- An x86-64 OS: your own bootloader, kernel, filesystem, graphics stack and
  desktop shell.
- **Application-compatible**: unmodified PE32/PE32+ programs written for
  Windows 7, 10 and 11 are the target (via the Win32/NT interfaces).
- **Privacy-first**: no telemetry, no device IDs, no activation servers, no
  cloud. Every network socket is visible to the user and controllable.
- **btrfs native**: the primary filesystem is btrfs (see
  [BTRFS.md](BTRFS.md)). NTFS is deliberately not the native FS.
- **Branded**: a custom desktop shell, and a set of modern Windows apps that
  have been decompiled, rebranded for LoOS, and rebuilt (see
  [COMPAT-APPS.md](COMPAT-APPS.md)).

## What it is not

- Not a Microsoft product, not affiliated with Microsoft, and not a fork of any
  Microsoft code. It *implements* compatible interfaces.
- Not a telemetry-light Windows — it is telemetry-**free**.
- Not (initially) a place to run kernel-mode third-party drivers or
  DRM/anti-cheat kernel filters.

## Design principles

1. **Local only.** Data stays on the machine unless the user sends it.
2. **Private by default.** The default state of every capability is "off until
   asked"; the UI shows network activity truthfully.
3. **Compatible.** The interface *is* the product. Apps must run.
4. **Legible.** Open source, documented, reproducible builds.
5. **Lean.** Small footprint, fast boot, no background phone-home.
6. **Honest engineering.** Where we borrow from open source, we say so and honor
   the license ([SOURCE-POLICY.md](SOURCE-POLICY.md)).

## Where the knowledge comes from

You are studying the leaked NT 5.2 source and other references to learn how the
interfaces behave. That study is allowed; the **shipped code is written fresh**.
See [SOURCE-POLICY.md](SOURCE-POLICY.md) for exactly what that means.

## Status

Early. Boot + kernel + a first Win32 surface are the current focus. See
[DEVELOPMENT.md](DEVELOPMENT.md) for the milestone board.
