# LoOS — API authoring & documentation standard

Rules for anyone (human or AI) adding to the [LoOS API](API.md). The goals:
**ultra easy to learn**, **ultra powerful**, **AI-friendly**, and **every single
symbol fully documented**. This file is the contract; the tooling in
[API.md](API.md#documentation-is-generated-complete-and-enforced) enforces it.

## 1. Naming (make it guessable)

- Functions: `loos_<area>_<verb>_<noun>` — `loos_fs_read`, `loos_proc_create`.
- Types: `loos_<noun>` — `loos_handle`, `loos_status`.
- Constants/flags: `LOOS_<AREA>_<FLAG>` — `LOOS_FS_READ`.
- Errors: `LOOS_ERR_<CONDITION>` — `LOOS_ERR_NOT_FOUND`.
- No abbreviations, no `p`/`lp`/`dw` hungarian, no `_t` suffixes.
- The verb set is fixed: `open/close/create/destroy/get/set/query/wait/cancel`
  mean the same thing in every area. If a new verb is really needed, it is added
  here first and used consistently.

## 2. Shape (make it predictable)

- **Open returns a handle; close releases it.** Every resource has a handle.
- **One call = one job.** No `flags` that silently change the whole meaning.
- **Errors come back as `loos_status`**, out-params last, never via `errno`.
- **Opaque handles** only; never expose kernel or internal struct layouts.
- **Self-contained headers**: each `<loos/area.h>` compiles alone, includes what
  it needs, and lists its friends.
- **No hidden allocation or ownership**: if a call allocates or takes ownership,
  the doc says so and says how to free it (`loos_<area>_close` / `loos_free`).
- **Thread-safety is documented per call** — one of `yes`, `no`, `reentrant`.
- **Async is a first-class shape**: a `..._wait(handle, timeout)` or an event
  handle, chosen consistently; never a callback with an undocumented lifetime.

## 3. Every symbol is documented — completely

No public function, type, constant, enum, or field ships without a doc block.
The block is structured and machine-parseable:

```c
/**
 * loos_net_connect - open a TCP connection.
 *
 * @since      1.0
 * @threadsafe yes
 * @category   net
 * @param host   NUL-terminated UTF-8 hostname or IP. Must not be NULL.
 * @param port   TCP port, host byte order.
 * @param out    Receives the socket handle. Must not be NULL.
 * @return LOOS_OK, or LOOS_ERR_NOT_FOUND / LOOS_ERR_ACCESS / LOOS_ERR_NO_MEMORY.
 * @ownership   Caller closes `*out` with loos_net_close().
 * @example
 *   loos_handle s;
 *   if (loos_net_connect("example.internal", 443, &s) == LOOS_OK) { ... }
 * @see loos_net_send, loos_net_recv, loos_net_close
 */
```

Required fields for a **function**: one-line summary, `@since`, `@threadsafe`,
`@category`, `@param` for each parameter, `@return` with the **full** error list,
`@ownership`, `@example`, `@see`. For a **type/constant/enum**: one-line summary,
`@since`, and the meaning of every field or value.

Rules:
- The summary is a real sentence a beginner can act on — not "does the thing".
- Every error listed is one the call can actually return.
- Every `@example` is minimal, compiles, and runs (it is a doctest).
- Ownership and lifetime are always stated, never implied.

## 4. Documentation tooling (generated + enforced)

| Tool | Produces |
|---|---|
| `tools/apidoc` | `docs/api/` reference (Markdown, one page per subsystem) |
| `tools/apiman` | `api/loos.api.json` machine-readable manifest for AI + tools |
| `tools/apilint` | CI gate: **fails** if any public symbol is under-documented |
| `tests/api/examples/` | compiles + runs every `@example` (doctests) |

The **manifest** (`api/loos.api.json`) is the AI contract: for each symbol it
carries name, signature, category, `@since`, params, returns, the full error
list, ownership, thread-safety and example. An AI reads the manifest instead of
guessing signatures — this is what makes the API *AI-friendly*.

## 5. AI-friendliness checklist

- Names are deterministic and area-prefixed → an agent can guess correctly.
- Types are explicit; no macro aliases that hide a function-like meaning.
- Headers are self-contained and small → an agent reads exactly one file.
- Errors are a closed, documented enum → an agent handles every case.
- The manifest gives signatures + examples → an agent calls without guessing.
- Examples form a tested corpus → an agent can pattern-match real usage.
- Nothing "obvious" is left undocumented: if a human must read the `.c` to use a
  symbol, the doc has failed the bar.

## 6. Review gate

A change touching `<loos/*.h>` is not done until: `apilint` passes (complete
docs), every new `@example` is a passing doctest, the manifest regenerates
clean, and the [API.md](API.md) header table is updated if a new area was added.

## Tasks

`sdk-style`, `sdk-docstd`, `sdk-apidoc`, `sdk-ailint`, `sdk-manifest`,
`sdk-examples`, `sdk-errors`, `sdk-search`, `sdk-aiguide`.
