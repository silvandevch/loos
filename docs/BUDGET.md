# LoOS — working within a budget

The user funds this with **existing subscriptions** (a Codex plan, a Claude
plan, and a DeepSeek plan) — **not** a large API budget. Everything here is
designed to be cheap. Cost ≈ tokens × iterations; both are controllable.

## 1. Route by tier (the big lever)

| Tier | Agent | Use for | Why |
|---|---|---|---|
| cheap/fast | `claude` (Sonnet 5.5, medium) | breadth on self-contained tasks: UI, design, docs, review, mechanical tables, CI | fastest to iterate; a wrong pass is cheap to spot |
| workhorse | `sol` (DeepSeek V4.1 Flash, high) | most implementation: Win32, API sets, compat, backend, graphics bridge, Lin, toolchain, tests, security engineering | best-in-table agentic coding (DeepSWE 77.0) at a fraction of a frontier price |
| expensive | `astra` (GPT 6 Astra) | only the hardest: core kernel, novel security/exploit, hardest reverse-engineering, math | you pay for correctness where mistakes cost days |

Never spend Astra on bulk work — and don't send bulk work back to Astra just
because it is "best": at high effort Sol's coding scores above Astra's, so
re-routing a slice to Astra throws away the savings. Never let Sol or Claude
guess at novel security or the hardest kernel work (a wrong kernel patch costs
more later than handing it up).

## 2. Keep context small

- **Docs, don't greps.** `docs/` exists so agents don't re-derive facts from a
  250k-file tree. Read the doc, then the file.
- **Paste prompts in 3 short parts** (`prompts/<agent>/PART-1..3.md`), not one
  giant blob. Re-paste only what changed.
- **Sol has a 1M context — use it on the right things**: the spec, the adjacent
  subsystem, the failing log — not on dumping the tree.
- **Pass paths, not file bodies.** "implement per `prompts/spec/foo.md`" beats
  pasting the spec + the file + the test.
- **One subsystem per session.** A deep kernel session and a UI session are two
  contexts; don't merge them.

## 3. Keep iterations low

- **Spec first.** A task with `what / where / acceptance` is one round-trip; a
  vague one is five.
- **Claim on the bus, don't collide.** A collision wastes an entire generation.
- **Test locally before reporting** — a QEMU boot or `./loos verify` catches
  what would otherwise be a review loop.
- **Escalate early.** A cheap tier thrashing for 6 turns costs more than one
  good turn from the tier above. At medium effort Claude should hand off after
  two failed attempts at the same check.

## 4. Use the subscription tiers, not pay-as-you-go

- Do the long, exploratory work (reading source, drafting docs) on **Claude**
  where the plan is generous, and the bursty hard work on the Codex plan.
- Keep **Sol on high effort** for real work — its published scores are at high,
  and DeepSeek's effort map collapses medium into high anyway, so low/medium
  buys nothing but worse code.
- **Claude at medium** is the right default (what Claude Code/apps use); don't
  burn a max-effort run on tables or docs.
- Keep a task on one agent until it's actually blocked — context-switching
  re-pays the setup cost.

## 5. Cheap habits that add up

- Ask for a **diff/plan first**, code second, on anything large.
- Batch mechanical work into **one** request ("generate all 12 forwarders"),
  not 12 requests.
- Turn logs into short summaries before feeding them back to a model.
- `./loos bus` messages are plain text on localhost — use them instead of
  re-pasting state into every prompt.
- Snapshot/commit `.agentbus` state so a restarted session can resume instead
  of replaying history.

## 6. What NOT to do

- Don't run Astra on rename/format sweeps.
- Don't paste the same docs into three agents' contexts repeatedly.
- Don't let two agents edit overlapping files "to be safe" — it's the most
  expensive form of waste (merge conflicts + two contexts).
