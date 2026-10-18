# LoOS — branding

LoOS has its own identity. There is no Microsoft branding anywhere in the
shipped system, and the apps you rebranded are LoOS apps.

## Identity

- **Name:** LoOS (pronounced freely; "Lo" = Local Only).
- **Tagline:** "Local only."
- **Rule:** never ship the words/logo/wordmark "Windows", "Windows NT", or MS
  logos in LoOS UI, icons, or app metadata. API names in code are interface and
  stay as they are; *user-visible* identity is LoOS.
- **[you]** provide: final logo, accent color, font(s) you have the right to
  ship, and the product version string. Put assets in `branding/assets/`.

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
3. Rebrand with a consistent kit so every app looks like LoOS: `branding/` holds
   the shared strings, icons, and a `manifest.example.json` describing the
   substitutions.
4. Add the app to the compatibility corpus in [DEVELOPMENT.md](DEVELOPMENT.md)
   so it's tested alongside stock Win7/10/11 apps.

## Where branding lives

```
branding/
  assets/            logo, icons, accent, fonts  ([you] supply)
  strings.json       product/company/version defaults for the rebrand
  manifest.example.json   substitution manifest for one app
  README.md          how to run the rebrand
```

A rebrand helper can be added later (`branding/rebrand.py`) that applies the
manifest to an app tree (resource/string substitution). Until then, the
manifest documents intent so the agents can implement the substitutions
consistently.

> Note: decompiling and redistributing third-party apps has its own legal
> footing separate from LoOS's code. This doc treats "you already did the
> rewrite" as given; just keep the record in `THIRD-PARTY.md` so it's visible.
