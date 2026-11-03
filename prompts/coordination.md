# Multi-agent coordination

Three AI agents work this repo at the same time. They don't talk directly — they
exchange messages through the **agent bus** (a SQLite + TCP broker,
`agents/busd.py`) and pick them up when they run. This keeps them decoupled,
parallel, and auditable.

| Bus id | Agent | Model | Owns |
|---|---|---|---|
| `astra` | Codex #1 | **GPT 6 Astra** (strongest) | the hardest, correctness-critical core: kernel, VM, syscall ABI, btrfs/VFS, drivers, novel security/crypto, hardest reverse-engineering, math |
| `sol` | DeepSeek Harness | **DeepSeek V4.1 Flash 1017** (thinking, high effort) | the workhorse: Win32 subsystem, API sets, compat shims, backend services, graphics bridge, Lin bridge/base, toolchain, tests, security engineering |
| `claude` | Claude Code | **Claude Sonnet 5.5** (medium effort, fast + broad) | UI/shell, design & branding, docs & writing, **review of everything**, mechanical tables & CI, provenance |

**Capability routing:** `claude` → `sol` → `astra`. Push breadth/UI/design/docs/
review/mechanical sweeps down to `claude`; take novel security, hardest kernel/RE,
and math up to `astra`. Most implementation is `sol`'s — it is the strongest
agentic coder on repo-scale work; don't have anyone else redo it.

If a task turns out harder than the tier can do correctly, **escalate** to the
next tier up — with what you tried, what failed, and the blocker. Never sit on a
task you can't do right. Owners on the board are computed by
`../scripts/seed_tasks.py` (competence matrix + load balancer, see
`../docs/AGENTS.md` §Routing).

## The loop every agent runs

1. **Read your inbox** first: `./loos bus inbox --for <me> --unread`.
2. **Claim** before working — add/claim a task, or post a `claim` message
   (topic = the subsystem) so two agents don't touch the same files.
3. **Do the work** per the specs (`../docs/` is the source of truth;
   `BACKEND-SPEC.md`, `UI-SPEC.md`, `spec/*` add detail).
4. **Report** — post a short result: what changed, how to verify, and any task
   it unblocks. Reference the claim/message id in `--ref`.
5. **Hand off / escalate** if needed.
6. **Ack** messages you've consumed: `./loos bus ack <id> --by <me>`.

Idle session? `./loos bus watch --for <me>` blocks and prints new messages.

## Topics (fixed set — use these)

`boot` `kernel` `mm` `io` `fs` `btrfs` `registry` `ps` `security` `win32`
`apisets` `graphics` `compositor` `shell` `ui` `net` `compat` `build` `ci`
`docs` `branding` `review`

## Message conventions

- **subject**: imperative, ≤ 8 words ("demand paging for section objects").
- **body**: what, where (paths), acceptance test, and `spec-ref` if any.
- **priority**: `low` | `norm` | `high` | `urgent`. `urgent` only for a hard
  blocker or "someone is about to ship something wrong."
- **refs**: always `--ref` the message/task you're replying to or claiming.

```bash
# claude hands a hard problem up to sol
./loos bus send --from claude --to sol --topic mm --priority high \
  --subject "section object demand paging" \
  "spec: prompts/spec/mm-sections.md  acceptance: tests/mm/fault_test  ref BACKEND-SPEC.md §2.3"

# sol reports a batch done to claude
./loos bus send --from sol --to claude --topic apisets --subject "12 kernel32 forwarders" \
  "tables in src/apisets/kernel32.tab  acceptance: tests/apisets/resolve  ref <claimid>"

# sol escalates the hardest work to astra
./loos bus send --from sol --to astra --topic io --priority high \
  --subject "IRP cancellation race (escalating)" \
  "tried X, failed Y; see analysis in thread io. blocker: <…>"
```

## Task board

```bash
./loos bus task add "btrfs write path" --owner astra --priority high
./loos bus task ls [--state doing] [--owner sol]
./loos bus task claim 7 --by sol       # atomic — fails if already taken
./loos bus task update 7 --state review --notes "claude: review"
./loos bus task done 7 --by astra
```

States: `todo → doing → review → done` (+ `blocked`, `cancelled`).

## Conflict & review rules

- **One writer per file.** Claim first; if taken, message the claimant instead
  of editing.
- **Everything is reviewed** — `claude` owns review; kernel-critical diffs from
  `astra`/`sol` get a second look too. Request it on topic `review`.
- **Source discipline** ([`../docs/SOURCE-POLICY.md`](../docs/SOURCE-POLICY.md)):
  write original code; don't 1:1 copy unless there's genuinely no other way. If
  you copy, comment the exact source and add a `docs/PROVENANCE.md` row. If you
  borrow OSS, add a `THIRD-PARTY.md` row with its license.

## Shared state

The broker keeps durable state in `$AGENTBUS_DIR` (default `<repo>/.agentbus`,
`bus.db`). All agents share it, even from separate `LOOS_WORKDIR` folders.
Start the broker once per session: `./loos bus start`.

## Human in the loop

The user is identity `user`. Agents may post questions to `user`; the user
answers by posting back. `urgent` + `--to user` means "stop and ask a human."
