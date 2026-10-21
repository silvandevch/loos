/* File enumeration, attributes, times, rename, copy, temporary names. */
#include "k32_internal.h"
#include <string.h>

static void ft_from_li(FILETIME *ft, LARGE_INTEGER li) { ft->dwLowDateTime = (DWORD)li.QuadPart; ft->dwHighDateTime = (DWORD)((uint64_t)li.QuadPart >> 32); }
static LARGE_INTEGER li_from_ft(const FILETIME *ft) { LARGE_INTEGER li; li.QuadPart = (LONGLONG)(((uint64_t)ft->dwHighDateTime << 32) | ft->dwLowDateTime); return li; }
static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }

/* ---------- FindFirstFile / FindNextFile ---------- */
#define FIND_MAGIC 0x444E4946u       /* 'FIND' */
typedef struct {
    ULONG magic;
    HANDLE dir;
    ULONG attr_mask_unused;
    int basic;                        /* FindExInfoBasic: no short names */
    int done;
    UNICODE_STRING mask; WCHAR mask_buf[260];
    uint8_t *pos; size_t left;        /* unparsed bytes in buf */
    uint8_t buf[4096] __attribute__((aligned(8)));
} find_ctx;

static int is_wild(const WCHAR *s, size_t n)
{
    size_t i;
    for (i = 0; i < n; i++) if (s[i] == '*' || s[i] == '?' || s[i] == '<' || s[i] == '>' || s[i] == '"') return 1;
    return 0;
}

static BOOL fill_from_buffer(find_ctx *c, WIN32_FIND_DATAW *d)
{
    FILE_BOTH_DIR_INFORMATION *e;
    size_t nlen, i;
    if (!c->left) return FALSE;
    e = (FILE_BOTH_DIR_INFORMATION *)c->pos;
    memset(d, 0, sizeof *d);
    d->dwFileAttributes = e->FileAttributes;
    ft_from_li(&d->ftCreationTime, e->CreationTime); ft_from_li(&d->ftLastAccessTime, e->LastAccessTime); ft_from_li(&d->ftLastWriteTime, e->LastWriteTime);
    d->nFileSizeLow = (DWORD)e->EndOfFile.QuadPart; d->nFileSizeHigh = (DWORD)((uint64_t)e->EndOfFile.QuadPart >> 32);
    nlen = e->FileNameLength / sizeof(WCHAR);
    if (nlen > 259) nlen = 259;
    for (i = 0; i < nlen; i++) d->cFileName[i] = e->FileName[i];
    if (!c->basic && e->ShortNameLength > 0) {
        size_t sl = (size_t)e->ShortNameLength / sizeof(WCHAR);
        if (sl > 13) sl = 13;
        for (i = 0; i < sl; i++) d->cAlternateFileName[i] = e->ShortName[i];
    }
    if (e->NextEntryOffset) { c->pos += e->NextEntryOffset; c->left -= e->NextEntryOffset; } else { c->left = 0; }
    return TRUE;
}

static NTSTATUS refill(find_ctx *c, int restart)
{
    IO_STATUS_BLOCK iosb;
    NTSTATUS st = NtQueryDirectoryFile(c->dir, NULL, NULL, NULL, &iosb, c->buf, sizeof c->buf, FileBothDirectoryInformation, FALSE, &c->mask, restart != 0);
    if (st == STATUS_PENDING) { st = NtWaitForSingleObject(c->dir, FALSE, NULL); if (NT_SUCCESS(st)) st = iosb.Status; }
    if (NT_SUCCESS(st)) { c->pos = c->buf; c->left = iosb.Information; }
    return st;
}

