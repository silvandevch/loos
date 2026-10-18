/* LoOS native (NT-level) types and the ntdll surface that kernel32 builds on.
 * Constants and layouts here are the public, documented NT ABI (WDK/PSDK
 * contract); they are facts needed for interop, written fresh.
 *
 * Calling convention: Windows x64. Compiled for Windows targets the default is
 * already right; on the Linux host (tests) it is forced with ms_abi so the same
 * source and a fake ntdll interoperate.
 */
#ifndef LOOS_NT_H
#define LOOS_NT_H

#include <stddef.h>
#include <stdint.h>

#if defined(__x86_64__) && !defined(_WIN32)
#define LOOS_MSABI __attribute__((ms_abi))
#else
#define LOOS_MSABI
#endif
#define NTAPI  LOOS_MSABI
#define WINAPI LOOS_MSABI

typedef uint16_t WCHAR, USHORT;
typedef uint8_t  UCHAR, BOOLEAN;
typedef uint32_t ULONG, DWORD;
typedef int32_t  LONG, NTSTATUS;
typedef int64_t  LONGLONG;
typedef uint64_t ULONGLONG, ULONG_PTR, SIZE_T;
typedef void    *HANDLE, *PVOID;
#define TRUE  1
#define FALSE 0
typedef const WCHAR *PCWSTR;
typedef WCHAR *PWSTR;

typedef union {
    struct { ULONG LowPart; LONG HighPart; };
    LONGLONG QuadPart;
} LARGE_INTEGER;

typedef struct { USHORT Length, MaximumLength; PWSTR Buffer; } UNICODE_STRING;
typedef struct { USHORT Length, MaximumLength; char *Buffer; } ANSI_STRING;

typedef struct {
    ULONG Length;
    HANDLE RootDirectory;
    UNICODE_STRING *ObjectName;
    ULONG Attributes;
    PVOID SecurityDescriptor;
    PVOID SecurityQualityOfService;
} OBJECT_ATTRIBUTES;

typedef struct {
    union { NTSTATUS Status; PVOID Pointer; };
    ULONG_PTR Information;
} IO_STATUS_BLOCK;

