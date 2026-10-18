/* File API: Win32 -> NT translation. */
#include "k32_internal.h"
#include <string.h>

#define FILE_ATTR_VALID 0x000031A7u

static DWORD map_generic(DWORD a)
{
    if (a & GENERIC_READ)    a |= FILE_GENERIC_READ;
    if (a & GENERIC_WRITE)   a |= FILE_GENERIC_WRITE;
    if (a & GENERIC_EXECUTE) a |= FILE_GENERIC_EXECUTE;
    if (a & GENERIC_ALL)     a |= FILE_ALL_ACCESS;
    return a & ~(GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL);
}

HANDLE WINAPI CreateFileW(LPCWSTR name, DWORD access, DWORD share, SECURITY_ATTRIBUTES *sa,
                          DWORD disposition, DWORD flags, HANDLE template_file)
{
    WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    HANDLE h = NULL;
    NTSTATUS st;
    DWORD err, nt_disp, options = 0, nt_access;
    ULONG oattrs = OBJ_CASE_INSENSITIVE;

    (void)template_file;
    switch (disposition) {
    case CREATE_NEW:        nt_disp = FILE_CREATE; break;
    case CREATE_ALWAYS:     nt_disp = FILE_OVERWRITE_IF; break;
    case OPEN_EXISTING:     nt_disp = FILE_OPEN; break;
    case OPEN_ALWAYS:       nt_disp = FILE_OPEN_IF; break;
    case TRUNCATE_EXISTING: nt_disp = FILE_OVERWRITE; break;
    default: *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return INVALID_HANDLE_VALUE;
    }
    if (disposition == TRUNCATE_EXISTING && !(access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA))) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
        return INVALID_HANDLE_VALUE;
    }
    if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return INVALID_HANDLE_VALUE; }

    err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us);
    if (err) { *k32_last_error_ptr() = err; return INVALID_HANDLE_VALUE; }

    nt_access = map_generic(access) | SYNCHRONIZE | FILE_READ_ATTRIBUTES;
    if (!(flags & FILE_FLAG_BACKUP_SEMANTICS)) options |= FILE_NON_DIRECTORY_FILE;
    if (!(flags & FILE_FLAG_OVERLAPPED))       options |= FILE_SYNCHRONOUS_IO_NONALERT;
    if (flags & FILE_FLAG_DELETE_ON_CLOSE)   { options |= FILE_DELETE_ON_CLOSE; nt_access |= DELETE; }
    if (flags & FILE_FLAG_NO_BUFFERING)        options |= FILE_NO_INTERMEDIATE_BUFFERING;
    if (flags & FILE_FLAG_WRITE_THROUGH)       options |= FILE_WRITE_THROUGH;
    if (flags & FILE_FLAG_SEQUENTIAL_SCAN)     options |= FILE_SEQUENTIAL_ONLY;
    if (flags & FILE_FLAG_RANDOM_ACCESS)       options |= FILE_RANDOM_ACCESS;
    if (flags & FILE_FLAG_OPEN_REPARSE_POINT)  options |= FILE_OPEN_REPARSE_POINT;
    if (sa && sa->bInheritHandle) oattrs |= OBJ_INHERIT;
    InitializeObjectAttributes(&oa, &us, oattrs, NULL, sa ? sa->lpSecurityDescriptor : NULL);

    st = NtCreateFile(&h, nt_access, &oa, &iosb, NULL, flags & FILE_ATTR_VALID, share & 7u, nt_disp, options, NULL, 0);
    if (!NT_SUCCESS(st)) {
        /* CREATE_NEW on an existing file reports ERROR_FILE_EXISTS, not the generic ALREADY_EXISTS */
        if (st == STATUS_OBJECT_NAME_COLLISION) *k32_last_error_ptr() = ERROR_FILE_EXISTS;
        else k32_set_nt_error(st);
        return INVALID_HANDLE_VALUE;
    }

    /* CREATE_ALWAYS / OPEN_ALWAYS report "it already existed" through last-error even on success. */
    if ((disposition == CREATE_ALWAYS && iosb.Information == FILE_OVERWRITTEN) ||
        (disposition == OPEN_ALWAYS && iosb.Information == FILE_OPENED))
        *k32_last_error_ptr() = ERROR_ALREADY_EXISTS;
    else
        *k32_last_error_ptr() = ERROR_SUCCESS;
    return h;
}

static BOOL is_bad_handle(HANDLE h) { return h == NULL || h == INVALID_HANDLE_VALUE; }

