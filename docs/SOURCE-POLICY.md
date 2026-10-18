# LoOS — source policy (no clean room)

This replaces the earlier clean-room plan. The operator's direction: agents may
read the original source directly. The goal is a working, original-as-possible
LoOS, not a legal isolation exercise.

## The three rules

1. **Read freely.** You may open, study, and learn from the NT 5.2 source tree
   (and anything else) to understand behavior and interfaces. Use it to answer
   "how does this work / what does this call / what does this return."

2. **Write your own code.** Prefer a fresh implementation of every function.
   **Do not copy source 1:1** — not line-for-line, not file-for-file — unless
   there is genuinely no other way to make it work (see below). Rewriting in your
   own structure, names, and style is the default.

3. **Borrow from open source when stuck.** If you can't implement something from
   scratch, use a proven open-source project (Linux, BSD, ReactOS, Wine, musl,
   Zircon, etc.). This is encouraged. **Respect those licenses** — see below.

## When 1:1 copying is allowed

Only as a last resort, and only for small, mechanical, low-creativity chunks
(e.g. a magic constant table, an on-disk structure definition, a hardware
register sequence). When you do:

- Add a comment naming the exact source (file + project/URL).
- Record it in `docs/PROVENANCE.md` (create it the first time).
- Prefer a link/reference over a copy when the value is a public spec.

Anything larger than that should be a rewrite or a borrow-from-OSS instead.

## Borrowing open-source code — license rules

| Source license | What it means for LoOS |
|---|---|
| **MIT / BSD / Apache-2.0** | Safe to use. Keep the copyright notice + license text in `THIRD-PARTY.md`. |
| **GPL / LGPL** | Usable, but **viral**: linking/combining GPL code makes the derived work GPL. LGPL is OK for libraries if you allow relinking. Note the obligation in `THIRD-PARTY.md`. |
| **CDDL / MPL** | File-level copyleft; keep those files separate and note it. |
| **Public domain / CC0** | Use freely; still credit it. |

Create and maintain **`THIRD-PARTY.md`** at repo root: one row per borrowed
component — name, version, license, files affected, where it's used. If you
borrow, you credit. No exceptions.

## What must be tracked

- `docs/PROVENANCE.md` — every 1:1 copy (should be rare), with exact source.
- `THIRD-PARTY.md` — every borrowed project, with license.

Agents: when you add either, post a `docs`-topic note on the bus (topic name
kept for continuity) so the others know.

## One honest note

Microsoft owns the copyright on the leaked tree. Studying it is fine; code that
is *derived* from it carries risk **if you publish it**. Rewriting to your own
expression greatly reduces that, and borrowing from permissively-licensed open
source avoids the issue entirely. That's why rules 1–3 are layered the way they
are: understand freely, write your own, borrow open source, and only copy MS
source 1:1 as a documented last resort. Your call how to weigh that — the tools
here just make the choices visible and recorded.

## Trademarks / branding

Don't ship the name "Windows", MS logos, or MS fonts. The OS is **LoOS**. API
names you must expose stay as-is (they're interface), but product identity is
ours. See [BRANDING.md](BRANDING.md).
