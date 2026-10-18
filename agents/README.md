# LoOS Agent Bus

A real message + task broker so **Astra**, **Claude** and **Sol** coordinate
properly — durable messages, topics/threads, read-receipts, long-poll
subscriptions, presence, and a task board with atomic claims.

No dependencies (Python stdlib only). No network exposure — it listens on
`127.0.0.1` by default.

## Components

| File | What |
|---|---|
| `busd.py` | the broker: SQLite storage + TCP server (newline-JSON) |
| `client.py` | `Bus` class to use from agent code |
| `agentbus` | the CLI |
| `bus.db` | the durable store (in `.agentbus/`) |

## Start / stop

```bash
./loos bus start        # start the broker (background)
./loos bus status       # up? who's online? active tasks?
./loos bus stop
# or run it in the foreground:
python3 agents/busd.py
```

The client auto-starts the broker if it isn't running, so `./loos bus send …`
just works.

## Messages

```bash
./loos bus send --from astra --to claude --topic kernel --priority high \
                --subject "section fault path" "spec: docs/… acceptance: …"
./loos bus inbox --for claude --unread
./loos bus read 12
./loos bus ack 12 --by claude
./loos bus thread kernel            # full conversation on a topic
./loos bus watch --for claude       # block + print new messages (auto-ack)
./loos bus who                      # who's online
```

Message fields: `from`, `to` (`any` = broadcast), `topic`, `subject`, `body`,
`priority` (`low|norm|high|urgent`), `refs` (`--ref` a prior id/claim), `thread`.

## Tasks (the board)

```bash
./loos bus task add "btrfs write path" --owner astra --priority high
./loos bus task ls [--state doing] [--owner sol]
./loos bus task claim 3 --by sol     # atomic: fails if already taken
./loos bus task update 3 --state review --notes "needs claude review"
./loos bus task done 3 --by sol
```

States: `todo → doing → review → done` (plus `blocked`, `cancelled`).
`claim` is atomic, so two agents can't grab the same task.

## From agent code

```python
import sys; sys.path.insert(0, "agents")
from client import Bus
b = Bus("astra")
b.send(to="claude", topic="apisets", subject="CRT forwarders", body="…")
for m in b.inbox(unread_only=True):
    print(m["id"], m["subject"]); b.ack(m["id"])
b.task_claim(4)
```

## Config

| Env | Default | Meaning |
|---|---|---|
| `AGENTBUS_HOST` | `127.0.0.1` | bind/connect host |
| `AGENTBUS_PORT` | `47700` | port |
| `AGENTBUS_DIR` | `<repo>/.agentbus` | where `bus.db` lives |
| `AGENTBUS_DB` | `$AGENTBUS_DIR/bus.db` | explicit db path |

> `AGENTBUS_DIR` is also the **shared state** location — put it somewhere all
> agents can see if they run in separate sandboxes.

## Why not a plain file?

A file only lets you *append and poll*. This gives ordering, per-identity
inboxes, broadcast, read receipts, threads, long-poll (no busy-wait), presence,
and an atomic task board — so three agents can work the same repo without
colliding or stepping on each other.