BOOL WINAPI ReadFile(HANDLE h, LPVOID buf, DWORD len, LPDWORD read, OVERLAPPED *ov)
{
    IO_STATUS_BLOCK iosb;
    LARGE_INTEGER off, *poff = NULL;
    NTSTATUS st;

    if (read) *read = 0;
    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    if (ov) {
        off.QuadPart = (LONGLONG)(((ULONGLONG)ov->OffsetHigh << 32) | ov->Offset);
        poff = &off;
        ov->Internal = (ULONG_PTR)STATUS_PENDING;
        ov->InternalHigh = 0;
    } else if (!read) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE;
    }
    st = NtReadFile(h, ov ? ov->hEvent : NULL, NULL, NULL, &iosb, buf, len, poff, NULL);
    if (st == STATUS_PENDING && !ov) {              /* synchronous handle: wait for completion */
        st = NtWaitForSingleObject(h, FALSE, NULL);
        if (NT_SUCCESS(st)) st = iosb.Status;
    }
    if (ov && st != STATUS_PENDING) { ov->Internal = (ULONG_PTR)st; ov->InternalHigh = iosb.Information; }
    if (st == STATUS_END_OF_FILE) {                 /* EOF on a file is success with 0 bytes */
        if (read) *read = 0;
        return TRUE;
    }
    if (!NT_SUCCESS(st) && st != STATUS_PENDING) { k32_set_nt_error(st); return FALSE; }
    if (st == STATUS_PENDING) { *k32_last_error_ptr() = ERROR_IO_PENDING; return FALSE; }
    if (read) *read = (DWORD)iosb.Information;
    return TRUE;
}

BOOL WINAPI WriteFile(HANDLE h, LPCVOID buf, DWORD len, LPDWORD written, OVERLAPPED *ov)
{
    IO_STATUS_BLOCK iosb;
    LARGE_INTEGER off, *poff = NULL;
    NTSTATUS st;

    if (written) *written = 0;
    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    if (ov) {
        off.QuadPart = (LONGLONG)(((ULONGLONG)ov->OffsetHigh << 32) | ov->Offset);
        poff = &off;
        ov->Internal = (ULONG_PTR)STATUS_PENDING;
        ov->InternalHigh = 0;
    } else if (!written) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE;
    }
    st = NtWriteFile(h, ov ? ov->hEvent : NULL, NULL, NULL, &iosb, buf, len, poff, NULL);
    if (st == STATUS_PENDING && !ov) {
        st = NtWaitForSingleObject(h, FALSE, NULL);
        if (NT_SUCCESS(st)) st = iosb.Status;
    }
    if (ov && st != STATUS_PENDING) { ov->Internal = (ULONG_PTR)st; ov->InternalHigh = iosb.Information; }
    if (!NT_SUCCESS(st) && st != STATUS_PENDING) { k32_set_nt_error(st); return FALSE; }
    if (st == STATUS_PENDING) { *k32_last_error_ptr() = ERROR_IO_PENDING; return FALSE; }
    if (written) *written = (DWORD)iosb.Information;
    return TRUE;
}

