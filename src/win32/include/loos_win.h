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
typedef unsigned int UINT;
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
#define ERROR_GEN_FAILURE            31
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
#define ERROR_NOT_SAME_DEVICE        17
#define ERROR_BUFFER_OVERFLOW        111
#define ERROR_MORE_DATA              234
#define ERROR_TIMEOUT                1460
#define ERROR_NOACCESS               998
#define ERROR_NOT_OWNER              288
#define ERROR_TOO_MANY_POSTS         298
#define ERROR_OPERATION_ABORTED      995
#define ERROR_IO_PENDING             997
#define ERROR_PRIVILEGE_NOT_HELD     1314
#define ERROR_MR_MID_NOT_FOUND       317
#define ERROR_RESOURCE_NAME_NOT_FOUND 1814
#define ERROR_ENVVAR_NOT_FOUND       203
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
BOOL  WINAPI VerifyVersionInfoA(OSVERSIONINFOEXA *info, DWORD type_mask, ULONGLONG condition_mask);

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

/* ---- strings, code pages ---- */
typedef const char *LPCSTR; typedef char *LPSTR; typedef const WCHAR *LPCWCH; typedef BOOL *LPBOOL;
HANDLE WINAPI CreateFileA(LPCSTR name, DWORD access, DWORD share, SECURITY_ATTRIBUTES *sa,
                          DWORD disposition, DWORD flags_attrs, HANDLE template_file);
DWORD WINAPI GetFileAttributesA(LPCSTR name);
BOOL WINAPI DeleteFileA(LPCSTR name);
BOOL WINAPI CreateDirectoryA(LPCSTR name, SECURITY_ATTRIBUTES *sa);
BOOL WINAPI RemoveDirectoryA(LPCSTR name);
DWORD WINAPI GetFullPathNameA(LPCSTR name, DWORD len, LPSTR buf, LPSTR *file_part);
DWORD WINAPI GetCurrentDirectoryA(DWORD len, LPSTR buf);
BOOL WINAPI SetCurrentDirectoryA(LPCSTR dir);
#define CP_ACP 0
#define CP_OEMCP 1
#define CP_THREAD_ACP 3
#define CP_UTF8 65001
#define MB_PRECOMPOSED 1
#define MB_ERR_INVALID_CHARS 8
#define WC_ERR_INVALID_CHARS 0x80
#define WC_NO_BEST_FIT_CHARS 0x400
#define NORM_IGNORECASE 1
#define CSTR_LESS_THAN 1
#define CSTR_EQUAL 2
#define CSTR_GREATER_THAN 3
#define ERROR_INVALID_FLAGS 1004
#define ERROR_NO_UNICODE_TRANSLATION 1113
#define LCMAP_LOWERCASE 0x100
#define LCMAP_UPPERCASE 0x200
UINT WINAPI GetACP(void);
UINT WINAPI GetOEMCP(void);
BOOL WINAPI IsValidCodePage(UINT cp);
int  WINAPI MultiByteToWideChar(UINT cp, DWORD flags, LPCSTR src, int cb, LPWSTR dst, int cch);
int  WINAPI WideCharToMultiByte(UINT cp, DWORD flags, LPCWSTR src, int cch, LPSTR dst, int cb, LPCSTR defchar, LPBOOL used_default);
int  WINAPI CompareStringW(DWORD locale, DWORD flags, LPCWSTR a, int ca, LPCWSTR b, int cb);
int  WINAPI CompareStringA(DWORD locale, DWORD flags, LPCSTR a, int ca, LPCSTR b, int cb);
int  WINAPI CompareStringOrdinal(LPCWSTR a, int ca, LPCWSTR b, int cb, BOOL ignore_case);
int  WINAPI LCMapStringW(DWORD locale, DWORD flags, LPCWSTR src, int cch, LPWSTR dst, int cdst);
int  WINAPI LCMapStringA(DWORD locale, DWORD flags, LPCSTR src, int cch, LPSTR dst, int cdst);
int  WINAPI lstrcmpW(LPCWSTR a, LPCWSTR b);
int  WINAPI lstrcmpiW(LPCWSTR a, LPCWSTR b);
int  WINAPI lstrcmpA(LPCSTR a, LPCSTR b);
int  WINAPI lstrcmpiA(LPCSTR a, LPCSTR b);
int  WINAPI lstrlenW(LPCWSTR s);
int  WINAPI lstrlenA(LPCSTR s);
LPWSTR WINAPI lstrcpyW(LPWSTR d, LPCWSTR s);
LPSTR  WINAPI lstrcpyA(LPSTR d, LPCSTR s);
LPWSTR WINAPI lstrcpynW(LPWSTR d, LPCWSTR s, int n);
LPSTR  WINAPI lstrcpynA(LPSTR d, LPCSTR s, int n);
LPWSTR WINAPI lstrcatW(LPWSTR d, LPCWSTR s);
LPSTR  WINAPI lstrcatA(LPSTR d, LPCSTR s);