HANDLE WINAPI FindFirstFileExW(LPCWSTR pattern, FINDEX_INFO_LEVELS level, LPVOID data, FINDEX_SEARCH_OPS op, LPVOID filter, DWORD flags)
{
    static WCHAR nt_buf[K32_PATH_MAX];
    WCHAR dos[K32_LOCAL_MAX], full[K32_LOCAL_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    find_ctx *c;
    size_t n, cut;
    DWORD err;
    NTSTATUS st;
    (void)filter; (void)flags; (void)op;

    if (!pattern || !*pattern || !data || (level != FindExInfoStandard && level != FindExInfoBasic)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return INVALID_HANDLE_VALUE; }
    n = GetFullPathNameW(pattern, K32_LOCAL_MAX, full, NULL);
    if (!n || n >= K32_LOCAL_MAX) { if (!n) return INVALID_HANDLE_VALUE; *k32_last_error_ptr() = ERROR_FILENAME_EXCED_RANGE; return INVALID_HANDLE_VALUE; }
    cut = n;
    while (cut > 0 && full[cut - 1] != '\\') cut--;                       /* full[0..cut) is the directory, with its trailing backslash */
    if (cut == n) { *k32_last_error_ptr() = ERROR_FILE_NOT_FOUND; return INVALID_HANDLE_VALUE; }     /* trailing backslash: no name to match */
    memcpy(dos, full, cut * sizeof(WCHAR)); dos[cut] = 0;
    if (cut > 3 && dos[cut - 1] == '\\') dos[cut - 1] = 0;               /* "C:\dir\" -> "C:\dir" ("C:\" stays) */
    if ((err = k32_dos_to_nt(dos, nt_buf, K32_PATH_MAX, &us)) != 0) { *k32_last_error_ptr() = err; return INVALID_HANDLE_VALUE; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);

    c = RtlAllocateHeap(RtlGetProcessHeap(), HEAP_ZERO_MEMORY, sizeof *c);
    if (!c) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return INVALID_HANDLE_VALUE; }
    st = NtOpenFile(&c->dir, FILE_LIST_DIRECTORY | SYNCHRONIZE, &oa, &iosb, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT);
    if (!NT_SUCCESS(st)) { RtlFreeHeap(RtlGetProcessHeap(), 0, c); *k32_last_error_ptr() = (st == STATUS_OBJECT_NAME_NOT_FOUND || st == STATUS_NOT_A_DIRECTORY) ? ERROR_PATH_NOT_FOUND : RtlNtStatusToDosError(st); return INVALID_HANDLE_VALUE; }
    c->magic = FIND_MAGIC; c->basic = level == FindExInfoBasic;
    { size_t ml = n - cut; memcpy(c->mask_buf, full + cut, ml * sizeof(WCHAR)); c->mask_buf[ml] = 0;
      c->mask.Buffer = c->mask_buf; c->mask.Length = (USHORT)(ml * sizeof(WCHAR)); c->mask.MaximumLength = (USHORT)(c->mask.Length + sizeof(WCHAR)); (void)is_wild; }
    st = refill(c, 1);
    if (!NT_SUCCESS(st) || !fill_from_buffer(c, data)) {
        NtClose(c->dir); c->magic = 0; RtlFreeHeap(RtlGetProcessHeap(), 0, c);
        *k32_last_error_ptr() = (st == STATUS_NO_MORE_FILES || st == STATUS_NO_SUCH_FILE || NT_SUCCESS(st)) ? ERROR_FILE_NOT_FOUND : RtlNtStatusToDosError(st);
        return INVALID_HANDLE_VALUE;
    }
    return c;
}

HANDLE WINAPI FindFirstFileW(LPCWSTR pattern, WIN32_FIND_DATAW *data) { return FindFirstFileExW(pattern, FindExInfoStandard, data, FindExSearchNameMatch, NULL, 0); }

BOOL WINAPI FindNextFileW(HANDLE h, WIN32_FIND_DATAW *data)
{
    find_ctx *c = h;
    NTSTATUS st;
    if (!c || c == INVALID_HANDLE_VALUE || c->magic != FIND_MAGIC || !data) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    if (c->done) { *k32_last_error_ptr() = ERROR_NO_MORE_FILES; return FALSE; }
    if (!c->left) {
        st = refill(c, 0);
        if (st == STATUS_NO_MORE_FILES || st == STATUS_NO_SUCH_FILE) { c->done = 1; *k32_last_error_ptr() = ERROR_NO_MORE_FILES; return FALSE; }
        if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    }
    return fill_from_buffer(c, data);
}

BOOL WINAPI FindClose(HANDLE h)
{
    find_ctx *c = h;
    if (!c || c == INVALID_HANDLE_VALUE || c->magic != FIND_MAGIC) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    NtClose(c->dir);
    c->magic = 0;
    RtlFreeHeap(RtlGetProcessHeap(), 0, c);
    return TRUE;
}

/* ---------- sizes, attributes, times ---------- */
DWORD WINAPI GetFileSize(HANDLE h, LPDWORD high)
{
    LARGE_INTEGER sz;
    if (!GetFileSizeEx(h, &sz)) return INVALID_FILE_SIZE;
    if (high) *high = (DWORD)((uint64_t)sz.QuadPart >> 32);
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return (DWORD)sz.QuadPart;
}

