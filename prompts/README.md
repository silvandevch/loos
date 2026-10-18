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
| `astra` | **GPT 6 Astra** | kernel, VM, syscalls, btrfs, drivers, crypto — hard, perf-critical, easy-to-get-wrong | `astra/PART-1.md`, `PART-2.md`, `PART-3.md` |
| `claude` | **Claude Sonnet 5.5** | architecture, Win32 subsystem, API sets, UI/shell, integration, **review** | `claude/PART-1.md`, `PART-2.md`, `PART-3.md` |
| `luna` | **GPT 6 Luna** | mechanical volume + **porting Win XP apps to LoOS apps**; weak at core | `luna/PART-1.md`, `PART-2.md`, `PART-3.md` |

- **PART 1** — who you are, your tier, the rules.
- **PART 2** — LoOS context, where you work, what to read.
- **PART 3** — the bus and your workflow.

Routing by difficulty: **Luna → Claude → Astra.** Take the hardest work to
Astra.

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