/* ---- process environment ---- */
#define STD_INPUT_HANDLE  ((DWORD)-10)
#define STD_OUTPUT_HANDLE ((DWORD)-11)
#define STD_ERROR_HANDLE  ((DWORD)-12)
typedef struct {
    DWORD cb; LPWSTR lpReserved, lpDesktop, lpTitle;
    DWORD dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
    WORD wShowWindow, cbReserved2; BYTE *lpReserved2;
    HANDLE hStdInput, hStdOutput, hStdError;
} STARTUPINFOW;
typedef struct {
    DWORD cb; LPSTR lpReserved, lpDesktop, lpTitle;
    DWORD dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
    WORD wShowWindow, cbReserved2; BYTE *lpReserved2;
    HANDLE hStdInput, hStdOutput, hStdError;
} STARTUPINFOA;
LPWSTR WINAPI GetCommandLineW(void);
LPSTR  WINAPI GetCommandLineA(void);
void   WINAPI GetStartupInfoW(STARTUPINFOW *si);
void   WINAPI GetStartupInfoA(STARTUPINFOA *si);
HANDLE WINAPI GetStdHandle(DWORD which);
BOOL   WINAPI SetStdHandle(DWORD which, HANDLE h);
DWORD  WINAPI GetEnvironmentVariableW(LPCWSTR name, LPWSTR buf, DWORD size);
BOOL   WINAPI SetEnvironmentVariableW(LPCWSTR name, LPCWSTR value);
LPWSTR WINAPI GetEnvironmentStringsW(void);
BOOL   WINAPI FreeEnvironmentStringsW(LPWSTR block);
LPSTR  WINAPI GetEnvironmentStringsA(void);
BOOL   WINAPI FreeEnvironmentStringsA(LPSTR block);
DWORD  WINAPI ExpandEnvironmentStringsW(LPCWSTR src, LPWSTR dst, DWORD size);
DWORD  WINAPI GetEnvironmentVariableA(LPCSTR name, LPSTR buf, DWORD size);
BOOL   WINAPI SetEnvironmentVariableA(LPCSTR name, LPCSTR value);
DWORD  WINAPI ExpandEnvironmentStringsA(LPCSTR src, LPSTR dst, DWORD size);
HMODULE WINAPI GetModuleHandleA(LPCSTR name);
HMODULE WINAPI LoadLibraryA(LPCSTR name);

/* ---- misc utility, time, system ---- */
typedef struct { WORD wYear, wMonth, wDayOfWeek, wDay, wHour, wMinute, wSecond, wMilliseconds; } SYSTEMTIME;
typedef struct { DWORD dwLowDateTime, dwHighDateTime; } FILETIME;
typedef struct {
    union { DWORD dwOemId; struct { WORD wProcessorArchitecture, wReserved; }; };
    DWORD dwPageSize; LPVOID lpMinimumApplicationAddress, lpMaximumApplicationAddress; ULONG_PTR dwActiveProcessorMask;
    DWORD dwNumberOfProcessors, dwProcessorType, dwAllocationGranularity; WORD wProcessorLevel, wProcessorRevision;
} SYSTEM_INFO;
#define PROCESSOR_ARCHITECTURE_AMD64 9
DWORD WINAPI GetTickCount(void);
int   WINAPI MulDiv(int number, int numerator, int denominator);
BOOL  WINAPI IsBadReadPtr(LPCVOID p, ULONG_PTR n);
BOOL  WINAPI IsBadWritePtr(LPVOID p, ULONG_PTR n);
BOOL  WINAPI IsBadStringPtrW(LPCWSTR p, ULONG_PTR max);
BOOL  WINAPI IsBadStringPtrA(LPCSTR p, ULONG_PTR max);
void  WINAPI GetSystemInfo(SYSTEM_INFO *si);
void  WINAPI GetNativeSystemInfo(SYSTEM_INFO *si);
void  WINAPI GetSystemTimeAsFileTime(FILETIME *ft);
void  WINAPI GetSystemTime(SYSTEMTIME *st);
void  WINAPI GetLocalTime(SYSTEMTIME *st);
BOOL  WINAPI FileTimeToSystemTime(const FILETIME *ft, SYSTEMTIME *st);
BOOL  WINAPI SystemTimeToFileTime(const SYSTEMTIME *st, FILETIME *ft);