BOOL WINAPI GetFileInformationByHandle(HANDLE h, BY_HANDLE_FILE_INFORMATION *info)
{
    IO_STATUS_BLOCK iosb;
    FILE_BASIC_INFORMATION b;
    FILE_STANDARD_INFORMATION s;
    FILE_INTERNAL_INFORMATION idx;
    NTSTATUS st;
    if (h == NULL || h == INVALID_HANDLE_VALUE || !info) { *k32_last_error_ptr() = h && info ? ERROR_INVALID_HANDLE : (info ? ERROR_INVALID_HANDLE : ERROR_INVALID_PARAMETER); return FALSE; }
    if (!NT_SUCCESS(st = NtQueryInformationFile(h, &iosb, &b, sizeof b, FileBasicInformation)) ||
        !NT_SUCCESS(st = NtQueryInformationFile(h, &iosb, &s, sizeof s, FileStandardInformation))) { k32_set_nt_error(st); return FALSE; }
    memset(&idx, 0, sizeof idx);
    NtQueryInformationFile(h, &iosb, &idx, sizeof idx, FileInternalInformation);       /* best effort: not every device has an index */
    memset(info, 0, sizeof *info);
    info->dwFileAttributes = b.FileAttributes;
    ft_from_li(&info->ftCreationTime, b.CreationTime); ft_from_li(&info->ftLastAccessTime, b.LastAccessTime); ft_from_li(&info->ftLastWriteTime, b.LastWriteTime);
    info->nFileSizeLow = (DWORD)s.EndOfFile.QuadPart; info->nFileSizeHigh = (DWORD)((uint64_t)s.EndOfFile.QuadPart >> 32);
    info->nNumberOfLinks = s.NumberOfLinks;
    info->nFileIndexLow = (DWORD)idx.IndexNumber.QuadPart; info->nFileIndexHigh = (DWORD)((uint64_t)idx.IndexNumber.QuadPart >> 32);
    return TRUE;
}

BOOL WINAPI GetFileAttributesExW(LPCWSTR name, GET_FILEEX_INFO_LEVELS level, LPVOID out)
{
    static WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    FILE_NETWORK_OPEN_INFORMATION ni;
    WIN32_FILE_ATTRIBUTE_DATA *d = out;
    DWORD err;
    NTSTATUS st;
    if (level != GetFileExInfoStandard || !out || !name || !*name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if ((err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us)) != 0) { *k32_last_error_ptr() = err; return FALSE; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);
    st = NtQueryFullAttributesFile(&oa, &ni);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    d->dwFileAttributes = ni.FileAttributes;
    ft_from_li(&d->ftCreationTime, ni.CreationTime); ft_from_li(&d->ftLastAccessTime, ni.LastAccessTime); ft_from_li(&d->ftLastWriteTime, ni.LastWriteTime);
    d->nFileSizeLow = (DWORD)ni.EndOfFile.QuadPart; d->nFileSizeHigh = (DWORD)((uint64_t)ni.EndOfFile.QuadPart >> 32);
    return TRUE;
}

