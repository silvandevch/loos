# LoOS — the LoOS API (the SDK)

The LoOS API is LoOS's own native interface — the equivalent of `windows.h`, but
**easier to learn, more powerful, and fully documented**. Our shell, services,
apps, package managers and tools are built on it. It is separate from the Win32
compat surface (`<loos/win32.h>`) that exists only to run unmodified Windows
programs.

## Design goals

1. **Ultra easy to learn.** One predictable pattern everywhere. If you know one
   `loos_*` call you can guess the rest. No hidden macros, no surprises, no
   "read the source to find out".
2. **Ultra powerful.** Real handles, real async, real shared memory, real
   kernel objects — not a toy wrapper. Nothing the OS can do is unreachable.
3. **AI-friendly.** Deterministic names, explicit types, complete self-contained
   headers, and a **machine-readable manifest** so an AI can use the API
   correctly without guessing.
4. **Fully documented — every single symbol.** No public function, type, or
   constant ships without a complete doc block (see [API-STYLE.md](API-STYLE.md)).
   CI fails the build otherwise.

## One umbrella header

```c
#include <loos.h>          /* the stable core surface + everything below */
```

| Header | Surface |
|---|---|
| `<loos/base.h>` | types, status codes, handles, calling convention, versioning |
| `<loos/proc.h>` | processes, threads, jobs, scheduling hints |
| `<loos/fs.h>` | files, directories, mounts, btrfs subvolumes/snapshots |
| `<loos/mm.h>` | memory, mappings, shared sections |
| `<loos/net.h>` | sockets, DNS, the firewall/privacy gate |
| `<loos/registry.h>` | settings store (hive-backed) |
| `<loos/security.h>` | tokens, accounts, privileges, code signing |
| `<loos/gfx.h>` | display class, surfaces, the compositor client |
| `<loos/pkg.h>` | the package service (winget + apt + MSIX/MSI) |
| `<loos/console.h>` | terminal/stdio |
| `<loos/shell.h>` | shell integration, notifications, clipboard |
| `<loos/loc.h>` | localisation/locale |
| `<loos/win32.h>` | the NT/Win32 compat contract (for running Windows apps) |

Each header is **self-contained** (include it alone and it compiles) and lists
its friends, so an AI or a human can read exactly one file to learn one area.

## The pattern (learn once, use everywhere)

Every subsystem follows the same five moves:

```c
loos_<area>_open(...)   ->  loos_handle        /* get a handle             */
loos_<area>_<verb>(...) ->  loos_status        /* do something with it     */
loos_<area>_<query>(...)                        /* ask something            */
loos_<area>_<wait>(...)                         /* optional: block/async    */
loos_<area>_close(h)                            /* release it               */
```

- names are `loos_<area>_<verb_noun>` — lower snake case, no abbreviations,
- **opaque handles** everywhere (like `HANDLE`), never raw kernel pointers,
- **status returns**: `loos_status` (`LOOS_OK == 0`, negative on error), out
  params last,
- **one call = one job** (no mode flags that mean five things),
- **strings** are `loos_str` (length + UTF-8); wide strings only at the Win32
  boundary,
- **ownership and lifetime are stated in the docs** for every handle and buffer.

## Base contract

```c
#define LOOS_API_VERSION 0x00010000u   /* 1.0 */

typedef int32_t  loos_status;              /* 0 = ok, negative = error        */
typedef struct { uintptr_t h; } loos_handle;
typedef struct { size_t len; const char *ptr; } loos_str;   /* UTF-8, not NUL-terminated */

#define LOOS_CALL __attribute__((sysv_abi))  /* the one calling convention */

#define LOOS_OK            0
#define LOOS_ERR_INVALID   (-0x00000001)
#define LOOS_ERR_NO_MEMORY (-0x00000002)
#define LOOS_ERR_NOT_FOUND (-0x00000003)
/* ...the full enum is documented, one line per code, in <loos/base.h> */
```

## A fully documented call (what every symbol looks like)

