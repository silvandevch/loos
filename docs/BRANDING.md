# LoOS — branding

LoOS has its own user-visible identity. Compatibility identifiers required by
Win32 remain in manifests and APIs; shipped artwork, titles, and app metadata
use LoOS branding.

## Identity

- **Name:** LoOS (pronounced freely; "Lo" = Local Only).
- **Tagline:** "Local only."
- **Rule:** never ship the words/logo/wordmark "Windows", "Windows NT", or MS
  logos in LoOS UI, icons, or app metadata. API names in code are interface and
  stay as they are; *user-visible* identity is LoOS.
- **Shared kit:** `branding/strings.json` is the identity record;
  `branding/brand.h` and `branding/brand.rcinc` provide matching source and
  resource constants for product name, company, tagline, version, and palette.
  Keep those values synchronized when identity changes.

## The rebrand workflow (your decompiled apps)

You decompiled modern Windows apps, rewrote them with Claude, and changed
branding (not compatibility). Keeping that repeatable and tidy matters, because
each app needs the same treatment — see [COMPAT-APPS.md](COMPAT-APPS.md) for the
per-app list.

For each app, the rebrand touches only **presentation/metadata**, never the
app's compatibility behavior:

| What to change | Examples |
|---|---|
| Strings / resources | product name, company, "About" box, file descriptions |
| Icons / images | app icon, splash, logo bitmaps |
| Window title / About | "LoOS <App>" instead of the vendor name |
| Manifest metadata | product/company/version fields |
| Menus / dialogs text | wording, "Check for updates" removed or pointed local |
| Telemetry calls | **removed** (LoOS is telemetry-free) |

What **not** to change: file formats, protocols, feature behavior, and the
Win32 interfaces — those are what keep the app compatible.

### Process

1. Keep raw decompiled/rewritten sources out of this repo if they carry vendor
   assets; keep only your original rewritten sources under `apps/<name>/`.
2. Record each app in [COMPAT-APPS.md](COMPAT-APPS.md) and in
   `THIRD-PARTY.md` (origin, license/legal note, what you changed).
3. Rebrand with a consistent kit. LoCmd, LoNotepad, LoTaskMgr, and LoCalc use
   shared brand values for shell output, titles, About text, colors, and version
   resources where applicable.
4. Add the app to the compatibility corpus in [DEVELOPMENT.md](DEVELOPMENT.md)
   so it's tested alongside stock Win7/10/11 apps.

## Where branding lives

```
branding/
  assets/            logo, icons, accent, fonts  ([you] supply)
  strings.json       canonical product/company/version record
  brand.h            shared C/C++ identity and palette constants
  brand.rcinc        shared Windows resource version constants
  manifest.example.json   substitution manifest for one app
  README.md          how to run the rebrand
```

The manifest records app-specific edits and review checks; it is not an
automated rewriter. App sources include the shared C and resource constants
directly.

> Note: decompiling and redistributing third-party apps has its own legal
> footing separate from LoOS's code. This doc treats "you already did the
> rewrite" as given; just keep the record in `THIRD-PARTY.md` so it's visible.
