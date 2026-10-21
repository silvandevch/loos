/* File mappings: CreateFileMapping/OpenFileMapping/MapViewOfFile over NT sections.
 * The ANSI entry points belong to the ANSI layer (ansi.c). */
#include "k32_internal.h"

#define ALLOC_GRANULARITY 0x10000u

static HANDLE create_mapping(HANDLE file, SECURITY_ATTRIBUTES *sa, DWORD protect, DWORD hi, DWORD lo, LPCWSTR name)
{
    WCHAR buf[512]; UNICODE_STRING us; OBJECT_ATTRIBUTES oa; LARGE_INTEGER size, *psize = NULL;
    HANDLE h = NULL; DWORD err, attrs = protect & (SEC_IMAGE | SEC_RESERVE | SEC_COMMIT | SEC_NOCACHE), page = protect & 0xFF, access;
    NTSTATUS st;

    switch (page) {
    case PAGE_READONLY: case PAGE_WRITECOPY: case PAGE_READWRITE:
    case PAGE_EXECUTE_READ: case PAGE_EXECUTE_READWRITE: case PAGE_EXECUTE_WRITECOPY: break;
    default: *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL;
    }
    if (file == INVALID_HANDLE_VALUE) {                 /* backed by the system, not a file: a size is mandatory */
        file = NULL;
        if (!hi && !lo) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    }
    if (!(attrs & (SEC_IMAGE | SEC_RESERVE))) attrs |= SEC_COMMIT;
    if (hi || lo) { size.QuadPart = (LONGLONG)(((uint64_t)hi << 32) | lo); psize = &size; }
    access = SECTION_MAP_READ | SECTION_QUERY;
    if (page != PAGE_READONLY && page != PAGE_EXECUTE_READ) access |= SECTION_MAP_WRITE;
    if (page >= PAGE_EXECUTE) access |= SECTION_MAP_EXECUTE;
    access |= SECTION_ALL_ACCESS & ~0xFFFFu;           /* standard rights */
    if ((err = k32_object_attrs(sa, name, buf, 512, &us, &oa))) { *k32_last_error_ptr() = err; return NULL; }
    st = NtCreateSection(&h, access | SECTION_EXTEND_SIZE, &oa, psize, page, attrs, file);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    *k32_last_error_ptr() = (st == STATUS_OBJECT_NAME_EXISTS) ? ERROR_ALREADY_EXISTS : ERROR_SUCCESS;
    return h;
}

HANDLE WINAPI CreateFileMappingW(HANDLE file, SECURITY_ATTRIBUTES *sa, DWORD protect, DWORD hi, DWORD lo, LPCWSTR name)
{ return create_mapping(file, sa, protect, hi, lo, name); }

HANDLE WINAPI OpenFileMappingW(DWORD access, BOOL inherit, LPCWSTR name)
{
    WCHAR buf[512]; UNICODE_STRING us; OBJECT_ATTRIBUTES oa; HANDLE h = NULL; DWORD err; NTSTATUS st;
    if (!name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    if ((err = k32_object_attrs(NULL, name, buf, 512, &us, &oa))) { *k32_last_error_ptr() = err; return NULL; }
    if (inherit) oa.Attributes |= OBJ_INHERIT;
    if (access & FILE_MAP_COPY) access = (access & ~(DWORD)FILE_MAP_COPY) | SECTION_MAP_READ;   /* copy-on-write needs read access only */
    st = NtOpenSection(&h, access, &oa);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return h;
}

LPVOID WINAPI MapViewOfFileEx(HANDLE mapping, DWORD access, DWORD off_hi, DWORD off_lo, SIZE_T bytes, LPVOID base)
{
    LARGE_INTEGER off; PVOID where = base; SIZE_T size = bytes; ULONG prot; NTSTATUS st;
    off.QuadPart = (LONGLONG)(((uint64_t)off_hi << 32) | off_lo);
    if (off_lo & (ALLOC_GRANULARITY - 1)) { *k32_last_error_ptr() = ERROR_MAPPED_ALIGNMENT; return NULL; }
    if (access & FILE_MAP_COPY)            prot = (access & FILE_MAP_EXECUTE) ? PAGE_EXECUTE_WRITECOPY : PAGE_WRITECOPY;
    else if (access & FILE_MAP_WRITE)      prot = (access & FILE_MAP_EXECUTE) ? PAGE_EXECUTE_READWRITE : PAGE_READWRITE;
    else if (access & FILE_MAP_READ)       prot = (access & FILE_MAP_EXECUTE) ? PAGE_EXECUTE_READ : PAGE_READONLY;
    else { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    st = NtMapViewOfSection(mapping, NtCurrentProcess(), &where, 0, 0, &off, &size, ViewShare, 0, prot);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return where;
}

LPVOID WINAPI MapViewOfFile(HANDLE mapping, DWORD access, DWORD off_hi, DWORD off_lo, SIZE_T bytes)
{ return MapViewOfFileEx(mapping, access, off_hi, off_lo, bytes, NULL); }

BOOL WINAPI UnmapViewOfFile(LPCVOID base)
{
    NTSTATUS st = NtUnmapViewOfSection(NtCurrentProcess(), (PVOID)base);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI FlushViewOfFile(LPCVOID base, SIZE_T bytes)
{
    PVOID b = (PVOID)base; SIZE_T n = bytes; IO_STATUS_BLOCK iosb;
    NTSTATUS st = NtFlushVirtualMemory(NtCurrentProcess(), &b, &n, &iosb);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