/* ---- file mapping ---- */
#define FILE_MAP_COPY       0x0001
#define FILE_MAP_WRITE      0x0002
#define FILE_MAP_READ       0x0004
#define FILE_MAP_ALL_ACCESS 0x000F001F
#define FILE_MAP_EXECUTE    0x0020
#define ERROR_MAPPED_ALIGNMENT 1132
HANDLE WINAPI CreateFileMappingW(HANDLE file, SECURITY_ATTRIBUTES *sa, DWORD protect, DWORD size_hi, DWORD size_lo, LPCWSTR name);
HANDLE WINAPI OpenFileMappingW(DWORD access, BOOL inherit, LPCWSTR name);
HANDLE WINAPI CreateFileMappingA(HANDLE file, SECURITY_ATTRIBUTES *sa, DWORD protect, DWORD size_hi, DWORD size_lo, LPCSTR name);
HANDLE WINAPI OpenFileMappingA(DWORD access, BOOL inherit, LPCSTR name);
LPVOID WINAPI MapViewOfFile(HANDLE mapping, DWORD access, DWORD off_hi, DWORD off_lo, SIZE_T bytes);
LPVOID WINAPI MapViewOfFileEx(HANDLE mapping, DWORD access, DWORD off_hi, DWORD off_lo, SIZE_T bytes, LPVOID base);
BOOL   WINAPI UnmapViewOfFile(LPCVOID base);
BOOL   WINAPI FlushViewOfFile(LPCVOID base, SIZE_T bytes);

/* ---- FormatMessage ---- */
#define FORMAT_MESSAGE_ALLOCATE_BUFFER 0x00000100
#define FORMAT_MESSAGE_IGNORE_INSERTS  0x00000200
#define FORMAT_MESSAGE_FROM_STRING     0x00000400
#define FORMAT_MESSAGE_FROM_HMODULE    0x00000800
#define FORMAT_MESSAGE_FROM_SYSTEM     0x00001000
#define FORMAT_MESSAGE_ARGUMENT_ARRAY  0x00002000
#define FORMAT_MESSAGE_MAX_WIDTH_MASK  0x000000FF
DWORD WINAPI FormatMessageW(DWORD flags, LPCVOID source, DWORD msg_id, DWORD lang_id, LPWSTR buf, DWORD size, PVOID *args);
DWORD WINAPI FormatMessageA(DWORD flags, LPCVOID source, DWORD msg_id, DWORD lang_id, LPSTR buf, DWORD size, PVOID *args);

