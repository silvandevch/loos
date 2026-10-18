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
