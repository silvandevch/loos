# Luna — PART 2 of 3 · context and workspace

**LoOS** is a local-only, privacy-first OS that runs unmodified Windows
applications (Windows 7/10/11 PE binaries, plus rebranded host apps). It is
built natively on **btrfs** and ships **no telemetry**. The OS source is being
written **fresh** under `src/`; the NT 5.2 source tree is kept as a **study lab**
you may read at will (see policy in PART 1).

## Where you work

- **WORKDIR**: the repo root (`$LOOS_WORKDIR`, default `/run/media/silvan/DATA/LoOS`).
  Everything you create goes here (`src/`, `tests/`).
- **Study tree**: `$NT_SRC` (default `~/LoOS-build/srv03rtm`) — **read-only**.
  Read it, learn from it, never write to it.
- **Outputs**: `$OUT_ROOT` (DATA) — build artifacts, ISOs, VM disks. Not hand-edited.

Set your folder with `export LOOS_WORKDIR=<path>` or a line in `.loosrc`.
Full detail: `docs/AGENT-WORKSPACE.md`.

## Read before writing (docs, don't grep)

| Need | Read |
|---|---|
| what/why | `docs/OVERVIEW.md`, `docs/ARCHITECTURE.md` |
| layout | `docs/LAYOUT.md` |
| build/run | `docs/BUILD.md`, `docs/TOOLS.md` |
| your filesystem work | `docs/BTRFS.md` |
| backend modules | `prompts/BACKEND-SPEC.md` |
| UI contract | `prompts/UI-SPEC.md` |
| can I use this code? | `docs/SOURCE-POLICY.md` |
| cost/budget | `docs/BUDGET.md` |

Your deep specs live in `prompts/spec/` (prose interface facts). `docs/` is the
source of truth; if a spec disagrees with the code, fix both and say so.

## Build/verify (quick)

```bash
./loos status            # roots, tools, sanity
./loos build             # compile (Wine/host driver; see docs/BUILD.md)
./loos qemu              # boot the result
```

Next: **PART 3 — the bus and your workflow.**