/* ---- file enumeration, attributes, copy/move ---- */
typedef struct {
    DWORD dwFileAttributes; FILETIME ftCreationTime, ftLastAccessTime, ftLastWriteTime; DWORD nFileSizeHigh, nFileSizeLow, dwReserved0, dwReserved1;
    WCHAR cFileName[260]; WCHAR cAlternateFileName[14];
} WIN32_FIND_DATAW;
typedef struct {
    DWORD dwFileAttributes; FILETIME ftCreationTime, ftLastAccessTime, ftLastWriteTime; DWORD nFileSizeHigh, nFileSizeLow, dwReserved0, dwReserved1;
    char cFileName[260]; char cAlternateFileName[14];
} WIN32_FIND_DATAA;
typedef struct { DWORD dwFileAttributes; FILETIME ftCreationTime, ftLastAccessTime, ftLastWriteTime; DWORD nFileSizeHigh, nFileSizeLow; } WIN32_FILE_ATTRIBUTE_DATA;
typedef struct {
    DWORD dwFileAttributes; FILETIME ftCreationTime, ftLastAccessTime, ftLastWriteTime; DWORD dwVolumeSerialNumber, nFileSizeHigh, nFileSizeLow, nNumberOfLinks, nFileIndexHigh, nFileIndexLow;
} BY_HANDLE_FILE_INFORMATION;
typedef enum { FindExInfoStandard = 0, FindExInfoBasic = 1 } FINDEX_INFO_LEVELS;
typedef enum { FindExSearchNameMatch = 0, FindExSearchLimitToDirectories = 1 } FINDEX_SEARCH_OPS;
typedef enum { GetFileExInfoStandard = 0 } GET_FILEEX_INFO_LEVELS;
#define MOVEFILE_REPLACE_EXISTING 0x1
#define MOVEFILE_COPY_ALLOWED 0x2
#define MOVEFILE_DELAY_UNTIL_REBOOT 0x4
#define MOVEFILE_WRITE_THROUGH 0x8
#define INVALID_FILE_SIZE ((DWORD)0xFFFFFFFF)
#define ERROR_NO_MORE_FILES 18
#define DRIVE_UNKNOWN 0
#define DRIVE_NO_ROOT_DIR 1
#define DRIVE_FIXED 3
HANDLE WINAPI FindFirstFileW(LPCWSTR pattern, WIN32_FIND_DATAW *data);
HANDLE WINAPI FindFirstFileA(LPCSTR pattern, WIN32_FIND_DATAA *data);
HANDLE WINAPI FindFirstFileExA(LPCSTR pattern, FINDEX_INFO_LEVELS info, LPVOID data, FINDEX_SEARCH_OPS op, LPVOID filter, DWORD flags);
HANDLE WINAPI FindFirstFileExW(LPCWSTR pattern, FINDEX_INFO_LEVELS info, LPVOID data, FINDEX_SEARCH_OPS op, LPVOID filter, DWORD flags);
BOOL   WINAPI FindNextFileW(HANDLE find, WIN32_FIND_DATAW *data);
BOOL   WINAPI FindNextFileA(HANDLE find, WIN32_FIND_DATAA *data);
BOOL   WINAPI FindClose(HANDLE find);
DWORD  WINAPI GetFileSize(HANDLE h, LPDWORD high);
BOOL   WINAPI GetFileInformationByHandle(HANDLE h, BY_HANDLE_FILE_INFORMATION *info);
BOOL   WINAPI GetFileAttributesExW(LPCWSTR name, GET_FILEEX_INFO_LEVELS level, LPVOID info);
BOOL   WINAPI GetFileAttributesExA(LPCSTR name, GET_FILEEX_INFO_LEVELS level, LPVOID info);
BOOL   WINAPI SetFileAttributesW(LPCWSTR name, DWORD attrs);
BOOL   WINAPI SetFileAttributesA(LPCSTR name, DWORD attrs);
BOOL   WINAPI GetFileTime(HANDLE h, FILETIME *created, FILETIME *accessed, FILETIME *written);
BOOL   WINAPI SetFileTime(HANDLE h, const FILETIME *created, const FILETIME *accessed, const FILETIME *written);
BOOL   WINAPI MoveFileExW(LPCWSTR from, LPCWSTR to, DWORD flags);
BOOL   WINAPI MoveFileW(LPCWSTR from, LPCWSTR to);
BOOL   WINAPI CopyFileW(LPCWSTR from, LPCWSTR to, BOOL fail_if_exists);
BOOL   WINAPI MoveFileExA(LPCSTR from, LPCSTR to, DWORD flags);
BOOL   WINAPI MoveFileA(LPCSTR from, LPCSTR to);
BOOL   WINAPI CopyFileA(LPCSTR from, LPCSTR to, BOOL fail_if_exists);
UINT   WINAPI GetTempFileNameW(LPCWSTR dir, LPCWSTR prefix, UINT unique, LPWSTR out);
UINT   WINAPI GetTempFileNameA(LPCSTR dir, LPCSTR prefix, UINT unique, LPSTR out);
UINT   WINAPI GetDriveTypeW(LPCWSTR root);
UINT   WINAPI GetDriveTypeA(LPCSTR root);
BOOL   WINAPI IsWow64Process(HANDLE process, BOOL *wow64);
UINT   WINAPI SetErrorMode(UINT mode);
PVOID  WINAPI ResolveDelayLoadedAPI(PVOID base, const void *descriptor, PVOID dll_hook, PVOID sys_hook, PVOID *thunk, ULONG flags);
FARPROC WINAPI DelayLoadFailureHook(LPCSTR dll, LPCSTR proc);