```c
/**
 * loos_fs_open - open a file by path.
 *
 * Opens `path` and returns a handle usable with loos_fs_read / loos_fs_write.
 * Creates the file if LOOS_FS_CREATE is set. The handle is owned by the caller
 * and must be closed with loos_fs_close().
 *
 * @since      1.0
 * @threadsafe yes
 * @category   fs
 * @param path   NUL-terminated UTF-8 path. Must not be NULL.
 * @param mode   Any combination of LOOS_FS_READ, LOOS_FS_WRITE, LOOS_FS_CREATE,
 *               LOOS_FS_TRUNCATE.
 * @param out    Receives the handle on success. Must not be NULL.
 * @return LOOS_OK on success, otherwise one of:
 *   LOOS_ERR_NOT_FOUND  path does not exist (and LOOS_FS_CREATE not set)
 *   LOOS_ERR_ACCESS     permission denied
 *   LOOS_ERR_NO_MEMORY  out of memory
 * @example
 *   loos_handle f;
 *   if (loos_fs_open("/etc/loos.conf", LOOS_FS_READ, &f) == LOOS_OK) {
 *       loos_fs_close(f);
 *   }
 * @see loos_fs_read, loos_fs_write, loos_fs_close
 */
loos_status loos_fs_open(const char *path, uint32_t mode, loos_handle *out);
```

The doc block is **structured and machine-parseable**; the reference docs and the
AI manifest are generated from it. The exact format and rules are in
[API-STYLE.md](API-STYLE.md).

## ABI stability & versioning

- **Add-only within a major version**; removals/signature changes need a major
  bump. `LOOS_API_VERSION` and per-symbol `@since` let callers gate.
- **Symbol versioning** (ELF `.symver`) so an upgraded OS keeps old binaries
  working; the loader resolves the version a binary needs.
- The API is the boundary the [LANGUAGE.md](LANGUAGE.md) C-ABI rule refers to:
  C, Rust (`extern "C"`) and Python (`ctypes`) all call the same surface.

## Native format

LoOS native binaries are **ELF64** (`x86_64-loos`) linked against `libc-loos` +
the LoOS API — not PE. PE and Linux-ELF are handled by the loaders (Win32 compat
/ [LINUX-COMPAT.md](LINUX-COMPAT.md)). See [TOOLCHAINS.md](TOOLCHAINS.md).

## Relationship to `windows.h`

| | LoOS API | Win32 compat (`loos/win32.h`) |
|---|---|---|
| For | our own apps/services/tools | unmodified Windows apps |
| Style | clean, opaque handles, UTF-8 | NT/Win32 contract, `HANDLE`, UTF-16 |
| Stability | stable, versioned ABI | best-effort match to the NT contract |
| Format | ELF64 native | PE32/PE32+ |

The Win32 layer is internally a client of the LoOS API: `kernel32`/`ntdll`
translate Win32 calls onto LoOS API calls.

## Documentation is generated, complete, and enforced

- **`docs/api/`** — the full generated reference (one page per subsystem, one
  entry per symbol, examples included).
- **`api/loos.api.json`** — the machine-readable **manifest**: every symbol with
  signature, params, returns, error list, ownership, `@since`, category and
  example. This is what AI agents consume to call the API correctly.
- **`tests/api/examples/`** — every `@example` compiles and runs in CI (doctests).
- **CI gate `apidoc`**: if any public symbol lacks a complete doc block, the
  build **fails**. Documentation is not optional.

## Tasks

`sdk-api`, `sdk-style`, `sdk-docstd`, `sdk-apidoc`, `sdk-ailint`, `sdk-manifest`,
`sdk-examples`, `sdk-errors`, `sdk-search`, `sdk-aiguide`, `sdk-abi`, `sdk-pkg`,
`sdk-bindings`. See [API-STYLE.md](API-STYLE.md), [TOOLCHAINS.md](TOOLCHAINS.md),
[PYTHON.md](PYTHON.md).
