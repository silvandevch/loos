# LoOS — security services (`crates/loos-security`)

Rust, `no_std` + `alloc`, `#![forbid(unsafe_code)]`, no external crates, host-tested
(`cd crates && cargo +1.97.1 test -p loos-security`). Nothing here talks to the network on its own and
nothing is reported anywhere: logs and traces stay local.

| Task | Module | What it does |
|---|---|---|
| #137 TLS + root store | `tls`, `x509`, `hash`, `aes`, `x25519`, `bignum` | TLS 1.3 client, sans-IO (`TLS_AES_128_GCM_SHA256`, X25519, `rsa_pss_rsae_sha256`); DER X.509 parser, chain/validity/hostname/pathLen validation, local distrust list. |
| #142 LogonUI | `logon` | Credential providers (password, PIN, pluggable), SAS-gated secure desktop (credentials refused on the default desktop), escalating lockout, equal-work unknown-user path, lock/unlock/logoff sessions. |
| #143 UAC | `uac` | Split tokens (filtered Medium / full High), integrity levels, one-shot expiring prompts answered on the secure desktop, audit trail. No auto-elevate allow-list by design. |
| #144 Disk encryption | `fde` | AES-256-XTS; random 512-bit volume key wrapped (RFC 3394) in up to 8 slots: passphrase (PBKDF2), 48-digit recovery key (mod-11 typo check), boot-measurement; MAC'd header; `EncryptedDisk` over any `BlockDevice`. |
| #145 Code signing | `codesign` | `image‖blob‖trailer` signatures (RSA PKCS#1 v1.5/SHA-256 over digest‖time), local dbx revocation, driver-load gate (Enforce / TestSigning), measured boot chain whose composite seals FDE keys. |
| #147 Event log + tracing | `evlog` | EVTX-style chunked CRC'd circular log (Security channel is retain-only; clearing is itself audited), query filters, persistence + corrupt-chunk recovery; ETW-style sessions with per-provider level/keyword masks and lossy rings. |

## Verified against published vectors
SHA-256, HMAC (RFC 4231), HKDF (RFC 5869), PBKDF2, AES-128/256 (FIPS 197), GCM, AES key wrap (RFC 3394),
XTS (IEEE 1619 #1), X25519 (RFC 7748), TLS 1.3 key-schedule prefix (RFC 8448). TLS is exercised end-to-end
against an in-test server built from the same primitives (not against a third-party server yet).

## Known limits (be honest)
- X.509: RSA keys + PKCS#1 v1.5/SHA-256 only; ECDSA/Ed25519 certs parse-fail verification (`Unsupported`). No EKU/name-constraints/CRL/OCSP.
- TLS: client only, one suite, no HelloRetryRequest, no session resumption, no client certs, no KeyUpdate.
- Crypto is plain software, not constant-time against cache attacks (RSA is public-key-only on the verify path; signing is a build-host tool).
- Secrets are not zeroized (no volatile writes without `unsafe`).
- The kernel integration (SAS keyboard hook, secure-desktop compositor switch, load-time driver check, FDE hook in the block layer) is not wired yet; these crates are the logic those hooks call.

## Local accounts: SAM and LSA (`loos-security::sam`)

- **SAM (`Sam`)**: machine SID `S-1-5-21-a-b-c`; built-in Administrator (RID 500) and Guest (501), both disabled until enabled;
  users and local groups from RID 1000; built-in aliases (Administrators 544, Users 545, Guests 546, Power Users 547,
  Backup Operators 551) under `S-1-5-32`. Names: 1-20 chars, none of `\ / [ ] : | < > + = ; , ? * @ "`, unique across users and
  groups (case-insensitive). Password policy (`min_len`, `complexity`, `max_age`) applies to create, reset and change.
  Group membership may nest; each user's effective groups (plus Everyone and Authenticated Users) feed the logon token.
  The last enabled administrator cannot be deleted, disabled or removed from Administrators.
- **LSA (`Lsa`)**: per-SID privileges and logon rights (`SeInteractiveLogonRight`, `SeDeny...`), default grants for the built-in
  groups, `token_for` builds the logon `Token` (groups from the SAM, privilege mask from the LSA, denied for disabled accounts or a
  deny right), and sealed secrets (AES-GCM, key derived from the boot key, name bound as AAD).
- **Persistence**: `Sam::to_bytes/from_bytes` and `Lsa::to_bytes/from_bytes` (strict parsers: truncation, trailing bytes and
  unknown rights are errors). Storing the blobs in the SAM hive is not wired yet. `loos-winrt` exposes the database to Windows programs
  (`Process::set_accounts`; a throw-away default "User" is used if the kernel installs none): `LookupAccountNameW/SidW`,
  `Convert(String)SidToSid(String)W`, `GetUserNameW`, `GetComputerNameW`, `OpenProcessToken`, `GetTokenInformation`
  (user, groups, privileges, elevation, integrity, session), `LogonUserW` (with lockout via `Sam::authenticate`),
  `LookupPrivilegeValueW/NameW`, `CheckTokenMembership`, `IsUserAnAdmin`. `NetUser*` is not provided. `Sam::export_to_logon` feeds `LogonUi` (same PBKDF2 iteration count).
- Salts and the boot key come from the caller: the crate has no RNG.
- **Boot hookup (`crates/kffi/src/win.rs`)**: `boot_accounts()` builds the database at kernel start from `krandom` entropy: random
  machine SID and LSA boot key, the disabled built-in Administrator/Guest, and one administrator `User` with a **blank password**
  (no remote logon exists; set a real one through the SAM). PBKDF2 cost is `BOOT_ITERATIONS` (4096, lower than
  `logon::ITERATIONS`; a `LogonUi` fed from this database must be built with the same value). Every Windows process gets its own
  clone via `Process::set_accounts`, so password or group changes made by one process are not seen by others until a shared
  account service exists.