/* ---- threads, processes, paths ---- */
typedef DWORD (WINAPI *LPTHREAD_START_ROUTINE)(LPVOID);
typedef struct { HANDLE hProcess, hThread; DWORD dwProcessId, dwThreadId; } PROCESS_INFORMATION;
#define CREATE_SUSPENDED 0x00000004
#define DETACHED_PROCESS 0x00000008
#define CREATE_NEW_CONSOLE 0x00000010
#define CREATE_NEW_PROCESS_GROUP 0x00000200
#define CREATE_UNICODE_ENVIRONMENT 0x00000400
#define CREATE_NO_WINDOW 0x08000000
#define DEBUG_PROCESS 0x00000001
#define DEBUG_ONLY_THIS_PROCESS 0x00000002
#define STARTF_USESTDHANDLES 0x00000100
#define STACK_SIZE_PARAM_IS_A_RESERVATION 0x00010000
#define STILL_ACTIVE 259
#define THREAD_PRIORITY_ERROR_RETURN 0x7FFFFFFF
#define THREAD_PRIORITY_IDLE (-15)
#define THREAD_PRIORITY_TIME_CRITICAL 15
HANDLE WINAPI CreateThread(SECURITY_ATTRIBUTES *sa, SIZE_T stack, LPTHREAD_START_ROUTINE start, LPVOID param, DWORD flags, LPDWORD tid);
HANDLE WINAPI CreateRemoteThread(HANDLE process, SECURITY_ATTRIBUTES *sa, SIZE_T stack, LPTHREAD_START_ROUTINE start, LPVOID param, DWORD flags, LPDWORD tid);
void   WINAPI ExitThread(DWORD code);
BOOL   WINAPI GetExitCodeThread(HANDLE h, LPDWORD code);
DWORD  WINAPI SuspendThread(HANDLE h);
DWORD  WINAPI ResumeThread(HANDLE h);
BOOL   WINAPI TerminateThread(HANDLE h, DWORD code);
HANDLE WINAPI OpenThread(DWORD access, BOOL inherit, DWORD tid);
DWORD  WINAPI GetThreadId(HANDLE h);
BOOL   WINAPI SetThreadPriority(HANDLE h, int priority);
int    WINAPI GetThreadPriority(HANDLE h);
BOOL   WINAPI SwitchToThread(void);
BOOL   WINAPI GetExitCodeProcess(HANDLE h, LPDWORD code);
BOOL   WINAPI TerminateProcess(HANDLE h, UINT code);
HANDLE WINAPI OpenProcess(DWORD access, BOOL inherit, DWORD pid);
DWORD  WINAPI GetProcessId(HANDLE h);
BOOL   WINAPI CreateProcessW(LPCWSTR app, LPWSTR cmdline, SECURITY_ATTRIBUTES *psa, SECURITY_ATTRIBUTES *tsa, BOOL inherit,
                             DWORD flags, LPVOID env, LPCWSTR cwd, STARTUPINFOW *si, PROCESS_INFORMATION *pi);
BOOL   WINAPI CreateProcessA(LPCSTR app, LPSTR cmdline, SECURITY_ATTRIBUTES *psa, SECURITY_ATTRIBUTES *tsa, BOOL inherit,
                             DWORD flags, LPVOID env, LPCSTR cwd, STARTUPINFOA *si, PROCESS_INFORMATION *pi);
