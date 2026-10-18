/* Win32 types, constants and the KERNEL32 surface implemented so far.
 * Values are the documented Win32 ABI. */
#ifndef LOOS_WIN_H
#define LOOS_WIN_H

#include "loos_nt.h"

typedef int BOOL;
typedef unsigned short WORD;
typedef unsigned char BYTE;
typedef const void *LPCVOID;
typedef void *LPVOID;
typedef const WCHAR *LPCWSTR;
typedef WCHAR *LPWSTR;
typedef DWORD *LPDWORD;
typedef HANDLE HMODULE;
typedef intptr_t INT_PTR;
typedef void *FARPROC;
#define INFINITE 0xFFFFFFFFu
#define INVALID_HANDLE_VALUE ((HANDLE)(intptr_t)-1)
#define INVALID_FILE_ATTRIBUTES ((DWORD)-1)
#define INVALID_SET_FILE_POINTER ((DWORD)-1)
#define MAX_PATH 260

typedef struct { DWORD nLength; LPVOID lpSecurityDescriptor; BOOL bInheritHandle; } SECURITY_ATTRIBUTES;
typedef struct {
    ULONG_PTR Internal, InternalHigh;
    union { struct { DWORD Offset, OffsetHigh; }; PVOID Pointer; };
    HANDLE hEvent;
} OVERLAPPED;

/* ---- error codes ---- */
#define ERROR_SUCCESS                0
#define ERROR_INVALID_FUNCTION       1
#define ERROR_BAD_LENGTH             24
#define ERROR_FILE_NOT_FOUND         2
#define ERROR_PATH_NOT_FOUND         3
#define ERROR_TOO_MANY_OPEN_FILES    4
#define ERROR_ACCESS_DENIED          5
#define ERROR_INVALID_HANDLE         6
#define ERROR_NOT_ENOUGH_MEMORY      8
#define ERROR_INVALID_DATA           13
#define ERROR_OUTOFMEMORY            14
#define ERROR_NOT_READY              21
#define ERROR_SHARING_VIOLATION      32
#define ERROR_HANDLE_EOF             38
#define ERROR_NOT_SUPPORTED          50
#define ERROR_FILE_EXISTS            80
#define ERROR_INVALID_PARAMETER      87
#define ERROR_BROKEN_PIPE            109
#define ERROR_DISK_FULL              112
#define ERROR_CALL_NOT_IMPLEMENTED   120
#define ERROR_INSUFFICIENT_BUFFER    122
#define ERROR_INVALID_NAME           123
#define ERROR_MOD_NOT_FOUND          126
#define ERROR_PROC_NOT_FOUND         127
#define ERROR_DIR_NOT_EMPTY          145
#define ERROR_NEGATIVE_SEEK          131
#define ERROR_BAD_EXE_FORMAT         193
#define ERROR_FILENAME_EXCED_RANGE   206
#define ERROR_BAD_PATHNAME           161
#define ERROR_ALREADY_EXISTS         183
#define ERROR_MORE_DATA              234
#define ERROR_TIMEOUT                1460
#define ERROR_NOACCESS               998
#define ERROR_NOT_OWNER              288
#define ERROR_TOO_MANY_POSTS         298
#define ERROR_OPERATION_ABORTED      995
#define ERROR_IO_PENDING             997
#define ERROR_PRIVILEGE_NOT_HELD     1314
#define ERROR_MR_MID_NOT_FOUND       317
#define ERROR_OLD_WIN_VERSION        1150
#define ERROR_DIRECTORY              267
#define ERROR_WAIT_NO_CHILDREN       128
#define ERROR_INVALID_ADDRESS        487
#define ERROR_COMMITMENT_LIMIT       1455

