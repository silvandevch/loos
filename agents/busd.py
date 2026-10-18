#!/usr/bin/env python3
"""
busd.py — LoOS Agent Bus broker.

A tiny, dependency-free message + task broker so the LoOS agents
(Astra / Claude / Luna) can coordinate properly: durable messages, topics,
threads, read-receipts, long-poll subscriptions, a task board with atomic
claims, and presence.

Storage: SQLite at $AGENTBUS_DB (default <repo>/.agentbus/bus.db).
Transport: newline-delimited JSON over TCP (default 127.0.0.1:47700).

Run it:
    ./loos bus start          # background via the CLI
    python3 agents/busd.py    # foreground
"""
from __future__ import annotations
import argparse, json, os, socket, sqlite3, threading, time
from datetime import datetime, timezone
from pathlib import Path
from socketserver import ThreadingTCPServer, StreamRequestHandler

PRIORITIES = {"low", "norm", "high", "urgent"}
TASK_STATES = {"todo", "doing", "blocked", "review", "done", "cancelled"}
WHO_TTL = 120  # seconds a heartbeat counts as "online"


def default_db() -> Path:
    root = os.environ.get("AGENTBUS_DIR")
    if root:
        return Path(root) / "bus.db"
    repo = Path(__file__).resolve().parent.parent
    return repo / ".agentbus" / "bus.db"


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


