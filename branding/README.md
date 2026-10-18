# branding/ — the LoOS brand kit + app rebrand

Shared, consistent branding for the OS and the apps you rebranded. See
[../docs/BRANDING.md](../docs/BRANDING.md) and
[../docs/COMPAT-APPS.md](../docs/COMPAT-APPS.md).

```
branding/
  strings.json            default product/company/version strings
  manifest.example.json   substitution manifest for one app
  assets/                 [you] drop logo, icons, fonts here
```

## strings.json

The canonical LoOS identity used everywhere. Build tooling and app rebrands read
these so the identity is defined once.

## manifest.example.json

Describes how to rebrand one app **without changing behavior**: which strings to
replace, which icons/assets to swap, which metadata fields to rewrite, and which
telemetry calls to delete. Copy it to `manifest.json` inside an app and fill it
in; a future `rebrand.py` applies it, and it also documents intent so Claude or
An agent can implement the substitution.

## Rebrand checklist (per app)

1. Copy `manifest.example.json` → `apps/<name>/manifest.json`, fill it in.
2. Apply string/resource/icon substitutions (presentation only).
3. **Remove all telemetry/update/network calls** — LoOS is telemetry-free.
4. Keep file formats, protocols, and Win32 usage unchanged.
5. Add the app to [../docs/COMPAT-APPS.md](../docs/COMPAT-APPS.md) and
   `THIRD-PARTY.md`.
6. Add it to the compat corpus in [../docs/DEVELOPMENT.md](../docs/DEVELOPMENT.md).

Do not commit vendor binaries or vendor assets; keep those private.
