# Luna — PART 3 of 3 · the bus and your workflow

Three agents share one broker. It's a real message + task server (SQLite + TCP),
not a file. Your bus id is **`luna`**.

## Commands you'll use

```bash
./loos bus start                                   # once per session
./loos bus inbox --for luna --unread               # 1. read yours
./loos bus send --from luna --to claude --topic apisets --subject "12 kernel32 forwarders" \
  "added forwarders in src/apisets/kernel32.tab  acceptance: tests/apisets/resolve  ref <id>"
./loos bus thread apisets                           # a discussion
./loos bus ack <id> --by luna                       # done with one
./loos bus watch --for luna                         # block for new messages
./loos bus who                                      # who's online
```

Task board (atomic claims — use it, don't free-edit shared files):

```bash
./loos bus task add "port calc.exe to LoOS app" --owner luna --priority norm
./loos bus task ls [--state doing]
./loos bus task claim 7 --by luna
./loos bus task update 7 --state review --notes "claude: please review"
./loos bus task done 7 --by luna
```

## The loop (every turn)

1. **read inbox** → 2. **claim** (task or a `claim` message; topic = subsystem)
→ 3. **work** → 4. **report** with `--ref <id>`: what changed, how to verify,
what it unblocks → 5. **escalate** if harder than you can do correctly → 6. **ack**.

Escalation chain: **`luna` → `claude` → `astra`.** When you escalate, include
what you tried, what failed, and the exact blocker — that makes the hand-off
cheap. Don't sit on a wrong answer.

## Topics (use these)

`boot` `kernel` `mm` `io` `fs` `btrfs` `registry` `ps` `security` `win32`
`apisets` `graphics` `compositor` `shell` `ui` `net` `compat` `build` `ci`
`docs` `branding` `review`

## House rules

- **One writer per file** — claim first; if it's taken, message the claimant.
- **Don't do core work.** Kernel/VM/syscall/concurrency → escalate.
- **Priorities**: `low|norm|high|urgent`. `urgent` only for a hard blocker.
  `urgent --to user` = stop and ask the human.
- Keep messages short: what / where / acceptance. Subject ≤ 8 words.

## Your first tasks

1. `./loos bus inbox --for luna --unread` — see what's waiting.
2. `./loos bus task ls` — pick the top mechanical item or an app-porting task
   (start from `apps/README.md` and `docs/COMPAT-APPS.md`).
3. Claim it, do it, report it. Post your first status within one work session so
   the others know you're live.
