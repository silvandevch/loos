# Network-egress audit

Policy (docs/OVERVIEW.md rule 3): no telemetry, ever; nothing leaves the machine unless the user started it, and the
network LED + firewall show it truthfully. This document lists every place in the tree that can cause traffic, why it
exists, and how it is controlled. Re-audit whenever `privacy.lint` needs an allowlist change.

Audit date: 2026-10-19. Scope: `src/`, `apps/`, `scripts/`, `branding/`, build files.

## Enforced by CI (`ctest -R privacy.lint`, `tests/privacy/privacy_lint.py`)

- Direct socket and name-resolution calls: `socket connect bind listen accept send* recv* getaddrinfo ...`
- Winsock/WinInet/WinHTTP/URLMon API names, linking `ws2_32 wininet winhttp urlmon`, `LoadLibrary("ws2_32")`
- Any literal `http(s)/ftp/ws(s)://host` string in code (comments excluded). Allowed hosts: `mirror.example`,
  `example.com`, `example.org`, `localhost`, `127.0.0.1` (UI placeholders and loopback only).
  The scanner understands string literals, so a URL's `//` is not mistaken for a comment.

Result at audit time: 143 files scanned, 0 findings.

## Findings fixed in this audit

| Where | Problem | Fix |
|---|---|---|
| `apps/loos-notepad/notepad.c` `NP_HELP_URL` | Help opened `https://go.microsoft.com/fwlink/...` via `ShellExecute` (a vendor server, reached from a LoOS app) | now `loos-help://notepad`, resolved locally by the shell; never a web address |
| `tests/privacy/privacy_lint.py` | comment regex treated the `//` of any URL as a comment, so URLs were invisible to the lint | string-aware scanner; new external-URL rule |

## Remaining intentional egress points (all user-initiated)

| Point | Trigger | Control |
|---|---|---|
| Update mirror (`updates.mirror`) | user enables updates **and** sets a mirror **and** presses Check | opt-in; default empty (no built-in address); `updates.offline` forbids it; subject to per-process firewall |
| Package managers (winget/apt clients, tasks #170-#182) | user runs an install | same firewall; repositories are user-configured; signatures verified before use (`pkg: trust/signing`) |
| `scripts/winapp-probe.py`, `scripts/wine-source.sh`, `scripts/00-fetch.sh` | developer runs them on a build machine | **build-time only**, never shipped in an image; they fetch test binaries/sources from pinned URLs |
| Apps the user runs | whatever the app does | the firewall defaults to ASK per process; the tray LED turns amber on any open socket; the "block all network" switch overrides everything |

## Things the shell guarantees (see `docs/SHELL.md`)

- The shell core has no socket API at all: its only network-related surface is the read-only `net_query` and the
  `fw_*` calls of the backend, which describe traffic rather than create it.
- Notifications have no push channel; they are published by local processes only and gated by a per-app permission.
- Permissions default to ASK; a flood of requests fails closed.

## Open items

- When the NetMonitor/Firewall services land, add a boot-time test that opens a socket from a test process with the
  hard switch on and asserts it is refused and the LED goes red (tracked with `net: firewall` #135).
- Extend the lint to Rust crates when the Rust surface (#246) lands (`std::net`, `reqwest`, `hyper`, `ureq`).
