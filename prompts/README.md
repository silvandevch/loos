# prompts/ — what you paste into each agent

Docs live in [`../docs/`](../docs/README.md). This folder holds only what an AI
agent is handed at the start of a session, plus the specs it implements.

> Every agent should read [`../docs/AGENTS.md`](../docs/AGENTS.md) first (it
> points at everything else). The prompt parts below assume that.

## Paste order (multi-part, per agent)

Each agent gets **three short parts pasted in order** — small enough to avoid
truncation and cheap enough to re-paste.

| Agent (bus id) | Model | Strong at | Paste |
|---|---|---|---|
| `astra` | **GPT 6 Astra** | kernel, VM, syscalls, btrfs, drivers, novel security, hardest RE, math — the hardest, correctness-critical work | `astra/PART-1.md`, `PART-2.md`, `PART-3.md` |
| `sol` | **DeepSeek V4.1 Flash 1017** (thinking, high effort) | the workhorse and the strongest agentic coder on repo-scale work (DeepSWE v1.1 77.0) — Win32 subsystem, API sets, compat, backend, graphics bridge, Lin bridge/base, toolchain, tests, security engineering | `sol/PART-1.md`, `PART-2.md`, `PART-3.md` |
| `claude` | **Claude Sonnet 5.5** (medium effort) | breadth on well-scoped work — UI/shell, design & branding, docs & writing, review, mechanical tables & CI, computer use | `claude/PART-1.md`, `PART-2.md`, `PART-3.md` |

- **PART 1** — who you are, your tier, the rules.
- **PART 2** — LoOS context, where you work, what to read.
- **PART 3** — the bus and your workflow.

Routing by category capability, not by who's free: **Claude → Sol → Astra.** Push
breadth/UI/design/docs/review/mechanical tables down to Claude; take novel
security (exploit invention), the debug-heavy core and math up to Astra. Owners are computed by `scripts/seed_tasks.py` from a
competence matrix + load balancer — see [`../docs/AGENTS.md`](../docs/AGENTS.md) §Routing.

## How they talk (the bus)

A real broker (`agents/busd.py`: SQLite + TCP), not a file. One shared state dir.

```bash
./loos bus start
./loos bus inbox --for astra --unread
./loos bus send --from astra --to claude --topic kernel "…"
./loos bus thread kernel
./loos bus task ls
```

Full protocol, topics and rules: [`coordination.md`](coordination.md) and
[`../agents/README.md`](../agents/README.md).

## Other files

- `coordination.md` — roles, topics, escalation, conflict/review rules.
- `BACKEND-SPEC.md` — backend module spec (mirrors
  [`../docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md); if they disagree, fix both).
- `UI-SPEC.md` — the LoOS desktop shell and its backend contract.
- `spec/` — prose interface facts the agents implement from.

## The policy (short version)

Read the original source freely; write your own code; don't 1:1 copy unless
there's genuinely no other way; borrow OSS when stuck and honor its license.
Full rules: [`../docs/SOURCE-POLICY.md`](../docs/SOURCE-POLICY.md).
