# kernel32 core (src/win32/kernel32, include/, ntdll/errmap.c)

kernel32 is a thin Win32 -> NT translation layer. It calls only the ntdll
`Nt*`/`Ldr*`/`Rtl*` surface declared in `src/win32/include/loos_nt.h`; the
kernel (or a test fake) implements those. All code is x64 Windows ABI
(`ms_abi` on the Linux host).

**Implemented.** Files: CreateFileW, ReadFile, WriteFile, CloseHandle,
DuplicateHandle, SetFilePointer(Ex), GetFileSizeEx, SetEndOfFile,
FlushFileBuffers, GetFileAttributesW, DeleteFileW, Create/RemoveDirectoryW,
GetFullPathNameW, Get/SetCurrentDirectoryW. Memory: VirtualAlloc/Free/Protect/Query.
Sync: events, mutexes, semaphores (named under `\BaseNamedObjects\`), Wait*,
Sleep(Ex), QueryPerformance*, GetTickCount64. Process/module: Get/SetLastError,
current process/thread ids, ExitProcess, GetModuleHandleW, LoadLibraryW,
FreeLibrary, GetProcAddress (via Ldr*). `RtlNtStatusToDosError` in ntdll/errmap.c.

**Translation rules worth knowing.**
- CreateFile: generic rights expand through the file generic mapping, then
  `|SYNCHRONIZE|FILE_READ_ATTRIBUTES`. Dispositions: CREATE_NEW->FILE_CREATE,
  CREATE_ALWAYS->OVERWRITE_IF, OPEN_EXISTING->OPEN, OPEN_ALWAYS->OPEN_IF,
  TRUNCATE_EXISTING->OVERWRITE (needs write access, else INVALID_PARAMETER
  before any syscall). Options: NON_DIRECTORY_FILE unless BACKUP_SEMANTICS;
  SYNCHRONOUS_IO_NONALERT unless OVERLAPPED; DELETE_ON_CLOSE adds DELETE access.
  CREATE_ALWAYS over an existing file and OPEN_ALWAYS on an existing one succeed
  with last-error ERROR_ALREADY_EXISTS; CREATE_NEW collision is ERROR_FILE_EXISTS.
- ReadFile at EOF is success with 0 bytes. SetFilePointer to a negative offset
  is ERROR_NEGATIVE_SEEK. Timeouts: ms -> negative 100ns relative; INFINITE -> NULL.
- Paths: see path.c header (verbatim `\\?\`, UNC, drive-relative, device names,
  dot trimming). NT form is `\??\...`.

**Added since v1.** Heaps (`ntdll/heap.c`: boundary-tag allocator, 16-byte aligned, coalescing, direct
regions for 256K+, pointer validation without dereferencing strays; `kernel32/heap.c`: Heap*, Local*,
Global*), code pages and strings (`strings.c`: UTF-8, 1252, 437, ASCII, Latin-1; compare/LCMap with simple
case folding for Latin-1/Latin-A/Greek/Cyrillic), process environment (`env.c`: command line, startup info,
std handles, sorted environment block, %VAR% expansion), misc (`misc.c`: MulDiv, IsBad*Ptr, system info,
FILETIME/SYSTEMTIME, UTC only), sync (`ntdll/sync.c`: WaitOnAddress over thread parking, critical sections,
SRW locks, condition variables; `kernel32/locks.c`: wrappers, InitOnce, Interlocked), TLS + FLS (`tls.c`).
The kernel owes two parking syscalls: `NtWaitForAlertByThreadId` / `NtAlertThreadByThreadId` (sticky alert).
`kernel32.dll` is now a pure forwarder DLL to `kernelbase.dll`, as on Windows 10, so API-set resolution to
kernelbase works. `docs/COVERAGE-win32.md` (generated) lists what real binaries still import that we lack.
Known gaps: no locale collation, UTC only, TlsFree/FlsFree clear only the calling thread's slot, SRW
readers can starve writers, a spurious wake restarts a timed wait.

**Limits / not yet.** Paths are capped at 16K WCHARs after NT conversion
(buffer halves) and 4K for GetFullPathName/SetCurrentDirectory (stack buffers,
no heap in kernel32 yet). No overlapped/async completion (OVERLAPPED only
supplies the offset on sync handles), no console handles, no heaps, no
CreateProcess/CreateThread, no FindFirstFile/MoveFile/CopyFile, named objects use
one global namespace (no per-session directory), no handle-inheritance beyond the
OBJ_INHERIT flag, process-wide cwd is a spinlock (no per-drive cwd table).
Real TEB/PEB offsets used for non-host builds (gs:[0x30]+0x68 last error, 0x40/0x48
client id, 0x60 PEB, +0x10 image base) are x64 Windows layout and untested on real hardware.

**Verify.** `tests/win32/run.sh` builds `tests/win32/kernel32` against a fake
ntdll (in-memory FS + objects) that records the exact NT arguments, under
ASan/UBSan. It tests translation, not kernel behaviour.

## Addendum: Rtl helpers, delay-load, FormatMessage
- ntdll `rtl.c`: counted strings, ANSI/Unicode (1252), case mapping (kernel32 wraps it), bitmaps, image-header helpers, time conversion.
- ntdll `delayload.c`: `LdrResolveDelayLoadedAPI`; kernelbase wraps it as `ResolveDelayLoadedAPI`.
- kernelbase `message.c`: `FormatMessageW/A` for FROM_STRING and FROM_SYSTEM (built-in English table, own wording), %n inserts with `!printf!` formats, array or va_list args, ALLOCATE_BUFFER (LocalFree). FROM_HMODULE not implemented: returns ERROR_MR_MID_NOT_FOUND.

## Addendum: file mapping
- kernelbase `mapping.c`: `CreateFileMappingW`, `OpenFileMappingW`, `MapViewOfFile[Ex]`, `UnmapViewOfFile`, `FlushViewOfFile`. ANSI variants belong to ansi.c (sol #72).
- New NT syscalls (provisional numbers 45-49 in syscalls.tbl; astra owns the real ABI): `NtCreateSection`, `NtOpenSection`, `NtMapViewOfSection`, `NtUnmapViewOfSection`, `NtFlushVirtualMemory`.
- Contract: view offsets are 64 KiB granular (else ERROR_MAPPED_ALIGNMENT); length 0 maps to the section end; a read-only section refuses a writable view (ACCESS_DENIED); `INVALID_HANDLE_VALUE` + size = anonymous section; named sections share via `\BaseNamedObjects`.
- `k32_object_attrs` now accepts `Local\` / `Global\` prefixes (single session: both map to the same directory), which also fixes named events/mutexes/semaphores.

## Addendum: ntdll printf family
- `ntdll/printf.c`: `sprintf swprintf _snprintf _snwprintf _vsnprintf _vsnwprintf vsprintf vswprintf` plus the `_s` forms, exported by plain name via `extra_exports.txt`. msvcrt semantics: `%s` follows the format width, `%S/%ws/%hs` flip it, `%ld` is 32-bit, `%I64d`/`%Ix`, `%p` = 16 upper-case hex digits; legacy `_snprintf` returns -1 on truncation and NUL-terminates only if it fits.
- Floating point is exact (big-number expansion, round-half-even); verified digit for digit against glibc over random bit patterns (printf_test.c). inf/nan print UCRT style (`inf`, `nan`); `%n` is ignored; `%a` is not implemented.
- Varargs on the Linux host: `LOOS_VA_LIST`/`LOOS_VA_START` in loos_nt.h (ms_va_list there); host builds expose the functions as `loos_<name>` so they do not collide with libc.
