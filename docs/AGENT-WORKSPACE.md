# LoOS — agent workspace (where the AIs work)

Short answer: **the agents work in the LoOS repo root** —
`/run/media/silvan/DATA/LoOS`. You can point them elsewhere with **one setting**
(`LOOS_WORKDIR`). This page says exactly how.

## The folders

| Folder | Purpose | Agents |
|---|---|---|
| `$LOOS_WORKDIR` (default = repo root) | **the working directory** — where agents create/edit code | read+write |
| `src/` (created by them) | LoOS OS source (kernel, win32, fs, shell) | read+write |
| `apps/<name>/` | your rebranded apps | read+write |
| `prompts/spec/` | interface facts they implement from | read+write |
| `docs/`, `prompts/` | specs and prompts | read (write only to update docs) |
| `$NT_SRC` (`~/LoOS-build/srv03rtm`) | **the original source** they study | **read-only** |
| `$OUT_ROOT` (`DATA/loos/out`) | build output, ISOs, VM disks | written by tools, not hand-edited |

## How to set the working folder

The working dir is the environment variable **`LOOS_WORKDIR`**. `env.sh` sets it
to the repo root by default; override it in `.loosrc` (gitignored) or the
environment.

**Option A — edit `.loosrc` (persistent, per machine):**

```bash
# /run/media/silvan/DATA/LoOS/.loosrc
LOOS_WORKDIR=/run/media/silvan/DATA/LoOS
```

**Option B — environment (per session):**

```bash
export LOOS_WORKDIR=/run/media/silvan/DATA/LoOS
```

**Option C — tell the agent in its prompt.** The prompts in `prompts/*/PART-2.md`
contain a line like `WORKDIR: <path>`; set it to whatever folder you want that
agent to build in. This is the practical way when the agents run as separate CLI
sessions.

Then verify:

```bash
./loos status        # prints SRC_ROOT / OUT_ROOT
echo "$LOOS_WORKDIR"
```

## Recommended: give each agent its own worktree (optional)

If you want them fully independent (no file collisions), give each a **git
worktree** or subfolder and set `LOOS_WORKDIR` per session:

```bash
git worktree add /run/media/silvan/DATA/LoOS-work/astra
git worktree add /run/media/silvan/DATA/LoOS-work/claude
# then: export LOOS_WORKDIR=/run/media/silvan/DATA/LoOS-work/astra
```

They still share **one bus** (same `AGENTBUS_DIR`), so they coordinate even from
separate folders.

## Rules for the workspace

- Never hand-edit `$NT_SRC` (the study tree) — the prep scripts own it. Read it,
  don't write it.
- Never hand-edit build output under `$OUT_ROOT` — regenerate it.
- Anything you create goes under `$LOOS_WORKDIR` (`src/`, `apps/`, `tests/`…).
- Keep big generated files off the repo; they belong on `$OUT_ROOT` (DATA).
- The bus state lives in `$AGENTBUS_DIR` (default `$LOOS_ROOT/.agentbus`) and is
  shared by all agents regardless of their `LOOS_WORKDIR`.
