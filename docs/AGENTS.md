# LoOS — how the agents work here

Three AI agents build LoOS together. This page is the short version: who does
what, how they talk, and the rules. Read
[SOURCE-POLICY.md](SOURCE-POLICY.md) once, then work. Where to work:
[AGENT-WORKSPACE.md](AGENT-WORKSPACE.md). Budget: [BUDGET.md](BUDGET.md).

## The agents

| Bus id | Model | Strength | Owns |
|---|---|---|---|
| `astra` | **GPT 6 Astra** | hardest, correctness-critical | kernel, VM, syscalls, btrfs, drivers, novel security/exploit, hardest reverse-engineering, math |
| `sol` | **DeepSeek V4.1 Flash 1017** (thinking, `reasoning_effort=high`) | agentic coding in an existing repo — the strongest coder in the table | Win32 subsystem, API sets, compat shims, backend services, graphics bridge, Lin bridge/base, toolchain, tests, security *engineering* (crypto/TLS/signing/firewall) |
| `claude` | **Claude Sonnet 5.5** (medium effort) | fast and cheap; knowledge work, docs, design, review; computer use | UI/shell, design & branding, docs & writing, review, mechanical tables & CI, provenance |

Each tier's measured strengths (and the reasons for its scores) are in its
PART-1 prompt: `../prompts/<agent>/PART-1.md`.

Route by **capability in the task's category**, not by who's free. The chain is
**Claude → Sol → Astra**: hand *UI/design/docs/review/mechanical tables* down to
Claude, hand *novel security (exploit invention, hardest RE), the debug-heavy
core and math* up to Astra, and do the bulk implementation in between yourself.
Sol is the strongest agentic coder on repository-scale work (DeepSWE v1.1 77.0
vs Astra 74.1), but Astra still owns novelty and correctness under uncertainty.
Claude runs at medium effort: it is excellent on small, self-contained tasks,
long-form docs and review, and loses the most on long autonomous loops. See
§Routing below.

## Routing

Owners are computed, not hand-picked. `scripts/seed_tasks.py` holds a
per-category **competence matrix** for `(astra, sol, claude)` and a **load
balancer**: take the best model for a category unless it is past its cap, then
fall through to the next model whose competence still clears a threshold
(≥ 72). If nobody clears it, hard-core work (`HARD` categories: boot, kernel,
io, fs, drivers, net, crypto, security, power, time, virt, lin-core) stays with
the best model — hard work is never dumped on a poor fit — and volume work goes
to whoever has the most headroom. Caps are `astra 44 · sol 122 · claude 160`;
astra stays on the hard core, sol carries the workhorse volume, and Claude takes
the breadth it leads. (Hard tasks whose only competent owner is astra still stay
with astra even past its cap — that is why the computed count can exceed a
cap.)

| Category | astra | sol | claude | | Category | astra | sol | claude |
|---|--:|--:|--:|---|---|--:|--:|--:|
| boot | 95 | 84 | 62 | | ui | 76 | 84 | 91 |
| kernel | 96 | 84 | 62 | | design | 68 | 80 | 93 |
| io | 93 | 84 | 64 | | docs | 72 | 82 | 93 |
| fs | 94 | 82 | 64 | | review | 90 | 88 | 90 |
| drivers | 92 | 82 | 58 | | build | 86 | 92 | 85 |
| net | 94 | 82 | 64 | | tests | 88 | 91 | 84 |
| crypto / security | 98 / 95 | 80 / 82 | 64 / 65 | | ci | 84 | 90 | 89 |
| privacy | 90 | 84 | 78 | | porting | 78 | 92 | 82 |
| graphics | 90 | 84 | 66 | | env | 76 | 84 | 90 |
| pe | 90 | 90 | 78 | | lin-core | 95 | 84 | 66 |
| win32 | 88 | 90 | 80 | | lin-fs / lin-bridge | 90 / 88 | 84 / 86 | 70 / 74 |
| apisets | 80 | 92 | 82 | | lin-gui / lin-base | 82 / 78 | 84 / 90 | 82 / 82 |
| compat | 82 | 92 | 80 | | lin-tooling / lin-corpus | 80 / 78 | 91 / 88 | 84 / 88 |
| backend | 84 | 92 | 84 | | gen | 82 | 80 | 88 |

Categories added for the full-OS backlog (M11+, packaging, installer, SDK, compilers):

| Category | astra | sol | claude | | Category | astra | sol | claude |
|---|--:|--:|--:|---|---|--:|--:|--:|
| pkg | 84 | 93 | 86 | | power / time | 92 / 92 | 84 / 84 | 66 / 64 |
| installer | 90 | 90 | 82 | | audio / print | 88 / 84 | 84 / 88 | 72 / 80 |
| recovery | 88 | 88 | 82 | | virt | 94 | 84 | 64 |
| update | 86 | 93 | 84 | | a11y | 72 | 82 | 92 |
| rust | 90 | 94 | 84 | | sdk | 88 | 92 | 86 |
| compiler | 86 | 91 | 82 | | python | 80 | 93 | 82 |

Scores are grounded in public 2026 evals and are only used for relative
ranking (they must match `COMPETENCE` in `../scripts/seed_tasks.py`):
- **astra** — OpenAI launch table: FrontierMath T4 97.6, ExploitBench 100,
  ARC-AGI-3 99.9, SRE-Bench 88.0, Terminal-Bench 4.0 57.9, DeepSWE v1.1 74.1.
- **sol** — DeepSeek V4.1 Flash 1017 launch table (high effort): DeepSWE v1.1
  77.0, Terminal-Bench 4.0 42.3, Apex Bench 72.1, Automation-Bench 60.5, Agents'
  Last Exam 35.4, ProgramBench 29.1; V4.1-Flash release: CyberGym 88.1,
  SEC-Bench Pro 62.8, ExploitGym 15.3.
- **claude** — Claude Sonnet 5.5, effort-adjusted (Anthropic's effort table puts
  medium at ~half of max): Terminal-Bench 4.0 28.8 (max 70.6), FrontierCode 36.5
  (max 46.2), CursorBench 39.2 (max 55.5); system card SWE-Bench Pro 81.3,
  OSWorld 2.1 80.1, AA-Briefcase 1811–1844 Elo.

Current board split (318 tasks, after the reseed):
**astra 86 · sol 130 · claude 102**.

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
