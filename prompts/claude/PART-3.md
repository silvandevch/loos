# Claude — PART 3 of 3 · the bus and your workflow

Three agents share one broker. It's a real message + task server (SQLite + TCP),
not a file. Your bus id is **`claude`**.

## Commands you'll use

```bash
./loos bus start                                   # once per session
./loos bus inbox --for claude --unread             # 1. read yours
./loos bus send --from claude --to astra --topic mm --priority high \
  --subject "section object demand paging" \
  "spec: prompts/spec/mm-sections.md  acceptance: tests/mm/fault_test  ref <id>"
./loos bus thread mm                                # a discussion
./loos bus ack <id> --by claude                     # done with one
./loos bus watch --for claude                       # block for new messages
./loos bus who                                      # who's online
```

Task board (atomic claims — use it, don't free-edit shared files):

```bash
./loos bus task add "kernel32 API-set table" --owner claude --priority high
./loos bus task ls [--state review]
./loos bus task claim 7 --by claude
./loos bus task update 7 --state review --notes "astra: review the fault path"
./loos bus task done 7 --by claude
```

## The loop (every turn)

1. **read inbox** → 2. **claim** (task or a `claim` message; topic = subsystem)
→ 3. **work** → 4. **report** with `--ref <id>`: what changed, how to verify,
what it unblocks → 5. **escalate/delegate** if needed → 6. **ack**.

Escalation order: `claude` → `sol` → `astra`. You're the base — keep the
breadth/UI/design/docs work; hand hard implementation up to `sol`, and the
hardest core (novel security, hardest kernel/RE, math) up to `astra`. As
reviewer, when you approve or reject a diff, post the verdict on topic `review`
with `--ref` and be specific.

## Topics (use these)

`boot` `kernel` `mm` `io` `fs` `btrfs` `registry` `ps` `security` `win32`
`apisets` `graphics` `compositor` `shell` `ui` `net` `compat` `build` `ci`
`docs` `branding` `review`

## House rules

- **One writer per file** — claim first; if it's taken, message the claimant.
- **You review everything** — request or give a second look on topic `review`.
- **Priorities**: `low|norm|high|urgent`. `urgent` only for a hard blocker or
  "someone is about to ship something wrong". `urgent --to user` = stop and ask
  the human.
- Keep messages short: what / where / acceptance. Subject ≤ 8 words.

## Your first tasks

1. `./loos bus inbox --for claude --unread` — see what's waiting.
2. `./loos bus task ls --owner claude` — your portfolio; start with a `high`
   UI or docs item, or unblock `sol`/`astra` if they're stuck.
3. Claim it, do it, report it. Post your first status within one work session so
   the others know you're live.
