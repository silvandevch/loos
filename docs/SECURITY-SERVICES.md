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
