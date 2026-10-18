# LoOS — how the agents work here

Three AI agents build LoOS together. This page is the short version: who does
what, how they talk, and the rules. Read
[SOURCE-POLICY.md](SOURCE-POLICY.md) once, then work. Where to work:
[AGENT-WORKSPACE.md](AGENT-WORKSPACE.md). Budget: [BUDGET.md](BUDGET.md).

## The agents

| Bus id | Model | Strength | Owns |
|---|---|---|---|
| `astra` | **GPT 6 Astra** (best) | hardest, correctness-critical | kernel, VM, syscalls, btrfs, drivers, novel security/crypto, hardest reverse-engineering, math |
| `sol` | **GPT 6.1 Sol** (near-Astra, cheaper) | the workhorse — matches Astra on complex coding | Win32 subsystem, API sets, compat shims, backend services, graphics bridge, Lin bridge/base, toolchain, tests |
| `claude` | **Claude Sonnet 5.5** | fast, broad, design/UI/docs | UI/shell, design & branding, docs & writing, review, mechanical tables & CI |

Route by **capability in the task's category**, not by who's free. The chain is
**Claude → Sol → Astra**: hand *UI/design/docs* down to Claude, hand *novel
security, hardest kernel/RE, and math* up to Astra, and do everything in between
yourself. Sol matches Astra on complex software engineering (DeepSWE v1.1) and
sits within ~2 points on computer use (OSWorld) at roughly a fifth of the cost;
Astra still leads on the hardest work. See §Routing below.

## Routing

Owners are computed, not hand-picked. `scripts/seed_tasks.py` holds a
per-category **competence matrix** for `(astra, sol, claude)` and a **load
balancer**: take the best model for a category unless it is past its cap, then
fall through to the next model whose competence still clears a threshold
(≥ 72). If nobody clears it, the best model keeps the task — hard work is never
dumped on a poor fit. Caps are `astra 44 · sol 122 · claude 128`; astra stays on
the hard core, sol carries the workhorse volume, and Claude is the sink for
breadth. (Hard tasks whose only competent owner is astra still stay with astra
even past its cap — that is why the computed count can exceed a cap.)

| Category | astra | sol | claude | | Category | astra | sol | claude |
|---|--:|--:|--:|---|---|--:|--:|--:|
| boot | 95 | 80 | 70 | | ui | 78 | 84 | 88 |
| kernel | 96 | 82 | 70 | | design | 70 | 80 | 90 |
| io | 93 | 80 | 72 | | docs | 74 | 80 | 90 |
| fs | 94 | 80 | 72 | | review | 90 | 86 | 88 |
| drivers | 92 | 78 | 65 | | build | 86 | 88 | 84 |
| net | 94 | 80 | 72 | | tests | 88 | 88 | 88 |
| crypto / security | 98 / 95 | 78 / 79 | 72 / 72 | | ci | 84 | 86 | 88 |
| privacy | 90 | 82 | 80 | | porting | 78 | 88 | 86 |
| graphics | 90 | 82 | 72 | | env | 78 | 84 | 84 |
| pe | 90 | 88 | 82 | | lin-core | 95 | 80 | 68 |
| win32 | 88 | 88 | 84 | | lin-fs / lin-bridge | 90 / 88 | 82 / 84 | 76 / 78 |
| apisets | 80 | 88 | 86 | | lin-gui / lin-base | 82 / 78 | 82 / 86 | 84 / 84 |
| compat | 82 | 88 | 84 | | lin-tooling / lin-corpus | 80 / 78 | 88 / 86 | 86 / 88 |
| backend | 84 | 86 | 82 | | | | | |

Categories added for the full-OS backlog (M11+, packaging / installer / system):

| Category | astra | sol | claude | | Category | astra | sol | claude |
|---|--:|--:|--:|---|---|--:|--:|--:|
| pkg | 84 | 90 | 88 | | power / time | 92 / 92 | 82 / 82 | 72 / 72 |
| installer | 90 | 86 | 84 | | audio / print | 88 / 84 | 82 / 86 | 78 / 84 |
| recovery | 88 | 86 | 84 | | virt | 94 | 80 | 70 |
| update | 86 | 90 | 86 | | a11y | 74 | 82 | 90 |
| rust | 90 | 92 | 86 | | | | |

Scores are grounded in public 2026 evals (FrontierMath, ExploitBench, SRE-Bench,
Terminal-Bench 4.0, DeepSWE v1.1, OSWorld 2.0/2.1, KernelGen, GDPval-AA) and are
only used for relative ranking. Current board split (307 tasks): **astra 50 ·
sol 129 · claude 128**.

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
- Sol: `../prompts/sol/PART-1.md` … `PART-3.md`

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
| what language? | [LANGUAGE.md](LANGUAGE.md) — **Rust-first**; C only where justified |
| our native API / SDK | [API.md](API.md) + [API-STYLE.md](API-STYLE.md) |
| compilers / targets | [TOOLCHAINS.md](TOOLCHAINS.md) — gcc/g++/clang/rust |
| Python | [PYTHON.md](PYTHON.md) — native interpreter |
| drivers / GPU (AMD, NVIDIA) | [DRIVERS.md](DRIVERS.md) |
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
