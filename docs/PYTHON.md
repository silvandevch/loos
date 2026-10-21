# LoOS — Python (native)

LoOS ships a **native Python interpreter** (`python3`) — not a toy, not a
sandbox: real CPython running on LoOS, using it for scripting, tooling, the
package ecosystem, and as the glue language for the OS and its apps.

## Goal

- `python3` runs Python 3 programs natively on LoOS, at real speed.
- `pip`, `venv`, `wheels` work, backed by the package service
  ([PACKAGING.md](PACKAGING.md)) so Python packages install the same way apt and
  winget packages do.
- CPython's **C extension modules** work, and `ctypes`/`cffi` can call the
  [LoOS API](API.md) directly.
- An **embeddable** `libpython` so LoOS apps/tools can host scripts.

## How (preferred: build CPython for LoOS)

Build CPython as a **native LoOS binary** against `libc-loos` (see
[TOOLCHAINS.md](TOOLCHAINS.md)) and target `x86_64-loos`:

```
x86_64-loos-gcc  <- build CPython with this base C library
   -> python3 (ELF, LoOS API)
      -> stdlib (pure-Python OK; C modules over libc-loos)
      -> pip / venv  (package service)
```

The work is mostly the **POSIX surface** CPython needs: threads/`pthread`,
`subprocess`/`fork`+`exec`, `mmap`, `select`/`epoll`-equivalent, `socket`,
`dlopen`, `os`/`dirent`/`stat`, `time`/`datetime`, `termios`, `signal`, and
`locale`. The Linux syscall compat layer ([LINUX-COMPAT.md](LINUX-COMPAT.md))
already provides much of this shape; the native build binds CPython to
`libc-loos` on top of the LoOS API instead.

**Interim fallback:** run the stock Ubuntu CPython via the Lin layer
(`L3`/`L5`) before the native build lands — same interpreter, just through the
Linux ABI. Either way the user sees a working `python3`.

## Packaging (privacy-preserving)

- `pip install` uses wheels; the default index is a **local/LAN mirror or an
  offline wheelhouse** so installs need not touch the public internet. If a
  public index is used, it is a named source visible in the firewall/LED
  ([PACKAGING.md](PACKAGING.md)).
- No telemetry: CPython's own none, and pip is configured with no usage
  reporting. Everything routes through the package trust gate.

## Integration with LoOS

- **LoOS API bindings**: a `loos` Python module (via `ctypes`, then `cffi`)
  exposing the same handles/status codes as [API.md](API.md).
- **Embedding**: `libpython` lets the shell, installer and services run `.py`
  hooks (`python-embed`).
- **Scripting points**: build hooks, package recipes, installer answer files,
  and automation use Python by default.

## Tasks

`python` (interpreter), `python-stdlib` (stdlib + C modules), `python-pip`
(pip/venv/wheels via the package service), `python-capi` (ctypes/cffi → LoOS
API), `python-embed` (libpython). Depends on `libc-loos`; interim path uses
`L3`/`L5`.
