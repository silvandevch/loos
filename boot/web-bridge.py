#!/usr/bin/env python3
"""QEMU web bridge for the LoOS Browser (development only).

The guest browser speaks a tiny compressed protocol over COM2:

    WGET <url>\\n
    OK <compressed_len> <raw_len> <final_url>\\n<raw DEFLATE bytes>
    ERR <reason>\\n

This helper does the HTTPS request on the host (certificate validation on), compresses the body with raw DEFLATE
and writes it to the emulated serial port. It exists only so the browser can render real pages in QEMU before the
native LoOS network stack lands. It is never part of a real-machine image.
"""
import os
import re
import socket
import sys
import urllib.error
import urllib.parse
import urllib.request
import time
import zlib

MAX_IN = 1 << 20
MAX_URL = 1400
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"


def read_line(f):
    out = bytearray()
    while len(out) < 8192:
        c = f.read(1)
        if not c:
            return None
        if c == b"\n":
            return bytes(out)
        if c != b"\r":
            out += c
    return None


def fetch(url):
    if len(url) > MAX_URL:
        raise ValueError("url too long")
    u = urllib.parse.urlsplit(url)
    if u.scheme not in ("http", "https"):
        raise ValueError("only http(s)")
    if u.username or u.password:
        raise ValueError("credentials not allowed")
    req = urllib.request.Request(url, headers={
        "User-Agent": UA,
        "Accept": "text/html,application/xhtml+xml,text/css;q=0.9,text/plain;q=0.8,*/*;q=0.5",
        "Accept-Language": "en-US,en;q=0.9",
        "Accept-Encoding": "identity",
        "Connection": "close",
    })
    with urllib.request.urlopen(req, timeout=25) as r:
        data = r.read(MAX_IN + 1)
        if len(data) > MAX_IN:
            data = data[:MAX_IN]
        final = r.geturl()
        ctype = r.headers.get("Content-Type", "")
    if "html" in ctype.lower():
        # Scripts are not executed and comments are invisible: dropping them cuts the serial transfer sharply.
        data = re.sub(rb"<script\b[^>]*>.*?</script\s*>", b"", data, flags=re.S | re.I)
        data = re.sub(rb"<!--.*?-->", b"", data, flags=re.S)
        data = re.sub(rb"<svg\b[^>]*>.*?</svg\s*>", b"", data, flags=re.S | re.I)
    if not final or len(final) > 1200 or any(c in final for c in " \t\r\n"):
        final = url
    return data, final, ctype


def compress(data):
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return c.compress(data) + c.flush()


def serve(sock_path):
    try:
        os.unlink(sock_path)
    except FileNotFoundError:
        pass
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(sock_path)
    srv.listen(1)
    os.chmod(sock_path, 0o600)
    print("web-bridge: listening on", sock_path, flush=True)
    while True:
        conn, _ = srv.accept()
        try:
            f = conn.makefile("rb", buffering=0)
            while True:
                line = read_line(f)
                if line is None:
                    break
                if not line.startswith(b"WGET "):
                    conn.sendall(b"ERR bad request\n")
                    continue
                url = line[5:].decode("utf-8", "replace").strip()
                t0 = time.time()
                try:
                    data, final, ctype = fetch(url)
                    body = compress(data)
                    hdr = ("OK %d %d %s\n" % (len(body), len(data), final)).encode("utf-8")
                    conn.sendall(hdr + body)
                    print("web-bridge: %.2fs %s -> %s, %d bytes, %s" % (time.time() - t0, url, final, len(data), ctype), flush=True)
                except Exception as e:  # noqa: BLE001 - report to the guest and keep serving
                    msg = str(e).replace("\n", " ")[:160]
                    conn.sendall(("ERR %s\n" % msg).encode("utf-8"))
                    print("web-bridge: %s failed: %s" % (url, msg), flush=True)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            try:
                conn.close()
            except OSError:
                pass


if __name__ == "__main__":
    path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/loos-web.sock"
    try:
        serve(path)
    except KeyboardInterrupt:
        pass
    finally:
        try:
            os.unlink(path)
        except FileNotFoundError:
            pass
