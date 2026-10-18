# LoOS — compatibility apps (rebranded)

Modern Windows apps that were decompiled, rewritten, and rebranded for LoOS.
They are **compatibility test targets**: if LoOS can run these *and* stock
Win7/10/11 apps, the Win32 surface is doing its job.

Branding is presentation-only — behavior, formats, and interfaces are unchanged
(see [BRANDING.md](BRANDING.md)).

## Registry

This registry tracks the application ports and their LoOS runtime status.

| App | Origin (era) | Source in repo | Rebrand only? | Runs on LoOS | Notes |
|---|---|---|---|---|---|
| LoOS Calculator | LoOS native app | `apps/calc/` | no | ☐ | `LoCalc.exe` cross-builds on Linux; arithmetic is local-only |
| LoCmd | NT command processor, extended for LoOS | `apps/loos-cmd/` | no | ☐ | Linux MinGW-w64 build; network clients/probes removed |
| LoNotepad | Windows 11 | `apps/loos-notepad/` | no | ☐ | Linux MinGW-w64 build; title, About UI, and metadata use LoOS branding |
| LoTaskMgr | Windows 7 / early 10 | `apps/loos-taskmgr/` | no | ☐ | Linux MinGW-w64 build; online lookups and unsupported shell/firewall actions removed |

Columns:
- **Origin (era):** which Windows version the app came from (drives the
  compatibility profile).
- **Source in repo:** where your rewritten/original source lives (`apps/<name>/`).
- **Rebrand only?:** yes if you changed only branding/metadata; no if you also
  changed behavior (call it out and why).
- **Runs on LoOS:** ☐ not yet / ☑ works / ✗ blocked (note the missing API).
- **Notes:** what broke, which API/shim it needed.

## How they're used

1. Each app is added to the **compatibility corpus** — see
   [DEVELOPMENT.md](DEVELOPMENT.md). CI boots LoOS in QEMU and launches the app.
2. When an app fails, the missing API/dll is recorded here and a bus task is
   posted (`topic: compat`) to `claude` (Win32/API sets) or `astra`
   (kernel/syscalls).
3. A passing app stays in the corpus forever as a regression gate.

## Per-app requirements to capture

When you add an app, note in its row or a small `apps/<name>/REQUIREMENTS.md`:

- **Imports**: the DLLs / API sets it needs (run a PE import dump).
- **Claimed version**: what `GetVersion`/`RtlGetVersion` returns it expects.
- **Files/system**: registry keys, file paths, services it assumes.
- **Graphics**: GDI / D3D / DPI needs.
- **Network**: any — and remember we strip telemetry.

## Where the sources live

```
apps/
  <name>/           your rewritten app source (original, rebranded)
  <name>/REQUIREMENTS.md   optional per-app notes
```

Do **not** commit vendor binaries or vendor assets. Keep originals in your
private workspace; the repo holds your rewritten sources and the manifest entry.

## Relationship to stock-app testing

Two corpora, both in CI:

- **stock** — unmodified real apps per era (7/10/11) proving true compatibility.
- **loos apps** — your rebranded apps proving the LoOS app set ships.

Both must pass before a milestone is done.