/* ---- NTSTATUS values used by the Win32 layer ---- */
#define STATUS_SUCCESS               ((NTSTATUS)0x00000000)
#define STATUS_WAIT_0                ((NTSTATUS)0x00000000)
#define STATUS_ABANDONED_WAIT_0      ((NTSTATUS)0x00000080)
#define STATUS_USER_APC              ((NTSTATUS)0x000000C0)
#define STATUS_TIMEOUT               ((NTSTATUS)0x00000102)
#define STATUS_PENDING               ((NTSTATUS)0x00000103)
#define STATUS_OBJECT_NAME_EXISTS    ((NTSTATUS)0x40000000)
#define STATUS_BUFFER_OVERFLOW       ((NTSTATUS)0x80000005)
#define STATUS_NO_MORE_FILES         ((NTSTATUS)0x80000006)
#define STATUS_UNSUCCESSFUL          ((NTSTATUS)0xC0000001)
#define STATUS_INVALID_INFO_CLASS    ((NTSTATUS)0xC0000003)
#define STATUS_ACCESS_VIOLATION      ((NTSTATUS)0xC0000005)
#define STATUS_INVALID_HANDLE        ((NTSTATUS)0xC0000008)
#define STATUS_INVALID_PARAMETER     ((NTSTATUS)0xC000000D)
#define STATUS_NO_SUCH_FILE          ((NTSTATUS)0xC000000F)
#define STATUS_END_OF_FILE           ((NTSTATUS)0xC0000011)
#define STATUS_NO_MEMORY             ((NTSTATUS)0xC0000017)
#define STATUS_CONFLICTING_ADDRESSES ((NTSTATUS)0xC0000018)
#define STATUS_ACCESS_DENIED         ((NTSTATUS)0xC0000022)
#define STATUS_BUFFER_TOO_SMALL      ((NTSTATUS)0xC0000023)
#define STATUS_OBJECT_TYPE_MISMATCH  ((NTSTATUS)0xC0000024)
#define STATUS_OBJECT_NAME_INVALID   ((NTSTATUS)0xC0000033)
#define STATUS_OBJECT_NAME_NOT_FOUND ((NTSTATUS)0xC0000034)
#define STATUS_OBJECT_NAME_COLLISION ((NTSTATUS)0xC0000035)
#define STATUS_OBJECT_PATH_NOT_FOUND ((NTSTATUS)0xC000003A)
#define STATUS_SHARING_VIOLATION     ((NTSTATUS)0xC0000043)
#define STATUS_DELETE_PENDING        ((NTSTATUS)0xC0000056)
#define STATUS_DISK_FULL             ((NTSTATUS)0xC000007F)
#define STATUS_PROCEDURE_NOT_FOUND   ((NTSTATUS)0xC000007A)
#define STATUS_INVALID_IMAGE_FORMAT  ((NTSTATUS)0xC000007B)
#define STATUS_PIPE_BROKEN           ((NTSTATUS)0xC000014B)
#define STATUS_DLL_NOT_FOUND         ((NTSTATUS)0xC0000135)
#define STATUS_NOT_IMPLEMENTED       ((NTSTATUS)0xC0000002)
#define STATUS_DIRECTORY_NOT_EMPTY   ((NTSTATUS)0xC0000101)
#define STATUS_FILE_IS_A_DIRECTORY   ((NTSTATUS)0xC00000BA)
#define STATUS_NOT_A_DIRECTORY       ((NTSTATUS)0xC0000103)
#define STATUS_MEDIA_WRITE_PROTECTED ((NTSTATUS)0xC00000A2)
#define STATUS_FILE_CLOSED           ((NTSTATUS)0xC0000128)
#define STATUS_MUTANT_NOT_OWNED      ((NTSTATUS)0xC0000046)
#define STATUS_SEMAPHORE_LIMIT_EXCEEDED ((NTSTATUS)0xC0000047)
#define STATUS_NOT_COMMITTED         ((NTSTATUS)0xC000002D)
#define STATUS_PRIVILEGE_NOT_HELD    ((NTSTATUS)0xC0000061)
#define STATUS_CANCELLED             ((NTSTATUS)0xC0000120)
#define STATUS_NAME_TOO_LONG         ((NTSTATUS)0xC0000106)
#define STATUS_INSUFFICIENT_RESOURCES ((NTSTATUS)0xC000009A)
#define STATUS_REVISION_MISMATCH     ((NTSTATUS)0xC0000059)
#define NT_SUCCESS(s) ((NTSTATUS)(s) >= 0)

/* ---- object attributes ---- */
#define OBJ_INHERIT          0x00000002u
#define OBJ_CASE_INSENSITIVE 0x00000040u
#define InitializeObjectAttributes(p, n, a, r, s) do { \
    (p)->Length = sizeof(OBJECT_ATTRIBUTES); (p)->RootDirectory = (r); (p)->Attributes = (a); \
    (p)->ObjectName = (n); (p)->SecurityDescriptor = (s); (p)->SecurityQualityOfService = NULL; } while (0)

