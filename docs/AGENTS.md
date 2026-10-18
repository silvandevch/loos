# LoOS — how the agents work here

Three AI agents build LoOS together. This page is the short version: who does
what, how they talk, and the rules. Read
[SOURCE-POLICY.md](SOURCE-POLICY.md) once, then work. Where to work:
[AGENT-WORKSPACE.md](AGENT-WORKSPACE.md). Budget: [BUDGET.md](BUDGET.md).

## The agents

| Bus id | Model | Strength | Owns |
|---|---|---|---|
| `astra` | **GPT 6 Astra** (best) | hard, correctness-critical, performance | kernel, VM, syscalls, btrfs, drivers, security, crypto — anything easy to get wrong |
| `claude` | **Claude Sonnet 5.5** | good general implementer | architecture, Win32 subsystem, API sets, UI/shell, integration, review |
| `luna` | **GPT 6 Luna** (fast) | speed over depth | mechanical work + **porting Win XP apps to LoOS apps**; API-set tables, stubs, tests, CI, docs |

Route by difficulty, not by who's free: **Luna → Claude → Astra**. If a task is
harder than your tier can do *correctly*, hand it up with what you found. Astra
takes the performance-critical and easy-to-get-wrong work; Claude implements
broadly and reviews; Luna does the volume and the app porting.

## Talking to each other (the bus)

A real broker (SQLite + TCP), not a file. Use it every turn:

```bash
./loos bus start                                    # once
./loos bus inbox  --for astra --unread              # 1. read yours
./loos bus send   --from astra --to claude --topic kernel --priority high \
                  --subject "section fault path" "spec: … acceptance: …"
./loos bus thread kernel                            # 3. the discussion
./loos bus ack    <id> --by astra                   # 4. done with one
./loos bus watch  --for claude                       # block for new messages
./loos bus task ls                                   # the board
```

Loop: **read inbox → claim (task or a message) → work → report (`--ref`) → ack.**
One writer per file. Topics and full protocol: `../prompts/coordination.md`.

## Your prompt

Paste the parts **in order** for your model:
- Astra: `../prompts/astra/PART-1.md` … `PART-3.md`
- Claude: `../prompts/claude/PART-1.md` … `PART-3.md`
- Luna: `../prompts/luna/PART-1.md` … `PART-3.md`

## Where to look (read, don't grep)

| Need | Read |
|---|---|
| what/why | [OVERVIEW.md](OVERVIEW.md), [ARCHITECTURE.md](ARCHITECTURE.md) |
| where to work | [AGENT-WORKSPACE.md](AGENT-WORKSPACE.md) |
| build/run | [BUILD.md](BUILD.md), [TOOLS.md](TOOLS.md) |
| filesystem | [BTRFS.md](BTRFS.md) |
| UI | [`../prompts/UI-SPEC.md`](../prompts/UI-SPEC.md) |
| backend modules | [`../prompts/BACKEND-SPEC.md`](../prompts/BACKEND-SPEC.md) |
| can I use this code? | [SOURCE-POLICY.md](SOURCE-POLICY.md) |
| apps | [BRANDING.md](BRANDING.md), [COMPAT-APPS.md](COMPAT-APPS.md) |
| cost | [BUDGET.md](BUDGET.md) |

## Rules (all of them)

1. Read the original source if useful; write your own code; don't 1:1 copy unless
   unavoidable; borrow OSS with license notes ([SOURCE-POLICY.md](SOURCE-POLICY.md)).
2. Keep a provenance row for any copy (`docs/PROVENANCE.md`) or borrow
   (`THIRD-PARTY.md`).
3. btrfs native, privacy always, no telemetry.
4. Test before reporting; kernel work gets a QEMU boot test.
5. Change behavior → update the doc.
6. Unsure? Ask on the bus. `--priority urgent --to user` to ask the human.

## Keeping prompts honest (no tricks)

These agents are given **straight, well-scoped tasks** — never jailbreaks,
role-play coercion, or manipulation aimed at bypassing a model's rules. That
kind of prompting is unnecessary here and risks the account. The work
(implementing an original OS, porting apps, reading a source tree you already
hold) is legitimate and is described honestly; honest framing is what gets good
results and keeps the subscriptions safe.
