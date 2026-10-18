#!/usr/bin/env python3
"""
client.py — LoOS Agent Bus client library.

Talk to the broker (busd.py) over TCP with newline-delimited JSON. Import the
`Bus` class from agent code, or use the `agentbus` CLI.

    from client import Bus
    b = Bus("astra")
    b.send(to="claude", topic="kernel", subject="paging", body="see spec")
    for m in b.inbox(unread_only=True):
        print(m["id"], m["subject"])

The broker must be running (`./loos bus start` or `agentbus serve`).
"""
from __future__ import annotations
import json, os, socket, time
from pathlib import Path

DEFAULT_HOST = os.environ.get("AGENTBUS_HOST", "127.0.0.1")
DEFAULT_PORT = int(os.environ.get("AGENTBUS_PORT", "47700"))


class BusError(RuntimeError):
    pass


class Bus:
    def __init__(self, identity: str, host: str = DEFAULT_HOST, port: int = DEFAULT_PORT,
                 auto_start: bool = True):
        self.identity = identity
        self.host = host
        self.port = port
        if auto_start:
            self._ensure_server()

    # -- transport -----------------------------------------------------------
    def _rpc(self, op: str, timeout: float = 30.0, **kw):
        req = {"op": op, "from": self.identity, **kw}
        try:
            with socket.create_connection((self.host, self.port), timeout=timeout) as s:
                s.settimeout(timeout)
                s.sendall((json.dumps(req) + "\n").encode())
                buf = b""
                while not buf.endswith(b"\n"):
                    chunk = s.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
        except OSError as e:
            raise BusError(f"bus not reachable at {self.host}:{self.port} ({e}). "
                           f"Start it: ./loos bus start") from e
        if not buf.strip():
            raise BusError("empty response from bus")
        rep = json.loads(buf)
        if not rep.get("ok", False):
            raise BusError(rep.get("error", "unknown error"))
        return rep.get("data")

    def _ensure_server(self):
        try:
            with socket.create_connection((self.host, self.port), timeout=0.3):
                return
        except OSError:
            pass
        # try to launch busd.py (best effort)
        try:
            import subprocess, sys
            daemon = Path(__file__).resolve().parent / "busd.py"
            subprocess.Popen([sys.executable, str(daemon), "--host", self.host,
                              "--port", str(self.port)],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                             start_new_session=True)
            for _ in range(40):
                time.sleep(0.1)
                try:
                    with socket.create_connection((self.host, self.port), timeout=0.3):
                        return
                except OSError:
                    continue
        except Exception:
            pass
        raise BusError(f"could not start bus at {self.host}:{self.port}")

    # -- API -----------------------------------------------------------------
    def hello(self):
        return self._rpc("hello")

    def send(self, to: str, body: str, topic: str = "", subject: str = "",
             priority: str = "norm", refs=None, thread: str | None = None) -> str:
        return self._rpc("send", to=to, body=body, topic=topic, subject=subject,
                         priority=priority, refs=refs or [], thread=thread)["id"]

    def inbox(self, unread_only=False, limit=100, since=None, to=None):
        return self._rpc("inbox", for_identity=to or self.identity,
                         unread_only=unread_only, limit=limit, since=since)["messages"]

    def get(self, mid: str):
        return self._rpc("get", id=mid)["message"]

    def ack(self, mid: str):
        self._rpc("ack", id=mid, by=self.identity); return True

    def thread(self, topic: str):
        return self._rpc("thread", topic=topic)["messages"]

    def subscribe(self, timeout: float = 60.0):
        return self._rpc("subscribe", for_identity=self.identity, timeout=timeout)["messages"]

    def who(self):
        return self._rpc("presence")["online"]

    # tasks ------------------------------------------------------------------
    def task_add(self, title, owner="", state="todo", priority="norm", notes="", deps=None):
        return self._rpc("task_add", title=title, owner=owner, state=state,
                         priority=priority, notes=notes, deps=deps or [])["id"]

    def task_update(self, task_id, state=None, owner=None, notes=None):
        self._rpc("task_update", id=task_id, state=state, owner=owner, notes=notes); return True

    def task_claim(self, task_id):
        self._rpc("task_claim", id=task_id, by=self.identity); return True

    def task_list(self, state=None, owner=None):
        return self._rpc("task_list", state=state, owner=owner)["tasks"]


def main():  # tiny smoke test
    import sys
    b = Bus(sys.argv[1] if len(sys.argv) > 1 else "cli")
    print("connected as", b.identity, "| online:", ", ".join(b.who()) or "(none)")


if __name__ == "__main__":
    main()