BOOL WINAPI CloseHandle(HANDLE h)
{
    NTSTATUS st;
    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtClose(h);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI DuplicateHandle(HANDLE sp, HANDLE src, HANDLE dp, HANDLE *dst, DWORD access, BOOL inherit, DWORD options)
{
    NTSTATUS st = NtDuplicateObject(sp, src, dp, dst, access, inherit ? OBJ_INHERIT : 0, options);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI SetFilePointerEx(HANDLE h, LARGE_INTEGER dist, LARGE_INTEGER *newpos, DWORD method)
{
    IO_STATUS_BLOCK iosb;
    FILE_POSITION_INFORMATION pos;
    NTSTATUS st;
    LONGLONG target;

    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    switch (method) {
    case FILE_BEGIN: target = dist.QuadPart; break;
    case FILE_CURRENT:
        st = NtQueryInformationFile(h, &iosb, &pos, sizeof pos, FilePositionInformation);
        if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
        target = pos.CurrentByteOffset.QuadPart + dist.QuadPart;
        break;
    case FILE_END: {
        FILE_STANDARD_INFORMATION si;
        st = NtQueryInformationFile(h, &iosb, &si, sizeof si, FileStandardInformation);
        if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
        target = si.EndOfFile.QuadPart + dist.QuadPart;
        break; }
    default: *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE;
    }
    if (target < 0) { *k32_last_error_ptr() = ERROR_NEGATIVE_SEEK; return FALSE; }
    pos.CurrentByteOffset.QuadPart = target;
    st = NtSetInformationFile(h, &iosb, &pos, sizeof pos, FilePositionInformation);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    if (newpos) newpos->QuadPart = target;
    return TRUE;
}

DWORD WINAPI SetFilePointer(HANDLE h, LONG dist, LONG *dist_high, DWORD method)
{
    LARGE_INTEGER d, np;
    d.QuadPart = dist_high ? (LONGLONG)(((ULONGLONG)(DWORD)*dist_high << 32) | (DWORD)dist) : (LONGLONG)dist;
    if (!SetFilePointerEx(h, d, &np, method)) return INVALID_SET_FILE_POINTER;
    if (dist_high) *dist_high = np.HighPart;
    else if (np.HighPart) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return INVALID_SET_FILE_POINTER; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return np.LowPart;
}

BOOL WINAPI GetFileSizeEx(HANDLE h, LARGE_INTEGER *size)
{
    IO_STATUS_BLOCK iosb;
    FILE_STANDARD_INFORMATION si;
    NTSTATUS st;
    if (is_bad_handle(h) || !size) { *k32_last_error_ptr() = is_bad_handle(h) ? ERROR_INVALID_HANDLE : ERROR_INVALID_PARAMETER; return FALSE; }
    st = NtQueryInformationFile(h, &iosb, &si, sizeof si, FileStandardInformation);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    *size = si.EndOfFile;
    return TRUE;
}

BOOL WINAPI SetEndOfFile(HANDLE h)
{
    IO_STATUS_BLOCK iosb;
    FILE_POSITION_INFORMATION pos;
    FILE_END_OF_FILE_INFORMATION eof;
    NTSTATUS st;
    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtQueryInformationFile(h, &iosb, &pos, sizeof pos, FilePositionInformation);
    if (NT_SUCCESS(st)) {
        eof.EndOfFile = pos.CurrentByteOffset;
        st = NtSetInformationFile(h, &iosb, &eof, sizeof eof, FileEndOfFileInformation);
    }
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI FlushFileBuffers(HANDLE h)
{
    IO_STATUS_BLOCK iosb;
    NTSTATUS st;
    if (is_bad_handle(h)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtFlushBuffersFile(h, &iosb);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

DWORD WINAPI GetFileAttributesW(LPCWSTR name)
{
    WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    FILE_BASIC_INFORMATION bi;
    DWORD err;
    NTSTATUS st;

    if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return INVALID_FILE_ATTRIBUTES; }
    err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us);
    if (err) { *k32_last_error_ptr() = err; return INVALID_FILE_ATTRIBUTES; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);
    st = NtQueryAttributesFile(&oa, &bi);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return INVALID_FILE_ATTRIBUTES; }
    return bi.FileAttributes;
}

/* Open DELETE access, mark delete-on-close via disposition, close. */
static BOOL delete_object(LPCWSTR name, ULONG options)
{
    WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    FILE_DISPOSITION_INFORMATION di = { TRUE };
    HANDLE h = NULL;
    DWORD err;
    NTSTATUS st;

    if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return FALSE; }
    err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us);
    if (err) { *k32_last_error_ptr() = err; return FALSE; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);
    st = NtOpenFile(&h, DELETE | SYNCHRONIZE, &oa, &iosb, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    options | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT);
    if (NT_SUCCESS(st)) {
        st = NtSetInformationFile(h, &iosb, &di, sizeof di, FileDispositionInformation);
        NtClose(h);
    }
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI DeleteFileW(LPCWSTR name)    { return delete_object(name, FILE_NON_DIRECTORY_FILE); }
BOOL WINAPI RemoveDirectoryW(LPCWSTR name) { return delete_object(name, FILE_DIRECTORY_FILE); }

BOOL WINAPI CreateDirectoryW(LPCWSTR name, SECURITY_ATTRIBUTES *sa)
{
    WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    HANDLE h = NULL;
    DWORD err;
    NTSTATUS st;

    if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return FALSE; }
    err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us);
    if (err) { *k32_last_error_ptr() = err; return FALSE; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE | ((sa && sa->bInheritHandle) ? OBJ_INHERIT : 0), NULL,
                               sa ? sa->lpSecurityDescriptor : NULL);
    st = NtCreateFile(&h, FILE_LIST_DIRECTORY | SYNCHRONIZE, &oa, &iosb, NULL, FILE_ATTRIBUTE_NORMAL,
                      FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_CREATE,
                      FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT, NULL, 0);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    NtClose(h);
    return TRUE;
}
