# LoOS — package managers (winget + apt, native)

LoOS ships **two first-class package managers built into the OS**: a
**winget-compatible** manager for Windows software, and a **native apt/dpkg**
engine for Debian/Ubuntu software. Both run on the LoOS kernel directly — not in
a VM, not in a chroot, not emulated (see [LINUX-COMPAT.md](LINUX-COMPAT.md) for
why native Linux binaries run at all). One daemon, one solver, one trust model,
one CLI.

## Why two

The user's workload is both Windows apps (the rebranded corpus, [COMPAT-APPS.md](COMPAT-APPS.md))
and Linux software (Debian/Ubuntu tooling, [LINUX-COMPAT.md](LINUX-COMPAT.md)).
Rather than pick a side, LoOS exposes both ecosystems through one package
service so `loos pkg install` works the same for a `.msix` and a `.deb`.

## The one service (`pkgd`)

`pkgd` is a single system service. Front-ends (CLI, the LoOS Store UI, MSI/APPX
shell integration, `apt`/`winget` shims) all talk to it. It owns:

- the **install database** (what is installed, from which source, which files it
  owns, its version and dependencies),
- the **dependency/version solver** (a shared SAT solver, `pkgapsolver`),
- the **download cache** and mirror selection (`pkgcache`),
- **transactional install/upgrade/rollback** (`pkgrollback`),
- the **trust gate** (`pkgtrust`) — every package must be signed by a trusted
  key or explicitly allowlisted, and every install is checked against the
  privacy rules (no telemetry, no unapproved egress).

## Format support

| Format | Engine | Notes |
|---|---|---|
| `winget` manifests (YAML) | `pkgwinget` + `pkgwingetman` | install/search/upgrade/source; community + custom REST sources |
| MSIX / APPX | `pkgmsix` | install/register/provision; per-app capability model |
| MSI | `pkgmsi` | msiexec-compatible engine + custom actions |
| `.deb` | `pkgapt` + `pkgdpkg` | native apt/dpkg: install/remove/upgrade, `/var/lib/dpkg` status db |
| apt repositories | `pkgaptrepo` | `sources.list`, GPG signature verification, mirror selection |
| distro base | `pkgbase` | bootstrap an Ubuntu/Debian rootfs into a LoOS namespace |

## Native apt (not a chroot)

The apt engine installs `.deb` payloads **into the LoOS namespace** using the
Linux VFS layer (`linfs`) built for [LINUX-COMPAT.md](LINUX-COMPAT.md): `/usr`,
`/etc`, `/var` are real LoOS filesystem paths that Linux binaries resolve to,
and `ld.so` + glibc come from the same package set. This is the same machinery
that runs an unmodified dynamic Debian binary (`L3`), so an installed package
runs for the same reason a copied-in binary does.

## CLI

```bash
loos pkg install <name|file>     # resolve across all sources/formats
loos pkg remove <name>
loos pkg update                  # refresh source metadata
loos pkg upgrade [name]          # upgrade all, or one
loos pkg search <query>
loos pkg list                    # installed
loos pkg info <name>             # versions, source, files, deps
loos pkg source add|remove|list  # winget sources + apt repos
```

`winget` and `apt` are also provided as thin shims so existing scripts work:

```bash
winget install 7zip.7zip
apt-get install -y build-essential
```

## Privacy

Package management is the one place the OS talks to the network a lot, so the
rules are explicit: metadata/downloads go only to the configured sources over
TLS; **no** usage telemetry, no install-behavior reporting, and every fetch is
recorded locally and visible in the privacy LED / `NetMonitor` backend
(`bNet`). The privacy lint in CI (`cipri`) gates any code path that could open
an outbound socket outside this service.

## Tasks

Owned in `pkg` (sol/heavy) unless noted: `pkgd`, `pkgapsolver`, `pkgwinget`,
`pkgwingetman`, `pkgmsix`, `pkgmsi`, `pkgapt`, `pkgaptrepo`, `pkgdpkg`,
`pkgtrust`, `pkgrollback`, `pkgoffline`, `pkgcache`, `pkgcli`, `pkgbase`
(lin-base), `pkgwsl`, plus the **LoOS Store** UI (`lostore`).