BOOL WINAPI SetFileAttributesW(LPCWSTR name, DWORD attrs)
{
    static WCHAR buf[K32_PATH_MAX];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    FILE_BASIC_INFORMATION b;
    HANDLE h = NULL;
    DWORD err;
    NTSTATUS st;
    if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return FALSE; }
    if ((err = k32_dos_to_nt(name, buf, K32_PATH_MAX, &us)) != 0) { *k32_last_error_ptr() = err; return FALSE; }
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);
    st = NtOpenFile(&h, FILE_WRITE_ATTRIBUTES | SYNCHRONIZE, &oa, &iosb, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    memset(&b, 0, sizeof b);                                           /* zero time fields mean "leave unchanged" */
    b.FileAttributes = (attrs & 0x31A7u) ? (attrs & 0x31A7u) : FILE_ATTRIBUTE_NORMAL;
    st = NtSetInformationFile(h, &iosb, &b, sizeof b, FileBasicInformation);
    NtClose(h);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI GetFileTime(HANDLE h, FILETIME *created, FILETIME *accessed, FILETIME *written)
{
    IO_STATUS_BLOCK iosb;
    FILE_BASIC_INFORMATION b;
    NTSTATUS st;
    if (h == NULL || h == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtQueryInformationFile(h, &iosb, &b, sizeof b, FileBasicInformation);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    if (created) ft_from_li(created, b.CreationTime);
    if (accessed) ft_from_li(accessed, b.LastAccessTime);
    if (written) ft_from_li(written, b.LastWriteTime);
    return TRUE;
}

BOOL WINAPI SetFileTime(HANDLE h, const FILETIME *created, const FILETIME *accessed, const FILETIME *written)
{
    IO_STATUS_BLOCK iosb;
    FILE_BASIC_INFORMATION b;
    NTSTATUS st;
    if (h == NULL || h == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    memset(&b, 0, sizeof b);                                           /* NULL argument = leave that time alone (zero) */
    if (created) b.CreationTime = li_from_ft(created);
    if (accessed) b.LastAccessTime = li_from_ft(accessed);
    if (written) b.LastWriteTime = li_from_ft(written);
    st = NtSetInformationFile(h, &iosb, &b, sizeof b, FileBasicInformation);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

/* ---------- rename / copy ---------- */
BOOL WINAPI CopyFileW(LPCWSTR from, LPCWSTR to, BOOL fail_if_exists)
{
    static uint8_t chunk[65536];
    static volatile int busy;
    HANDLE in, out;
    BOOL ok = FALSE;
    FILETIME c, a, w;
    DWORD attrs, n, wr;
    if (!from || !to) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    in = CreateFileW(from, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, NULL);
    if (in == INVALID_HANDLE_VALUE) return FALSE;
    attrs = GetFileAttributesW(from);
    out = CreateFileW(to, GENERIC_WRITE, 0, NULL, fail_if_exists ? CREATE_NEW : CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN, NULL);
    if (out == INVALID_HANDLE_VALUE) { DWORD e = *k32_last_error_ptr(); CloseHandle(in); *k32_last_error_ptr() = e; return FALSE; }
    while (__atomic_exchange_n(&busy, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause();
    for (;;) {
        if (!ReadFile(in, chunk, sizeof chunk, &n, NULL)) goto done;
        if (n == 0) break;
        if (!WriteFile(out, chunk, n, &wr, NULL) || wr != n) { if (wr != n) *k32_last_error_ptr() = ERROR_DISK_FULL; goto done; }
    }
    if (GetFileTime(in, &c, &a, &w)) SetFileTime(out, &c, &a, &w);
    ok = TRUE;
done:
    __atomic_store_n(&busy, 0, __ATOMIC_RELEASE);
    {   DWORD e = *k32_last_error_ptr();
        CloseHandle(in); CloseHandle(out);
        if (!ok) { DeleteFileW(to); *k32_last_error_ptr() = e; }
        else if (attrs != INVALID_FILE_ATTRIBUTES) SetFileAttributesW(to, attrs & ~(DWORD)FILE_ATTRIBUTE_DIRECTORY);
    }
    return ok;
}

BOOL WINAPI MoveFileExW(LPCWSTR from, LPCWSTR to, DWORD flags)
{
    static WCHAR nt_from[K32_PATH_MAX], nt_to[K32_PATH_MAX];
    static uint8_t ribuf[sizeof(FILE_RENAME_INFORMATION) + 2 * K32_PATH_MAX] __attribute__((aligned(8)));
    UNICODE_STRING uf, ut;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    FILE_RENAME_INFORMATION *ri = (FILE_RENAME_INFORMATION *)ribuf;
    HANDLE h = NULL;
    DWORD err;
    NTSTATUS st;

    if (!from) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (flags & MOVEFILE_DELAY_UNTIL_REBOOT) { *k32_last_error_ptr() = ERROR_NOT_SUPPORTED; return FALSE; }
    if (!to) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }              /* delete-at-reboot form is the unsupported one */
    if ((err = k32_dos_to_nt(from, nt_from, K32_PATH_MAX, &uf)) != 0 || (err = k32_dos_to_nt(to, nt_to, K32_PATH_MAX, &ut)) != 0) { *k32_last_error_ptr() = err; return FALSE; }
    InitializeObjectAttributes(&oa, &uf, OBJ_CASE_INSENSITIVE, NULL, NULL);
    st = NtOpenFile(&h, DELETE | SYNCHRONIZE, &oa, &iosb, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    memset(ri, 0, sizeof *ri);
    ri->ReplaceIfExists = (flags & MOVEFILE_REPLACE_EXISTING) != 0;
    ri->FileNameLength = ut.Length;
    memcpy(ri->FileName, ut.Buffer, ut.Length);
    st = NtSetInformationFile(h, &iosb, ri, (ULONG)(sizeof(FILE_RENAME_INFORMATION) - sizeof(WCHAR) + ut.Length), FileRenameInformation);
    NtClose(h);
    if (st == STATUS_NOT_SAME_DEVICE && (flags & MOVEFILE_COPY_ALLOWED)) {                       /* across volumes: copy then delete */
        DWORD a = GetFileAttributesW(from);
        if (a != INVALID_FILE_ATTRIBUTES && (a & FILE_ATTRIBUTE_DIRECTORY)) { *k32_last_error_ptr() = ERROR_NOT_SAME_DEVICE; return FALSE; }
        if (!CopyFileW(from, to, !(flags & MOVEFILE_REPLACE_EXISTING))) return FALSE;
        return DeleteFileW(from);
    }
    if (st == STATUS_NOT_SAME_DEVICE) { *k32_last_error_ptr() = ERROR_NOT_SAME_DEVICE; return FALSE; }
    if (!NT_SUCCESS(st)) { if (st == STATUS_OBJECT_NAME_COLLISION) *k32_last_error_ptr() = ERROR_ALREADY_EXISTS; else k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
BOOL WINAPI MoveFileW(LPCWSTR from, LPCWSTR to) { return MoveFileExW(from, to, 0); }

/* ---------- temporary names, drives, process type ---------- */
UINT WINAPI GetTempFileNameW(LPCWSTR dir, LPCWSTR prefix, UINT unique, LPWSTR out)
{
    static const char hex[] = "0123456789abcdef";
    WCHAR path[K32_LOCAL_MAX];
    size_t dl, i, pos;
    UINT n = unique, attempts = 0;
    if (!dir || !prefix || !out) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    dl = wlen(dir);
    if (dl + 24 >= K32_LOCAL_MAX || dl + 14 > MAX_PATH) { *k32_last_error_ptr() = ERROR_BUFFER_OVERFLOW; return 0; }
    if (n == 0) { LARGE_INTEGER c; QueryPerformanceCounter(&c); n = (UINT)(c.QuadPart & 0xFFFF) | 1u; }
    for (;;) {
        memcpy(path, dir, dl * sizeof(WCHAR)); pos = dl;
        if (pos && path[pos - 1] != '\\') path[pos++] = '\\';
        for (i = 0; i < 3 && prefix[i]; i++) path[pos++] = prefix[i];
        for (i = 4; i > 0; i--) path[pos++] = (WCHAR)hex[(n >> ((i - 1) * 4)) & 0xF];
        path[pos++] = '.'; path[pos++] = 't'; path[pos++] = 'm'; path[pos++] = 'p'; path[pos] = 0;
        if (unique) break;                                              /* caller chose the number: no existence check, no creation */
        {   HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
            if (h != INVALID_HANDLE_VALUE) { CloseHandle(h); break; }
            if (*k32_last_error_ptr() != ERROR_FILE_EXISTS || ++attempts > 0xFFFF) return 0;
            n = (n + 1) & 0xFFFF; if (!n) n = 1;
        }
    }
    memcpy(out, path, (pos + 1) * sizeof(WCHAR));
    return n;
}

UINT WINAPI GetDriveTypeW(LPCWSTR root)
{
    WCHAR probe[K32_LOCAL_MAX];
    if (!root) { if (!GetCurrentDirectoryW(K32_LOCAL_MAX, probe)) return DRIVE_UNKNOWN; root = probe; }
    if (!((root[0] >= 'A' && root[0] <= 'Z') || (root[0] >= 'a' && root[0] <= 'z')) || root[1] != ':') return DRIVE_NO_ROOT_DIR;
    { WCHAR r[4] = { root[0], ':', '\\', 0 };
      return GetFileAttributesW(r) == INVALID_FILE_ATTRIBUTES ? DRIVE_NO_ROOT_DIR : DRIVE_FIXED; }               /* LoOS volumes are all fixed btrfs volumes */
}

BOOL WINAPI IsWow64Process(HANDLE process, BOOL *wow64) { (void)process; if (!wow64) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; } *wow64 = FALSE; return TRUE; }
UINT WINAPI SetErrorMode(UINT mode) { static UINT cur; UINT old = cur; cur = mode; return old; }