/* ---- access rights ---- */
#define DELETE                   0x00010000u
#define READ_CONTROL             0x00020000u
#define SYNCHRONIZE              0x00100000u
#define STANDARD_RIGHTS_REQUIRED 0x000F0000u
#define GENERIC_READ             0x80000000u
#define GENERIC_WRITE            0x40000000u
#define GENERIC_EXECUTE          0x20000000u
#define GENERIC_ALL              0x10000000u
#define FILE_READ_DATA        0x0001u
#define FILE_LIST_DIRECTORY   0x0001u
#define FILE_WRITE_DATA       0x0002u
#define FILE_ADD_FILE         0x0002u
#define FILE_APPEND_DATA      0x0004u
#define FILE_READ_EA          0x0008u
#define FILE_WRITE_EA         0x0010u
#define FILE_EXECUTE          0x0020u
#define FILE_READ_ATTRIBUTES  0x0080u
#define FILE_WRITE_ATTRIBUTES 0x0100u
#define FILE_GENERIC_READ    (READ_CONTROL | FILE_READ_DATA | FILE_READ_ATTRIBUTES | FILE_READ_EA | SYNCHRONIZE)
#define FILE_GENERIC_WRITE   (READ_CONTROL | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA | FILE_APPEND_DATA | SYNCHRONIZE)
#define FILE_GENERIC_EXECUTE (READ_CONTROL | FILE_READ_ATTRIBUTES | FILE_EXECUTE | SYNCHRONIZE)
#define FILE_ALL_ACCESS      (STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x1FFu)
#define EVENT_ALL_ACCESS     (STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3u)
#define MUTANT_ALL_ACCESS    (STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x1u)
#define SEMAPHORE_ALL_ACCESS (STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3u)

/* ---- NtCreateFile parameters ---- */
#define FILE_SHARE_READ   1u
#define FILE_SHARE_WRITE  2u
#define FILE_SHARE_DELETE 4u
#define FILE_SUPERSEDE    0u
#define FILE_OPEN         1u
#define FILE_CREATE       2u
#define FILE_OPEN_IF      3u
#define FILE_OVERWRITE    4u
#define FILE_OVERWRITE_IF 5u
#define FILE_DIRECTORY_FILE            0x00000001u
#define FILE_WRITE_THROUGH             0x00000002u
#define FILE_SEQUENTIAL_ONLY           0x00000004u
#define FILE_NO_INTERMEDIATE_BUFFERING 0x00000008u
#define FILE_SYNCHRONOUS_IO_ALERT      0x00000010u
#define FILE_SYNCHRONOUS_IO_NONALERT   0x00000020u
#define FILE_NON_DIRECTORY_FILE        0x00000040u
#define FILE_RANDOM_ACCESS             0x00000800u
#define FILE_DELETE_ON_CLOSE           0x00001000u
#define FILE_OPEN_REPARSE_POINT        0x00200000u
/* IO_STATUS_BLOCK.Information after create/open */
#define FILE_SUPERSEDED   0u
#define FILE_OPENED       1u
#define FILE_CREATED      2u
#define FILE_OVERWRITTEN  3u
#define FILE_EXISTS       4u
#define FILE_DOES_NOT_EXIST 5u

typedef enum { FileBasicInformation = 4, FileStandardInformation = 5, FilePositionInformation = 14,
               FileDispositionInformation = 13, FileAllocationInformation = 19, FileEndOfFileInformation = 20 } FILE_INFORMATION_CLASS;
typedef struct { LARGE_INTEGER CreationTime, LastAccessTime, LastWriteTime, ChangeTime; ULONG FileAttributes; } FILE_BASIC_INFORMATION;
typedef struct { LARGE_INTEGER AllocationSize, EndOfFile; ULONG NumberOfLinks; BOOLEAN DeletePending, Directory; } FILE_STANDARD_INFORMATION;
typedef struct { LARGE_INTEGER CurrentByteOffset; } FILE_POSITION_INFORMATION;
typedef struct { BOOLEAN DeleteFile; } FILE_DISPOSITION_INFORMATION;
typedef struct { LARGE_INTEGER EndOfFile; } FILE_END_OF_FILE_INFORMATION;

/* ---- virtual memory ---- */
#define MEM_COMMIT   0x00001000u
#define MEM_RESERVE  0x00002000u
#define MEM_DECOMMIT 0x00004000u
#define MEM_RELEASE  0x00008000u
#define MEM_RESET    0x00080000u
#define MEM_TOP_DOWN 0x00100000u
#define PAGE_NOACCESS          0x01
#define PAGE_READONLY          0x02
#define PAGE_READWRITE         0x04
#define PAGE_WRITECOPY         0x08
#define PAGE_EXECUTE           0x10
#define PAGE_EXECUTE_READ      0x20
#define PAGE_EXECUTE_READWRITE 0x40
#define PAGE_GUARD             0x100
typedef struct {
    PVOID BaseAddress, AllocationBase;
    ULONG AllocationProtect;
    SIZE_T RegionSize;
    ULONG State, Protect, Type;
} MEMORY_BASIC_INFORMATION;

