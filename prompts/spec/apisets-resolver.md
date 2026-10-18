# API-set resolver (src/win32/apisets)

**Contract.** `apiset_resolve(name, out, outsz)` maps an import name such as
`api-ms-win-core-file-l1-2-1.dll` to a LoOS host module (`kernelbase.dll`).

**Rule.** Case-insensitive; strip `.dll`; strip trailing `-<digits>` (the
revision, never affects the result); binary-search the remainder
(`api-ms-win-core-file-l1-2`) in the sorted schema table.

**Status codes.** `OK`, `NOT_AN_APISET` (no `api-ms-`/`ext-ms-` prefix, so the
caller treats it as a real DLL), `UNKNOWN` (prefixed but not in schema; loader
fails the import), `BAD_NAME` (empty, >127 chars, no revision), `BUFFER_TOO_SMALL`.

**Data.** `apiset.map` (contract -> host) is the source; `gen_table.py` emits
the sorted `apiset_table.c` (checked in). Tests assert sort order and that every
entry round-trips.

**Caveat.** The map is a best-effort schema written from memory of the Win10
surface. It has not been diffed against a real `apisetschema` and may list
contracts that don't exist or miss some. Host modules (kernelbase, ucrtbase,
combase, ...) are LoOS names and don't exist yet. Next: PE import binder calls
this before LoadLibrary-equivalent lookup.