DWORD  WINAPI SearchPathW(LPCWSTR path, LPCWSTR name, LPCWSTR ext, DWORD len, LPWSTR buf, LPWSTR *file_part);
DWORD  WINAPI SearchPathA(LPCSTR path, LPCSTR name, LPCSTR ext, DWORD len, LPSTR buf, LPSTR *file_part);
UINT   WINAPI GetSystemDirectoryW(LPWSTR buf, UINT len);
UINT   WINAPI GetWindowsDirectoryW(LPWSTR buf, UINT len);
UINT   WINAPI GetSystemWindowsDirectoryW(LPWSTR buf, UINT len);
UINT   WINAPI GetSystemDirectoryA(LPSTR buf, UINT len);
UINT   WINAPI GetWindowsDirectoryA(LPSTR buf, UINT len);
UINT   WINAPI GetSystemWindowsDirectoryA(LPSTR buf, UINT len);
DWORD  WINAPI GetTempPathW(DWORD len, LPWSTR buf);
DWORD  WINAPI GetTempPathA(DWORD len, LPSTR buf);
DWORD  WINAPI GetModuleFileNameW(HMODULE m, LPWSTR buf, DWORD len);
DWORD  WINAPI GetModuleFileNameA(HMODULE m, LPSTR buf, DWORD len);
BOOL   WINAPI GetComputerNameW(LPWSTR buf, LPDWORD len);
BOOL   WINAPI GetComputerNameA(LPSTR buf, LPDWORD len);
LOOS_MSABI BOOL KernelBaseDllMain(HANDLE module, DWORD reason, LPVOID reserved);     /* loader entry point; deliberately not exported */

/* ---- console / file types ---- */
#include "loos_console.h"
typedef struct { int16_t X, Y; } COORD;
typedef struct { COORD dwSize, dwCursorPosition; WORD wAttributes; int16_t Left, Top, Right, Bottom; COORD dwMaximumWindowSize; } CONSOLE_SCREEN_BUFFER_INFO;
#define FILE_TYPE_UNKNOWN 0
#define FILE_TYPE_DISK 1
#define FILE_TYPE_CHAR 2
#define FILE_TYPE_PIPE 3
#define CTRL_C_EVENT 0
#define CTRL_BREAK_EVENT 1
#define CTRL_CLOSE_EVENT 2
typedef BOOL (WINAPI *PHANDLER_ROUTINE)(DWORD ctrl_type);
DWORD WINAPI GetFileType(HANDLE h);
BOOL WINAPI WriteConsoleW(HANDLE h, const void *buf, DWORD n, LPDWORD written, LPVOID reserved);
BOOL WINAPI WriteConsoleA(HANDLE h, const void *buf, DWORD n, LPDWORD written, LPVOID reserved);
BOOL WINAPI ReadConsoleW(HANDLE h, LPVOID buf, DWORD n, LPDWORD read, LPVOID input_control);
BOOL WINAPI ReadConsoleA(HANDLE h, LPVOID buf, DWORD n, LPDWORD read, LPVOID input_control);
BOOL WINAPI GetConsoleMode(HANDLE h, LPDWORD mode);
BOOL WINAPI SetConsoleMode(HANDLE h, DWORD mode);
BOOL WINAPI GetConsoleScreenBufferInfo(HANDLE h, CONSOLE_SCREEN_BUFFER_INFO *info);
BOOL WINAPI SetConsoleCursorPosition(HANDLE h, COORD pos);
BOOL WINAPI SetConsoleTextAttribute(HANDLE h, WORD attr);
BOOL WINAPI FillConsoleOutputCharacterW(HANDLE h, WCHAR ch, DWORD n, COORD at, LPDWORD written);
BOOL WINAPI FillConsoleOutputCharacterA(HANDLE h, char ch, DWORD n, COORD at, LPDWORD written);
BOOL WINAPI FillConsoleOutputAttribute(HANDLE h, WORD attr, DWORD n, COORD at, LPDWORD written);
UINT WINAPI GetConsoleCP(void);
UINT WINAPI GetConsoleOutputCP(void);
BOOL WINAPI SetConsoleCP(UINT cp);
BOOL WINAPI SetConsoleOutputCP(UINT cp);
BOOL WINAPI SetConsoleTitleW(LPCWSTR title);
BOOL WINAPI SetConsoleTitleA(LPCSTR title);
DWORD WINAPI GetConsoleTitleW(LPWSTR buf, DWORD n);
DWORD WINAPI GetConsoleTitleA(LPSTR buf, DWORD n);
BOOL WINAPI FlushConsoleInputBuffer(HANDLE h);
BOOL WINAPI GetNumberOfConsoleInputEvents(HANDLE h, LPDWORD count);   /* LoOS: counts pending input bytes */
BOOL WINAPI SetConsoleCtrlHandler(PHANDLER_ROUTINE routine, BOOL add);
DWORD WINAPI CtrlRoutine(LPVOID ctrl_type);         /* the console subsystem calls this on a fresh thread for Ctrl+C/Break/Close */
HANDLE WINAPI GetConsoleWindow(void);