typedef enum { EventNotificationType = 0, EventSynchronizationType = 1 } EVENT_TYPE;
typedef enum { WaitAllObjects = 0, WaitAnyObject = 1 } WAIT_TYPE;

#define NtCurrentProcess() ((HANDLE)(intptr_t)-1)
#define NtCurrentThread()  ((HANDLE)(intptr_t)-2)

/* ---- the ntdll surface (implemented by the kernel via syscalls, or by a test fake) ---- */
NTSTATUS NTAPI NtCreateFile(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, IO_STATUS_BLOCK *iosb,
                            LARGE_INTEGER *alloc, ULONG attrs, ULONG share, ULONG disposition,
                            ULONG options, PVOID ea, ULONG ea_len);
NTSTATUS NTAPI NtOpenFile(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, IO_STATUS_BLOCK *iosb, ULONG share, ULONG options);
NTSTATUS NTAPI NtReadFile(HANDLE h, HANDLE event, PVOID apc, PVOID apc_ctx, IO_STATUS_BLOCK *iosb,
                          PVOID buf, ULONG len, LARGE_INTEGER *off, ULONG *key);
NTSTATUS NTAPI NtWriteFile(HANDLE h, HANDLE event, PVOID apc, PVOID apc_ctx, IO_STATUS_BLOCK *iosb,
                           const void *buf, ULONG len, LARGE_INTEGER *off, ULONG *key);
NTSTATUS NTAPI NtQueryInformationFile(HANDLE h, IO_STATUS_BLOCK *iosb, PVOID info, ULONG len, FILE_INFORMATION_CLASS cls);
NTSTATUS NTAPI NtSetInformationFile(HANDLE h, IO_STATUS_BLOCK *iosb, PVOID info, ULONG len, FILE_INFORMATION_CLASS cls);
NTSTATUS NTAPI NtQueryAttributesFile(OBJECT_ATTRIBUTES *oa, FILE_BASIC_INFORMATION *info);
NTSTATUS NTAPI NtFlushBuffersFile(HANDLE h, IO_STATUS_BLOCK *iosb);
NTSTATUS NTAPI NtClose(HANDLE h);
NTSTATUS NTAPI NtDuplicateObject(HANDLE src_proc, HANDLE src, HANDLE dst_proc, HANDLE *dst, ULONG access, ULONG attrs, ULONG options);

NTSTATUS NTAPI NtAllocateVirtualMemory(HANDLE proc, PVOID *base, ULONG_PTR zero_bits, SIZE_T *size, ULONG type, ULONG protect);
NTSTATUS NTAPI NtFreeVirtualMemory(HANDLE proc, PVOID *base, SIZE_T *size, ULONG type);
NTSTATUS NTAPI NtProtectVirtualMemory(HANDLE proc, PVOID *base, SIZE_T *size, ULONG newprot, ULONG *oldprot);
NTSTATUS NTAPI NtQueryVirtualMemory(HANDLE proc, PVOID addr, int cls, PVOID buf, SIZE_T len, SIZE_T *retlen);

