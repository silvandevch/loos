# Package manager cores

## `crates/dpkg` (`loos-dpkg`, tasks #175 + parsing half of #174)
- `version` – Debian version ordering (epoch, `~`, letters-before-symbols, numeric runs; policy 5.6.12).
- `control` – stanza parser/serializer for `/var/lib/dpkg/status` and `Packages` (continuations, ` .` blank lines).
- `deps` – `a (>= 1) | b:any, c [arch] <profile>` parser; restrictions are ignored.
- `resolver` – `Universe::plan_install` (dependency closure in install order, alternatives tried left to right preferring the named package then newest version, Provides, Conflicts/Breaks, cycle-tolerant, upgrade detection) and `reverse_deps` for removal. Not done: Recommends/Suggests, version-pinning, backtracking across alternatives, multiarch co-installation, maintainer-script execution, and the apt HTTP/GPG client (#174 remainder, needs net + signature code).

## `crates/winget` (`loos-winget`, task #170)
- `yaml` – strict YAML subset (block maps/seqs, quoted/plain scalars, `|`/`>` blocks, flow seqs). Rejects anchors, tags, flow maps, tabs, duplicate keys, multi-doc.
- `manifest` – singleton or multi-file manifests; requires https URLs and 64-hex SHA-256 for every installer; `select_installer(host_arch, scope)`; `cmp_version`.
- `index` – search (id/moniker/name/tags), `latest`, `version`, `upgrades`. The downloader must verify the SHA-256 before running anything (#176 trust gate).

Verify: `cargo +1.97.1 test -p loos-dpkg -p loos-winget` (from `crates/`).

## `crates/msix` (`loos-msix`, task #171)
`zip` – read-only ZIP (stored/deflate, no ZIP64/encryption) with zip-slip name validation at open, per-entry size cap, inflate limited to the declared size, CRC-32 verified on every read. `xml` – DTD-free subset (depth ≤ 64, entity-expansion safe). `manifest` – `AppxManifest.xml` identity/properties/dependencies/applications/capabilities with validation (4-part version, name charset, known architectures). `Package::open` checks every `Application Executable` exists; `payload()` excludes store metadata; `install_dir()` yields `Name_Version_Arch__Publisher`. Signature is only detected (`signed`), never trusted — verification belongs to #176. Not done: block-map hash verification, bundles (.msixbundle), registration (file-type/protocol/app-execution aliases), dependency install, per-user vs provisioned state. Test: `cargo +1.97.1 test -p loos-msix`.

## `crates/pkgstate` (`loos-pkgstate`, tasks #176 + #177)
- `trust` – deny-by-default `Policy`: https only, no userinfo/odd ports, host allowlist (`*.x.y` = subdomains only), 64-hex SHA-256 required, signed publisher must be allowlisted unless `allow_unsigned`. Nothing is built in; the admin supplies hosts and publishers (privacy gate: no vendor default endpoints). Signature *verification* itself (PKCS#7/Authenticode/p7x) is still open; `signed_publisher` must come from a verified check.
- `sha256` – streaming SHA-256 + constant-time `verify`.
- `ownership` – path→package db (case-insensitive normalized paths, `..` rejected), shared-file co-ownership, conflict detection, upgrade stale-file report, `plan_remove` (deepest first, keeps files other packages own), `missing()` repair report, text (de)serialization.
- `journal` – write-ahead install journal; `rollback_plan()` returns inverse ops newest first; crash-safe text form (partial last line dropped); `needs_recovery()` for boot. Not done: actually applying undo ops, A/B system rollback (#209), file hash tracking for repair-by-redownload.
Test: `cargo +1.97.1 test -p loos-pkgstate`.

## `crates/msi` (`loos-msi`, task #172 — read side)
`cfb` – OLE compound file reader (v3/v4 sectors, DIFAT, mini stream, loop/size guards). `database` – MSI stream-name codec, string pool (narrow/wide indices), `_Tables`/`_Columns`, column-major row decoding into `Table`/`Cell`; cabinet/icon streams land in `blobs`. `plan` – `Plan::build(db, props)`: Directory resolution (property overrides such as `INSTALLDIR`, standard folders kept as symbolic roots, `..`/cycle rejection), File list in media-sequence order with long names, features + sizes by level, and a **custom-action audit** (`risky_actions()` = DLL/EXE/script; `elevated` flags deferred no-impersonate actions). The engine never executes custom actions. **Caveat:** the tests use fixtures written by a test-side CFB/MSI encoder written from the format spec, not a Microsoft-built MSI (none available offline); validate against real packages when one is at hand. Not done: CAB (MSZIP/LZX) extraction, long-string pool entries, transforms (.mst), patches (.msp), the sequence-table interpreter, conditions, registry/shortcut/service tables, writing the uninstall record (hand off to `pkgstate`). Test: `cargo +1.97.1 test -p loos-msi`.