/* ---- file API constants ---- */
#define CREATE_NEW        1
#define CREATE_ALWAYS     2
#define OPEN_EXISTING     3
#define OPEN_ALWAYS       4
#define TRUNCATE_EXISTING 5
#define FILE_BEGIN   0
#define FILE_CURRENT 1
#define FILE_END     2
#define FILE_ATTRIBUTE_READONLY  0x00000001
#define FILE_ATTRIBUTE_HIDDEN    0x00000002
#define FILE_ATTRIBUTE_SYSTEM    0x00000004
#define FILE_ATTRIBUTE_DIRECTORY 0x00000010
#define FILE_ATTRIBUTE_ARCHIVE   0x00000020
#define FILE_ATTRIBUTE_NORMAL    0x00000080
#define FILE_ATTRIBUTE_TEMPORARY 0x00000100
#define FILE_FLAG_WRITE_THROUGH    0x80000000u
#define FILE_FLAG_OVERLAPPED       0x40000000u
#define FILE_FLAG_NO_BUFFERING     0x20000000u
#define FILE_FLAG_RANDOM_ACCESS    0x10000000u
#define FILE_FLAG_SEQUENTIAL_SCAN  0x08000000u
#define FILE_FLAG_DELETE_ON_CLOSE  0x04000000u
#define FILE_FLAG_BACKUP_SEMANTICS 0x02000000u
#define FILE_FLAG_OPEN_REPARSE_POINT 0x00200000u

/* ---- waits ---- */
#define WAIT_OBJECT_0     0x00000000u
#define WAIT_ABANDONED_0  0x00000080u
#define WAIT_IO_COMPLETION 0x000000C0u
#define WAIT_TIMEOUT      0x00000102u
#define WAIT_FAILED       0xFFFFFFFFu
#define MAXIMUM_WAIT_OBJECTS 64

/* ---- version ---- */
#define VER_EQUAL         1
#define VER_GREATER       2
#define VER_GREATER_EQUAL 3
#define VER_LESS          4
#define VER_LESS_EQUAL    5
#define VER_AND           6
#define VER_OR            7
#define VER_MINORVERSION     0x0000001
#define VER_MAJORVERSION     0x0000002
#define VER_BUILDNUMBER      0x0000004
#define VER_PLATFORMID       0x0000008
#define VER_SERVICEPACKMINOR 0x0000010
#define VER_SERVICEPACKMAJOR 0x0000020
#define VER_SUITENAME        0x0000040
#define VER_PRODUCT_TYPE     0x0000080
#define VER_NT_WORKSTATION       1
#define VER_NT_DOMAIN_CONTROLLER 2
#define VER_NT_SERVER            3
#define VER_PLATFORM_WIN32_NT    2
typedef RTL_OSVERSIONINFOW OSVERSIONINFOW;
typedef RTL_OSVERSIONINFOEXW OSVERSIONINFOEXW;
typedef struct { DWORD dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion, dwBuildNumber, dwPlatformId; char szCSDVersion[128]; } OSVERSIONINFOA;
typedef struct { DWORD dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion, dwBuildNumber, dwPlatformId; char szCSDVersion[128];
                 WORD wServicePackMajor, wServicePackMinor, wSuiteMask; BYTE wProductType, wReserved; } OSVERSIONINFOEXA;
DWORD WINAPI GetVersion(void);
BOOL  WINAPI GetVersionExW(OSVERSIONINFOW *info);
BOOL  WINAPI GetVersionExA(OSVERSIONINFOA *info);
BOOL  WINAPI VerifyVersionInfoW(OSVERSIONINFOEXW *info, DWORD type_mask, ULONGLONG condition_mask);

/* ---- last error ---- */
DWORD WINAPI GetLastError(void);
void  WINAPI SetLastError(DWORD e);

/* ---- files ---- */
HANDLE WINAPI CreateFileW(LPCWSTR name, DWORD access, DWORD share, SECURITY_ATTRIBUTES *sa,
                          DWORD disposition, DWORD flags_attrs, HANDLE template_file);