NTSTATUS NTAPI NtCreateEvent(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, EVENT_TYPE type, BOOLEAN initial);
NTSTATUS NTAPI NtSetEvent(HANDLE h, LONG *prev);
NTSTATUS NTAPI NtResetEvent(HANDLE h, LONG *prev);
NTSTATUS NTAPI NtCreateMutant(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, BOOLEAN initial_owner);
NTSTATUS NTAPI NtReleaseMutant(HANDLE h, LONG *prev);
NTSTATUS NTAPI NtCreateSemaphore(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, LONG initial, LONG maximum);
NTSTATUS NTAPI NtReleaseSemaphore(HANDLE h, LONG count, LONG *prev);
NTSTATUS NTAPI NtWaitForSingleObject(HANDLE h, BOOLEAN alertable, LARGE_INTEGER *timeout);
NTSTATUS NTAPI NtWaitForMultipleObjects(ULONG n, HANDLE *h, WAIT_TYPE type, BOOLEAN alertable, LARGE_INTEGER *timeout);
NTSTATUS NTAPI NtDelayExecution(BOOLEAN alertable, LARGE_INTEGER *interval);
NTSTATUS NTAPI NtQueryPerformanceCounter(LARGE_INTEGER *counter, LARGE_INTEGER *freq);
NTSTATUS NTAPI NtQuerySystemTime(LARGE_INTEGER *t);
NTSTATUS NTAPI NtTerminateProcess(HANDLE process, NTSTATUS exit_status);

/* loader */
NTSTATUS NTAPI LdrLoadDll(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle);
NTSTATUS NTAPI LdrGetDllHandle(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle);
NTSTATUS NTAPI LdrGetProcedureAddress(PVOID module, ANSI_STRING *name, ULONG ordinal, PVOID *addr);
NTSTATUS NTAPI LdrUnloadDll(PVOID module);

/* heap (ntdll) */
#define HEAP_NO_SERIALIZE          0x00000001u
#define HEAP_GROWABLE              0x00000002u
#define HEAP_GENERATE_EXCEPTIONS   0x00000004u
#define HEAP_ZERO_MEMORY           0x00000008u
#define HEAP_REALLOC_IN_PLACE_ONLY 0x00000010u
PVOID   NTAPI RtlCreateHeap(ULONG flags, PVOID base, SIZE_T reserve, SIZE_T commit, PVOID lock, PVOID params);
HANDLE  NTAPI RtlDestroyHeap(HANDLE heap);
PVOID   NTAPI RtlAllocateHeap(HANDLE heap, ULONG flags, SIZE_T size);
BOOLEAN NTAPI RtlFreeHeap(HANDLE heap, ULONG flags, PVOID ptr);
PVOID   NTAPI RtlReAllocateHeap(HANDLE heap, ULONG flags, PVOID ptr, SIZE_T size);
SIZE_T  NTAPI RtlSizeHeap(HANDLE heap, ULONG flags, PVOID ptr);
BOOLEAN NTAPI RtlValidateHeap(HANDLE heap, ULONG flags, PVOID ptr);
HANDLE  NTAPI RtlGetProcessHeap(void);

/* version */
typedef struct {
    ULONG dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion, dwBuildNumber, dwPlatformId;
    WCHAR szCSDVersion[128];
} RTL_OSVERSIONINFOW;
typedef struct {
    ULONG dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion, dwBuildNumber, dwPlatformId;
    WCHAR szCSDVersion[128];
    USHORT wServicePackMajor, wServicePackMinor, wSuiteMask;
    UCHAR wProductType, wReserved;
} RTL_OSVERSIONINFOEXW;
NTSTATUS NTAPI RtlGetVersion(RTL_OSVERSIONINFOW *info);
NTSTATUS NTAPI RtlVerifyVersionInfo(RTL_OSVERSIONINFOEXW *info, ULONG type_mask, ULONGLONG condition_mask);
ULONGLONG NTAPI VerSetConditionMask(ULONGLONG mask, ULONG type_mask, UCHAR condition);

/* process */
void NTAPI RtlExitUserProcess(NTSTATUS code);
/* Process start (ntdll): load the image and its imports, resolve the compatibility
 * claim, run DLL initialisers and call the image entry point. Does not return. */
void NTAPI LdrStartProcess(PCWSTR image_path);
void NTAPI RtlUserThreadStart(PVOID entry, PVOID arg);

/* ntdll utility (implemented in src/win32/ntdll/errmap.c) */
ULONG NTAPI RtlNtStatusToDosError(NTSTATUS status);

#endif