class Store:
    def __init__(self, db_path: Path):
        db_path.parent.mkdir(parents=True, exist_ok=True)
        self.lock = threading.RLock()
        self.db = sqlite3.connect(db_path, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("""CREATE TABLE IF NOT EXISTS messages(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT, sender TEXT, recipient TEXT, topic TEXT, subject TEXT,
            body TEXT, priority TEXT, status TEXT DEFAULT 'unread',
            refs TEXT, acked_by TEXT, acked_ts TEXT)""")
        self.db.execute("""CREATE TABLE IF NOT EXISTS tasks(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT, title TEXT, owner TEXT, state TEXT, priority TEXT,
            notes TEXT, deps TEXT, updated TEXT)""")
        self.db.commit()
        self.presence: dict[str, float] = {}

    # -- messages ------------------------------------------------------------
    def send(self, sender, to, body, topic="", subject="", priority="norm",
             refs=None, thread=None):
        if priority not in PRIORITIES:
            priority = "norm"
        with self.lock:
            cur = self.db.execute(
                "INSERT INTO messages(ts,sender,recipient,topic,subject,body,priority,refs)"
                " VALUES(?,?,?,?,?,?,?,?)",
                (now(), sender, to, topic or thread or "", subject, body, priority,
                 json.dumps(refs or [])))
            self.db.commit()
            return cur.lastrowid

    def _row(self, r):
        m = dict(r)
        m["refs"] = json.loads(m.get("refs") or "[]")
        return m

    def inbox(self, ident, unread_only=False, limit=100, since=None):
        q = "SELECT * FROM messages WHERE (recipient=? OR recipient='any')"
        args = [ident]
        if unread_only:
            q += " AND status='unread'"
        if since is not None:
            q += " AND id>?"
            args.append(since)
        q += " ORDER BY id ASC LIMIT ?"
        args.append(limit)
        with self.lock:
            return [self._row(r) for r in self.db.execute(q, args)]

    def get(self, mid):
        with self.lock:
            r = self.db.execute("SELECT * FROM messages WHERE id=?", (mid,)).fetchone()
        return self._row(r) if r else None

    def ack(self, mid, by):
        with self.lock:
            self.db.execute("UPDATE messages SET status='ack', acked_by=?, acked_ts=?"
                            " WHERE id=?", (by, now(), mid))
            self.db.commit()

    def thread(self, topic):
        with self.lock:
            rows = self.db.execute("SELECT * FROM messages WHERE topic=? ORDER BY id ASC",
                                   (topic,)).fetchall()
        return [self._row(r) for r in rows]

    # -- tasks ---------------------------------------------------------------
    def task_add(self, title, owner="", state="todo", priority="norm", notes="", deps=None):
        if state not in TASK_STATES:
            state = "todo"
        with self.lock:
            cur = self.db.execute(
                "INSERT INTO tasks(ts,title,owner,state,priority,notes,deps,updated)"
                " VALUES(?,?,?,?,?,?,?,?)",
                (now(), title, owner, state, priority, notes, json.dumps(deps or []), now()))
            self.db.commit()
            return cur.lastrowid

    def task_update(self, tid, state=None, owner=None, notes=None):
        sets, args = [], []
        if state is not None:
            if state not in TASK_STATES:
                raise ValueError(f"bad state {state}")
            sets.append("state=?"); args.append(state)
        if owner is not None:
            sets.append("owner=?"); args.append(owner)
        if notes is not None:
            sets.append("notes=?"); args.append(notes)
        if not sets:
            return
        sets.append("updated=?"); args.append(now()); args.append(tid)
        with self.lock:
            self.db.execute(f"UPDATE tasks SET {','.join(sets)} WHERE id=?", args)
            self.db.commit()

    def task_claim(self, tid, by):
        with self.lock:
            cur = self.db.execute(
                "UPDATE tasks SET state='doing', owner=?, updated=?"
                " WHERE id=? AND state='todo'", (by, now(), tid))
            self.db.commit()
            if cur.rowcount == 0:
                raise ValueError("task not claimable (already taken or not 'todo')")

    def task_list(self, state=None, owner=None):
        q, args = "SELECT * FROM tasks", []
        conds = []
        if state:
            conds.append("state=?"); args.append(state)
        if owner:
            conds.append("owner=?"); args.append(owner)
        if conds:
            q += " WHERE " + " AND ".join(conds)
        q += " ORDER BY CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1" \
             " WHEN 'norm' THEN 2 ELSE 3 END, id ASC"
        with self.lock:
            rows = self.db.execute(q, args).fetchall()
        out = []
        for r in rows:
            t = dict(r); t["deps"] = json.loads(t.get("deps") or "[]"); out.append(t)
        return out

    # -- presence ------------------------------------------------------------
    def beat(self, ident):
        with self.lock:
            self.presence[ident] = time.time()

    def online(self):
        t = time.time()
        with self.lock:
            return sorted(k for k, v in self.presence.items() if t - v < WHO_TTL)


class Handler(StreamRequestHandler):
    store: Store = None  # set on server

    def handle(self):
        line = self.rfile.readline()
        if not line:
            return
        try:
            req = json.loads(line)
            data = self.dispatch(req)
            rep = {"ok": True, "data": data}
        except Exception as e:  # noqa: BLE001 — surface any error to the client
            rep = {"ok": False, "error": f"{type(e).__name__}: {e}"}
        self.wfile.write((json.dumps(rep) + "\n").encode())

    def dispatch(self, req):
        op = req.get("op")
        s = self.store
        frm = req.get("from", "")
        if frm:
            s.beat(frm)
        if op == "hello":
            return {"you": frm, "server_time": now(), "online": s.online()}
        if op == "send":
            mid = s.send(frm, req["to"], req["body"], req.get("topic", ""),
                         req.get("subject", ""), req.get("priority", "norm"),
                         req.get("refs"), req.get("thread"))
            return {"id": mid}
        if op == "inbox":
            return {"messages": s.inbox(req.get("for_identity", frm),
                                        req.get("unread_only", False),
                                        req.get("limit", 100), req.get("since"))}
        if op == "get":
            return {"message": s.get(req["id"])}
        if op == "ack":
            s.ack(req["id"], req.get("by", frm)); return {"acked": req["id"]}
        if op == "thread":
            return {"messages": s.thread(req["topic"])}
        if op == "subscribe":
            ident = req.get("for_identity", frm)
            deadline = time.time() + float(req.get("timeout", 60))
            while True:
                msgs = s.inbox(ident, unread_only=True, limit=50)
                if msgs:
                    return {"messages": msgs}
                if time.time() >= deadline:
                    return {"messages": []}
                time.sleep(0.5)
        if op == "presence":
            return {"online": s.online()}
        if op == "task_add":
            tid = s.task_add(req["title"], req.get("owner", ""), req.get("state", "todo"),
                             req.get("priority", "norm"), req.get("notes", ""),
                             req.get("deps"))
            return {"id": tid}
        if op == "task_update":
            s.task_update(req["id"], req.get("state"), req.get("owner"), req.get("notes"))
            return {"updated": req["id"]}
        if op == "task_claim":
            s.task_claim(req["id"], req.get("by", frm)); return {"claimed": req["id"]}
        if op == "task_list":
            return {"tasks": s.task_list(req.get("state"), req.get("owner"))}
        raise ValueError(f"unknown op: {op}")


class Server(ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def run(host="127.0.0.1", port=47700, db=None):
    store = Store(Path(db) if db else default_db())
    Handler.store = store
    srv = Server((host, port), Handler)
    srv.serve_forever()


def main():
    ap = argparse.ArgumentParser(description="LoOS Agent Bus broker")
    ap.add_argument("--host", default=os.environ.get("AGENTBUS_HOST", "127.0.0.1"))
    ap.add_argument("--port", type=int, default=int(os.environ.get("AGENTBUS_PORT", "47700")))
    ap.add_argument("--db", default=os.environ.get("AGENTBUS_DB"))
    a = ap.parse_args()
    try:
        run(a.host, a.port, a.db)
    except OSError as e:
        print(f"busd: cannot bind {a.host}:{a.port}: {e}", file=os.sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
