# branding/ — the LoOS brand kit + app rebrand

Shared, consistent branding for the OS and the apps you rebranded. See
[../docs/BRANDING.md](../docs/BRANDING.md) and
[../docs/COMPAT-APPS.md](../docs/COMPAT-APPS.md).

```
branding/
  strings.json            canonical human-readable identity
  brand.h                 shared C/C++ identity and color tokens
  brand.rcinc             shared Windows resource version values
  manifest.example.json   substitution manifest for one app
  assets/                 app artwork and licensed fonts
```

## strings.json

Canonical human-readable identity. C/C++ sources include `brand.h` and Windows
resource scripts include `brand.rcinc`; keep those values synchronized with
this JSON file when changing the product identity.

## manifest.example.json

Describes the app-specific branding edits and metadata fields. It is a checklist
for review; it does not currently drive an automated rewrite.

## Rebrand checklist (per app)

1. Copy `manifest.example.json` → `apps/<name>/manifest.json`, fill it in.
2. Use shared brand tokens for product/company/version strings and palette;
   update app resources, manifests, About text, and window titles.
3. **Remove all telemetry/update/network calls** — LoOS is telemetry-free.
4. Keep file formats, protocols, and Win32 usage unchanged.
5. Add the app to [../docs/COMPAT-APPS.md](../docs/COMPAT-APPS.md) and
   `THIRD-PARTY.md`.
6. Add it to the compat corpus in [../docs/DEVELOPMENT.md](../docs/DEVELOPMENT.md).

Do not commit vendor binaries or vendor assets; keep those private.