/* ---- synchronisation ---- */
typedef RTL_CRITICAL_SECTION CRITICAL_SECTION;
typedef RTL_SRWLOCK SRWLOCK;
typedef RTL_CONDITION_VARIABLE CONDITION_VARIABLE;
typedef RTL_RUN_ONCE INIT_ONCE;
void WINAPI InitializeCriticalSection(CRITICAL_SECTION *cs);
BOOL WINAPI InitializeCriticalSectionAndSpinCount(CRITICAL_SECTION *cs, DWORD spin);
BOOL WINAPI InitializeCriticalSectionEx(CRITICAL_SECTION *cs, DWORD spin, DWORD flags);
void WINAPI DeleteCriticalSection(CRITICAL_SECTION *cs);
void WINAPI EnterCriticalSection(CRITICAL_SECTION *cs);
void WINAPI LeaveCriticalSection(CRITICAL_SECTION *cs);
BOOL WINAPI TryEnterCriticalSection(CRITICAL_SECTION *cs);
DWORD WINAPI SetCriticalSectionSpinCount(CRITICAL_SECTION *cs, DWORD spin);
void WINAPI InitializeSRWLock(SRWLOCK *l);
void WINAPI AcquireSRWLockExclusive(SRWLOCK *l);
void WINAPI AcquireSRWLockShared(SRWLOCK *l);
void WINAPI ReleaseSRWLockExclusive(SRWLOCK *l);
void WINAPI ReleaseSRWLockShared(SRWLOCK *l);
BOOLEAN WINAPI TryAcquireSRWLockExclusive(SRWLOCK *l);
BOOLEAN WINAPI TryAcquireSRWLockShared(SRWLOCK *l);
void WINAPI InitializeConditionVariable(CONDITION_VARIABLE *cv);
void WINAPI WakeConditionVariable(CONDITION_VARIABLE *cv);
void WINAPI WakeAllConditionVariable(CONDITION_VARIABLE *cv);
BOOL WINAPI SleepConditionVariableCS(CONDITION_VARIABLE *cv, CRITICAL_SECTION *cs, DWORD ms);
BOOL WINAPI SleepConditionVariableSRW(CONDITION_VARIABLE *cv, SRWLOCK *l, DWORD ms, ULONG flags);
BOOL WINAPI WaitOnAddress(volatile void *addr, PVOID compare, SIZE_T size, DWORD ms);
void WINAPI WakeByAddressSingle(PVOID addr);
void WINAPI WakeByAddressAll(PVOID addr);
void WINAPI InitOnceInitialize(INIT_ONCE *o);
BOOL WINAPI InitOnceBeginInitialize(INIT_ONCE *o, DWORD flags, BOOL *pending, PVOID *ctx);
BOOL WINAPI InitOnceComplete(INIT_ONCE *o, DWORD flags, PVOID ctx);
BOOL WINAPI InitOnceExecuteOnce(INIT_ONCE *o, BOOL (WINAPI *fn)(INIT_ONCE *, PVOID, PVOID *), PVOID param, PVOID *ctx);
LONG WINAPI InterlockedIncrement(volatile LONG *p);
LONG WINAPI InterlockedDecrement(volatile LONG *p);
LONG WINAPI InterlockedExchange(volatile LONG *p, LONG v);
LONG WINAPI InterlockedExchangeAdd(volatile LONG *p, LONG v);
LONG WINAPI InterlockedCompareExchange(volatile LONG *p, LONG exch, LONG cmp);
LONGLONG WINAPI InterlockedCompareExchange64(volatile LONGLONG *p, LONGLONG exch, LONGLONG cmp);

/* ---- thread-local storage ---- */
#define TLS_OUT_OF_INDEXES 0xFFFFFFFFu
#define FLS_OUT_OF_INDEXES 0xFFFFFFFFu
DWORD  WINAPI TlsAlloc(void);
BOOL   WINAPI TlsFree(DWORD index);
LPVOID WINAPI TlsGetValue(DWORD index);
BOOL   WINAPI TlsSetValue(DWORD index, LPVOID value);
DWORD  WINAPI FlsAlloc(void (WINAPI *callback)(PVOID));
BOOL   WINAPI FlsFree(DWORD index);
PVOID  WINAPI FlsGetValue(DWORD index);
BOOL   WINAPI FlsSetValue(DWORD index, PVOID value);

