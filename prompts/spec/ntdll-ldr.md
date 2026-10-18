# ntdll loader (src/win32/ldr)

Host-testable core of `LdrLoadDll`, `LdrUnloadDll`, `LdrGetProcedureAddress`,
`GetModuleHandle`. All OS access goes through `ldr_backend` (open file, alloc
image memory, free, call an entry point). Real ntdll plugs in Nt* syscalls;
tests plug in malloc + a recording fake.

**Name rules.** Strip directory, lowercase, append `.dll` when there is no
extension (trailing `.` = none). `api-ms-`/`ext-ms-` names go through
`apiset_resolve`; unknown contracts fail `DLL_NOT_FOUND`. Loaded modules are
keyed by the resulting host name, so `api-ms-win-core-file-l1-2-0` and
`kernelbase` are the same module.

**Load.** open -> `pe_open` (machine must equal process bitness, EXE/DLL flag
must match) -> alloc at ImageBase (`fixed` if the image has no relocs) ->
`pe_map` -> `pe_relocate` if moved -> TLS index assignment -> bind imports.
Import binding loads each host module recursively (cycles are fine: a module is
visible as MAPPED before it binds) and follows export forwarders (`DLL.Func`,
`DLL.#N`, depth limit 16, forwarder targets become dependencies).

**Init.** Dependencies first (DFS, cycle-safe): TLS callbacks then
`DllMain(PROCESS_ATTACH)`. EXE entry is not called (`ldr_entry_point`). A FALSE
from DllMain fails the load with `INIT_FAILED`; DLLs attached earlier in the
same operation are detached in reverse order and everything unreachable is freed.

**Lifetime.** `explicit_refs` (LoadLibrary count) are the GC roots; a module is
live iff reachable from a root through import edges. Unload and failed loads run
mark-and-sweep, detaching in reverse init order. Import cycles are collected.

**Statuses.** `ldr_status_to_ntstatus` maps to the public ntstatus.h values
(DLL_NOT_FOUND C0000135, INVALID_IMAGE_FORMAT C000007B, PROCEDURE_NOT_FOUND
C000007A, ...).

**Not yet.** Delay-load binding (the PE layer can bind; the loader leaves delay
IATs alone until a helper exists), per-thread TLS block allocation (index and
callbacks are done, `pe_tls_init_block` gives the template), search paths /
SxS manifests / KnownDlls (backend's `open_module` policy), WoW64, load-order
lists in the PEB, `LOAD_LIBRARY_AS_DATAFILE`, ASLR policy, host modules the
schema names that LoOS doesn't ship (`kernel.appcore.dll`, `gdi32full.dll`, ...
need real modules or aliases).
Caveat: a forwarder target loaded after a failed op can stay BOUND-but-uninitialised
only until the next `ldr_get_proc` / load, which initialises all bound modules.
Verify: `tests/win32/run.sh` (ASan/UBSan; 5 scenarios incl. cycle, DllMain failure, rebase, 32-bit, TLS).