BOOL WINAPI ReadFile(HANDLE h, LPVOID buf, DWORD len, LPDWORD read, OVERLAPPED *ov);
BOOL WINAPI WriteFile(HANDLE h, LPCVOID buf, DWORD len, LPDWORD written, OVERLAPPED *ov);
BOOL WINAPI CloseHandle(HANDLE h);
BOOL WINAPI DuplicateHandle(HANDLE src_proc, HANDLE src, HANDLE dst_proc, HANDLE *dst, DWORD access, BOOL inherit, DWORD options);
BOOL WINAPI SetFilePointerEx(HANDLE h, LARGE_INTEGER dist, LARGE_INTEGER *newpos, DWORD method);
DWORD WINAPI SetFilePointer(HANDLE h, LONG dist, LONG *dist_high, DWORD method);
BOOL WINAPI GetFileSizeEx(HANDLE h, LARGE_INTEGER *size);
BOOL WINAPI SetEndOfFile(HANDLE h);
BOOL WINAPI FlushFileBuffers(HANDLE h);
DWORD WINAPI GetFileAttributesW(LPCWSTR name);
BOOL WINAPI DeleteFileW(LPCWSTR name);
BOOL WINAPI CreateDirectoryW(LPCWSTR name, SECURITY_ATTRIBUTES *sa);
BOOL WINAPI RemoveDirectoryW(LPCWSTR name);
DWORD WINAPI GetFullPathNameW(LPCWSTR name, DWORD len, LPWSTR buf, LPWSTR *file_part);
DWORD WINAPI GetCurrentDirectoryW(DWORD len, LPWSTR buf);
BOOL WINAPI SetCurrentDirectoryW(LPCWSTR dir);

/* ---- virtual memory ---- */
LPVOID WINAPI VirtualAlloc(LPVOID addr, SIZE_T size, DWORD type, DWORD protect);
BOOL WINAPI VirtualFree(LPVOID addr, SIZE_T size, DWORD type);
BOOL WINAPI VirtualProtect(LPVOID addr, SIZE_T size, DWORD newprot, DWORD *oldprot);
SIZE_T WINAPI VirtualQuery(LPCVOID addr, MEMORY_BASIC_INFORMATION *info, SIZE_T len);

/* ---- sync ---- */
HANDLE WINAPI CreateEventW(SECURITY_ATTRIBUTES *sa, BOOL manual_reset, BOOL initial, LPCWSTR name);
BOOL WINAPI SetEvent(HANDLE h);
BOOL WINAPI ResetEvent(HANDLE h);
HANDLE WINAPI CreateMutexW(SECURITY_ATTRIBUTES *sa, BOOL initial_owner, LPCWSTR name);
BOOL WINAPI ReleaseMutex(HANDLE h);
HANDLE WINAPI CreateSemaphoreW(SECURITY_ATTRIBUTES *sa, LONG initial, LONG maximum, LPCWSTR name);
BOOL WINAPI ReleaseSemaphore(HANDLE h, LONG count, LONG *prev);
DWORD WINAPI WaitForSingleObject(HANDLE h, DWORD ms);
DWORD WINAPI WaitForSingleObjectEx(HANDLE h, DWORD ms, BOOL alertable);
DWORD WINAPI WaitForMultipleObjects(DWORD n, const HANDLE *h, BOOL wait_all, DWORD ms);
DWORD WINAPI WaitForMultipleObjectsEx(DWORD n, const HANDLE *h, BOOL wait_all, DWORD ms, BOOL alertable);
void  WINAPI Sleep(DWORD ms);
DWORD WINAPI SleepEx(DWORD ms, BOOL alertable);
BOOL  WINAPI QueryPerformanceCounter(LARGE_INTEGER *c);
BOOL  WINAPI QueryPerformanceFrequency(LARGE_INTEGER *f);
ULONGLONG WINAPI GetTickCount64(void);

/* ---- process / module ---- */
HANDLE WINAPI GetCurrentProcess(void);
HANDLE WINAPI GetCurrentThread(void);
DWORD  WINAPI GetCurrentProcessId(void);
DWORD  WINAPI GetCurrentThreadId(void);
void   WINAPI ExitProcess(DWORD code);
HMODULE WINAPI GetModuleHandleW(LPCWSTR name);
HMODULE WINAPI LoadLibraryW(LPCWSTR name);
BOOL    WINAPI FreeLibrary(HMODULE m);
FARPROC WINAPI GetProcAddress(HMODULE m, const char *name);   /* name < 0x10000 = ordinal */

#endif