/* ---- heaps ---- */
#define LMEM_FIXED    0x0000
#define LMEM_MOVEABLE 0x0002
#define LMEM_ZEROINIT 0x0040
#define GMEM_FIXED    0x0000
#define GMEM_MOVEABLE 0x0002
#define GMEM_ZEROINIT 0x0040
typedef HANDLE HLOCAL, HGLOBAL;
HANDLE WINAPI HeapCreate(DWORD flags, SIZE_T initial, SIZE_T maximum);
BOOL   WINAPI HeapDestroy(HANDLE heap);
HANDLE WINAPI GetProcessHeap(void);
LPVOID WINAPI HeapAlloc(HANDLE heap, DWORD flags, SIZE_T size);
LPVOID WINAPI HeapReAlloc(HANDLE heap, DWORD flags, LPVOID mem, SIZE_T size);
BOOL   WINAPI HeapFree(HANDLE heap, DWORD flags, LPVOID mem);
SIZE_T WINAPI HeapSize(HANDLE heap, DWORD flags, LPCVOID mem);
BOOL   WINAPI HeapValidate(HANDLE heap, DWORD flags, LPCVOID mem);
SIZE_T WINAPI HeapCompact(HANDLE heap, DWORD flags);
BOOL   WINAPI HeapSetInformation(HANDLE heap, int info_class, PVOID info, SIZE_T len);
BOOL   WINAPI HeapQueryInformation(HANDLE heap, int info_class, PVOID info, SIZE_T len, SIZE_T *ret);
HLOCAL WINAPI LocalAlloc(UINT flags, SIZE_T size);
HLOCAL WINAPI LocalReAlloc(HLOCAL mem, SIZE_T size, UINT flags);
HLOCAL WINAPI LocalFree(HLOCAL mem);
SIZE_T WINAPI LocalSize(HLOCAL mem);
LPVOID WINAPI LocalLock(HLOCAL mem);
BOOL   WINAPI LocalUnlock(HLOCAL mem);
HGLOBAL WINAPI GlobalAlloc(UINT flags, SIZE_T size);
HGLOBAL WINAPI GlobalReAlloc(HGLOBAL mem, SIZE_T size, UINT flags);
HGLOBAL WINAPI GlobalFree(HGLOBAL mem);
SIZE_T WINAPI GlobalSize(HGLOBAL mem);
LPVOID WINAPI GlobalLock(HGLOBAL mem);
BOOL   WINAPI GlobalUnlock(HGLOBAL mem);

/* ---- virtual memory ---- */
LPVOID WINAPI VirtualAlloc(LPVOID addr, SIZE_T size, DWORD type, DWORD protect);
BOOL WINAPI VirtualFree(LPVOID addr, SIZE_T size, DWORD type);
BOOL WINAPI VirtualProtect(LPVOID addr, SIZE_T size, DWORD newprot, DWORD *oldprot);
SIZE_T WINAPI VirtualQuery(LPCVOID addr, MEMORY_BASIC_INFORMATION *info, SIZE_T len);

/* ---- sync ---- */
HANDLE WINAPI CreateEventW(SECURITY_ATTRIBUTES *sa, BOOL manual_reset, BOOL initial, LPCWSTR name);
HANDLE WINAPI CreateEventA(SECURITY_ATTRIBUTES *sa, BOOL manual_reset, BOOL initial, LPCSTR name);
BOOL WINAPI SetEvent(HANDLE h);
BOOL WINAPI ResetEvent(HANDLE h);
HANDLE WINAPI CreateMutexW(SECURITY_ATTRIBUTES *sa, BOOL initial_owner, LPCWSTR name);
HANDLE WINAPI CreateMutexA(SECURITY_ATTRIBUTES *sa, BOOL initial_owner, LPCSTR name);
BOOL WINAPI ReleaseMutex(HANDLE h);
HANDLE WINAPI CreateSemaphoreW(SECURITY_ATTRIBUTES *sa, LONG initial, LONG maximum, LPCWSTR name);
HANDLE WINAPI CreateSemaphoreA(SECURITY_ATTRIBUTES *sa, LONG initial, LONG maximum, LPCSTR name);
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
