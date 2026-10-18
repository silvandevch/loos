# Compatibility layer, ntdll glue, and the PE build

## Version claim (src/win32/compat, ntdll/version.c, kernel32/version.c)
Resolution: user profile > highest `<supportedOS>` in the manifest > db default
(Windows 11, 10.0.22631). The Win10/11 manifest GUID maps to the native era.
Eras: win7 6.1.7601 SP1, win8 6.2.9200, win8.1 6.3.9600, win10 10.0.19045,
win11 10.0.22631. `compat_db` is the store behind `Compat.getAppProfile /
setClaimedVersion` (text format, `compat_db_parse/dump`; persistence and IPC are the
caller's). ntdll mirrors the claim with `rtl_set_version`; `ldr_api.c` does it from
the `exe_mapped` loader hook, i.e. after the image is mapped and bound but before any
DllMain runs. `RtlVerifyVersionInfo`: major, minor, SP major, SP minor are a
lexicographic walk where the first non-EQUAL condition governs and a differing field
decides alone; strict GREATER/LESS on equal values is false (the NT 5.2 code returned
success there); minor compares numerically. Legacy (pre-2000) condition masks are
not supported, only `VerSetConditionMask` style.

## Per-era compatibility shims
`src/win32/compat/era_shims.tab` is the X-macro table of version profiles and
feature bits. `compat_shim_profile_of` exposes each era's version and declared
shim features; `compat_shim_has` tests a feature. Current rows cover version
reporting, DPI awareness by era, and long-path awareness for Windows 10/11.
Keep this declarative table and its host tests in sync when adding compatibility
behavior. A feature bit records the selected profile capability; it does not
claim that an unimplemented Win32 API exists.

## Import redirects
`redirect <app|*> <dll>[!<sym>] <dll>[!<sym>]` rules, consulted by the loader for
every import via `ldr_backend.redirect_import`. Symbol rule: both sides name a symbol.
DLL rule: both sides bare, the symbol (or ordinal) is kept. These rules are
additional to the per-era feature table above and can be supplied by the user/profile.

## ntdll (src/win32/ntdll)
`syscalls.tbl` (provisional numbers; astra owns the real ABI, task #12) generates the
`mov r10,rcx; mov eax,N; syscall; ret` stubs. `ldr_api.c` implements LdrLoadDll,
LdrGetDllHandle, LdrGetProcedureAddress, LdrUnloadDll, LdrStartProcess,
RtlUserThreadStart over `src/win32/ldr` with an Nt*-based backend. Search path today:
explicit DOS path, else `C:\Windows\System32\`. Images map as RWX by copy (no W^X yet).

## Building real PE images
`scripts/win32/build-pe.sh [outdir]` compiles with `clang -target x86_64-pc-windows-msvc`
(freestanding, own `crt/string.c` + `chkstk.S`) and links with LLD in link mode
(`lld-link`, or the local Rust toolchain's `rust-lld -flavor link`). Exports are
generated from definitions marked WINAPI/NTAPI, so only implemented functions export.
Outputs `ntdll.dll`, `kernelbase.dll`, and the `kernel32.dll` forwarding image
(plus import libraries). Kernel32 ANSI entry points in `kernel32/ansi.c` convert
CP_ACP strings and delegate to W implementations; host coverage lives in
`win32.ansi_adapter_test`. `tests/win32/run.sh` builds the PE images,
loads them with the LoOS loader at a forced non-preferred address (relocation), checks
kernel32's imports are all satisfied by ntdll, and calls pure ntdll code
(RtlGetVersion, RtlVerifyVersionInfo, VerSetConditionMask, RtlNtStatusToDosError)
through the Windows x64 ABI on the Linux host. kernel32 itself is not executed on the
host (it reads the TEB through gs).

## Loader rule change
An image with no reloc table is movable unless `IMAGE_FILE_RELOCS_STRIPPED` is set
(position-independent DLLs such as kernel32 link with no `.reloc`).
