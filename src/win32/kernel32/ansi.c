/* ANSI adapters for the Unicode-first kernel32 implementation. */
#include "k32_internal.h"
#include <string.h>

#define ANSI_CP CP_ACP

static LPWSTR ansi_to_wide(LPCSTR text)
{
    int chars;
    LPWSTR wide;
    if (!text) return NULL;
    chars = MultiByteToWideChar(ANSI_CP, 0, text, -1, NULL, 0);
    if (!chars) return NULL;
    wide = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)chars * sizeof(WCHAR));
    if (!wide) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return NULL; }
    if (!MultiByteToWideChar(ANSI_CP, 0, text, -1, wide, chars)) {
        RtlFreeHeap(RtlGetProcessHeap(), 0, wide);
        return NULL;
    }
    return wide;
}

static BOOL wide_to_ansi(LPCWSTR text, LPSTR out, DWORD capacity, DWORD *length)
{
    int needed;
    if (!text || !out || capacity > 0x7fffffffU) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
        return FALSE;
    }
    needed = WideCharToMultiByte(ANSI_CP, 0, text, -1, NULL, 0, NULL, NULL);
    if (!needed) return FALSE;
    if ((DWORD)needed > capacity) {
        if (length) *length = (DWORD)needed;
        *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER;
        return FALSE;
    }
    if (!WideCharToMultiByte(ANSI_CP, 0, text, -1, out, (int)capacity, NULL, NULL)) return FALSE;
    if (length) *length = (DWORD)needed - 1;
    return TRUE;
}

static void free_wide(LPWSTR wide)
{
    if (wide) RtlFreeHeap(RtlGetProcessHeap(), 0, wide);
}

int WINAPI CompareStringA(DWORD locale, DWORD flags, LPCSTR a, int ca, LPCSTR b, int cb)
{
    LPWSTR wa, wb;
    int result;
    if (!a || !b || ca < -1 || cb < -1) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (ca == -1) wa = ansi_to_wide(a);
    else {
        int n = MultiByteToWideChar(ANSI_CP, 0, a, ca, NULL, 0);
        wa = n > 0 ? RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)n * sizeof(WCHAR)) : NULL;
        if (wa && !MultiByteToWideChar(ANSI_CP, 0, a, ca, wa, n)) { free_wide(wa); wa = NULL; }
    }
    if (cb == -1) wb = ansi_to_wide(b);
    else {
        int n = MultiByteToWideChar(ANSI_CP, 0, b, cb, NULL, 0);
        wb = n > 0 ? RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)n * sizeof(WCHAR)) : NULL;
        if (wb && !MultiByteToWideChar(ANSI_CP, 0, b, cb, wb, n)) { free_wide(wb); wb = NULL; }
    }
    if (!wa || !wb) { free_wide(wa); free_wide(wb); return 0; }
    result = CompareStringW(locale, flags, wa, ca == -1 ? -1 : MultiByteToWideChar(ANSI_CP, 0, a, ca, NULL, 0),
                            wb, cb == -1 ? -1 : MultiByteToWideChar(ANSI_CP, 0, b, cb, NULL, 0));
    free_wide(wa); free_wide(wb);
    return result;
}

int WINAPI LCMapStringA(DWORD locale, DWORD flags, LPCSTR src, int count, LPSTR dst, int capacity)
{
    LPWSTR ws = NULL, mapped = NULL;
    int wn, mn, needed;
    if (!src || count < -1 || capacity < 0 || (!dst && capacity)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    wn = count == -1 ? MultiByteToWideChar(ANSI_CP, 0, src, -1, NULL, 0) : MultiByteToWideChar(ANSI_CP, 0, src, count, NULL, 0);
    if (!wn) return 0;
    ws = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)wn * sizeof(WCHAR));
    if (!ws || !MultiByteToWideChar(ANSI_CP, 0, src, count, ws, wn)) { free_wide(ws); return 0; }
    mn = LCMapStringW(locale, flags, ws, count == -1 ? -1 : wn, NULL, 0);
    if (!mn) { free_wide(ws); return 0; }
    mapped = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)mn * sizeof(WCHAR));
    if (!mapped || !LCMapStringW(locale, flags, ws, count == -1 ? -1 : wn, mapped, mn)) { free_wide(ws); free_wide(mapped); return 0; }
    needed = WideCharToMultiByte(ANSI_CP, 0, mapped, count == -1 ? -1 : mn, NULL, 0, NULL, NULL);
    if (!needed) { free_wide(ws); free_wide(mapped); return 0; }
    if (!dst || !capacity) { free_wide(ws); free_wide(mapped); return needed; }
    if (needed > capacity) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; free_wide(ws); free_wide(mapped); return 0; }
    needed = WideCharToMultiByte(ANSI_CP, 0, mapped, count == -1 ? -1 : mn, dst, capacity, NULL, NULL);
    free_wide(ws); free_wide(mapped);
    return needed;
}

DWORD WINAPI GetEnvironmentVariableA(LPCSTR name, LPSTR buffer, DWORD size)
{
    LPWSTR wname = ansi_to_wide(name);
    WCHAR value[32768];
    DWORD n, out = 0;
    if (!wname) return 0;
    n = GetEnvironmentVariableW(wname, value, 32768);
    free_wide(wname);
    if (!n) return 0;
    if (n >= 32768) { *k32_last_error_ptr() = ERROR_BUFFER_OVERFLOW; return 0; }
    if (!buffer || !size) {
        int needed = WideCharToMultiByte(ANSI_CP, 0, value, -1, NULL, 0, NULL, NULL);
        return needed > 0 ? (DWORD)needed : 0;
    }
    if (!wide_to_ansi(value, buffer, size, &out)) return out;
    return out;
}

BOOL WINAPI SetEnvironmentVariableA(LPCSTR name, LPCSTR value)
{
    LPWSTR wn = ansi_to_wide(name), wv = value ? ansi_to_wide(value) : NULL;
    BOOL ok;
    if (!wn || (value && !wv)) { free_wide(wn); free_wide(wv); return FALSE; }
    ok = SetEnvironmentVariableW(wn, wv);
    free_wide(wn); free_wide(wv);
    return ok;
}

DWORD WINAPI ExpandEnvironmentStringsA(LPCSTR src, LPSTR dst, DWORD size)
{
    LPWSTR wsrc = ansi_to_wide(src);
    WCHAR wdst[32768];
    DWORD n, out;
    if (!wsrc) return 0;
    n = ExpandEnvironmentStringsW(wsrc, wdst, 32768);
    free_wide(wsrc);
    if (!n || n > 32768) return n;
    if (!dst || !size) {
        int needed = WideCharToMultiByte(ANSI_CP, 0, wdst, -1, NULL, 0, NULL, NULL);
        return needed > 0 ? (DWORD)needed : 0;
    }
    if (!wide_to_ansi(wdst, dst, size, &out)) return out;
    return out + 1;
}

LPSTR WINAPI GetEnvironmentStringsA(void)
{
    LPWSTR wide = GetEnvironmentStringsW();
    static const WCHAR empty_block[2] = { 0, 0 };
    size_t chars = 0, i;
    int bytes;
    LPSTR ansi;
    if (!wide) return NULL;
    if (!wide[0]) { FreeEnvironmentStringsW(wide); wide = (LPWSTR)empty_block; chars = 2; }
    else {
        while (wide[chars]) {
            for (i = chars; wide[i]; i++) { }
            chars = i + 1;
        }
        chars++;
    }
    bytes = WideCharToMultiByte(ANSI_CP, 0, wide, (int)chars, NULL, 0, NULL, NULL);
    if (bytes <= 0) { if (wide != empty_block) FreeEnvironmentStringsW(wide); return NULL; }
    ansi = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)bytes);
    if (!ansi) { if (wide != empty_block) FreeEnvironmentStringsW(wide); *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return NULL; }
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, (int)chars, ansi, bytes, NULL, NULL)) {
        RtlFreeHeap(RtlGetProcessHeap(), 0, ansi);
        if (wide != empty_block) FreeEnvironmentStringsW(wide);
        return NULL;
    }
    if (wide != empty_block) FreeEnvironmentStringsW(wide);
    return ansi;
}

BOOL WINAPI FreeEnvironmentStringsA(LPSTR block)
{
    if (!block) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    return RtlFreeHeap(RtlGetProcessHeap(), 0, block) != 0;
}

static DWORD path_result_a(DWORD (WINAPI *fn)(DWORD, LPWSTR), DWORD capacity, LPSTR dst)
{
    WCHAR wide[K32_LOCAL_MAX];
    DWORD n = fn(K32_LOCAL_MAX, wide);
    DWORD out = 0;
    if (!n || n >= K32_LOCAL_MAX) return n;
    if (!dst || !capacity) return (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!wide_to_ansi(wide, dst, capacity, &out)) return out;
    return out;
}

DWORD WINAPI GetCurrentDirectoryA(DWORD size, LPSTR dst)
{
    return path_result_a(GetCurrentDirectoryW, size, dst);
}

BOOL WINAPI SetCurrentDirectoryA(LPCSTR path)
{
    LPWSTR wide = ansi_to_wide(path);
    BOOL result;
    if (!wide) return FALSE;
    result = SetCurrentDirectoryW(wide);
    free_wide(wide);
    return result;
}

DWORD WINAPI GetFullPathNameA(LPCSTR path, DWORD size, LPSTR dst, LPSTR *file_part)
{
    LPWSTR wpath = ansi_to_wide(path), wfile = NULL;
    WCHAR wide[K32_LOCAL_MAX];
    DWORD n, out;
    if (!wpath) return 0;
    n = GetFullPathNameW(wpath, K32_LOCAL_MAX, wide, file_part ? &wfile : NULL);
    free_wide(wpath);
    if (!n || n >= K32_LOCAL_MAX) return n;
    out = (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!out) return 0;
    if (!dst || !size) return out;
    if (out > size) return out;
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, -1, dst, (int)size, NULL, NULL)) return 0;
    if (file_part && wfile) *file_part = dst + (WideCharToMultiByte(ANSI_CP, 0, wide, (int)(wfile - wide), NULL, 0, NULL, NULL));
    return out - 1;
}

HANDLE WINAPI CreateFileA(LPCSTR name, DWORD access, DWORD share, SECURITY_ATTRIBUTES *sa,
                          DWORD disposition, DWORD flags_attrs, HANDLE template_file)
{
    LPWSTR wide = ansi_to_wide(name);
    HANDLE result;
    if (!wide) return INVALID_HANDLE_VALUE;
    result = CreateFileW(wide, access, share, sa, disposition, flags_attrs, template_file);
    free_wide(wide);
    return result;
}

DWORD WINAPI GetFileAttributesA(LPCSTR name)
{
    LPWSTR wide = ansi_to_wide(name);
    DWORD result;
    if (!wide) return INVALID_FILE_ATTRIBUTES;
    result = GetFileAttributesW(wide);
    free_wide(wide);
    return result;
}

BOOL WINAPI GetFileAttributesExA(LPCSTR name, GET_FILEEX_INFO_LEVELS level, LPVOID info)
{
    LPWSTR wide = ansi_to_wide(name);
    BOOL result;
    if (!wide) return FALSE;
    result = GetFileAttributesExW(wide, level, info); free_wide(wide); return result;
}

BOOL WINAPI SetFileAttributesA(LPCSTR name, DWORD attrs)
{
    LPWSTR wide = ansi_to_wide(name);
    BOOL result;
    if (!wide) return FALSE;
    result = SetFileAttributesW(wide, attrs); free_wide(wide); return result;
}

static void find_data_a(const WIN32_FIND_DATAW *wide, WIN32_FIND_DATAA *ansi)
{
    memset(ansi, 0, sizeof *ansi);
    ansi->dwFileAttributes = wide->dwFileAttributes;
    ansi->ftCreationTime = wide->ftCreationTime;
    ansi->ftLastAccessTime = wide->ftLastAccessTime;
    ansi->ftLastWriteTime = wide->ftLastWriteTime;
    ansi->nFileSizeHigh = wide->nFileSizeHigh;
    ansi->nFileSizeLow = wide->nFileSizeLow;
    ansi->dwReserved0 = wide->dwReserved0;
    ansi->dwReserved1 = wide->dwReserved1;
    (void)WideCharToMultiByte(ANSI_CP, 0, wide->cFileName, -1, ansi->cFileName, sizeof ansi->cFileName, NULL, NULL);
    (void)WideCharToMultiByte(ANSI_CP, 0, wide->cAlternateFileName, -1, ansi->cAlternateFileName, sizeof ansi->cAlternateFileName, NULL, NULL);
}

HANDLE WINAPI FindFirstFileA(LPCSTR pattern, WIN32_FIND_DATAA *data)
{
    LPWSTR wide_pattern = ansi_to_wide(pattern);
    WIN32_FIND_DATAW wide_data;
    HANDLE h;
    if (!wide_pattern || !data) { free_wide(wide_pattern); *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return INVALID_HANDLE_VALUE; }
    h = FindFirstFileW(wide_pattern, &wide_data);
    free_wide(wide_pattern);
    if (h != INVALID_HANDLE_VALUE) find_data_a(&wide_data, data);
    return h;
}

HANDLE WINAPI FindFirstFileExA(LPCSTR pattern, FINDEX_INFO_LEVELS info, LPVOID data,
                               FINDEX_SEARCH_OPS op, LPVOID filter, DWORD flags)
{
    LPWSTR wide_pattern = ansi_to_wide(pattern);
    WIN32_FIND_DATAW wide_data;
    HANDLE h;
    if (!wide_pattern || !data) { free_wide(wide_pattern); *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return INVALID_HANDLE_VALUE; }
    h = FindFirstFileExW(wide_pattern, info, &wide_data, op, filter, flags);
    free_wide(wide_pattern);
    if (h != INVALID_HANDLE_VALUE) find_data_a(&wide_data, (WIN32_FIND_DATAA *)data);
    return h;
}

BOOL WINAPI FindNextFileA(HANDLE find, WIN32_FIND_DATAA *data)
{
    WIN32_FIND_DATAW wide_data;
    if (!data) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (!FindNextFileW(find, &wide_data)) return FALSE;
    find_data_a(&wide_data, data);
    return TRUE;
}

BOOL WINAPI MoveFileExA(LPCSTR from, LPCSTR to, DWORD flags)
{
    LPWSTR wf = ansi_to_wide(from), wt = ansi_to_wide(to);
    BOOL result;
    if (!wf || !wt) { free_wide(wf); free_wide(wt); return FALSE; }
    result = MoveFileExW(wf, wt, flags); free_wide(wf); free_wide(wt); return result;
}

BOOL WINAPI MoveFileA(LPCSTR from, LPCSTR to) { return MoveFileExA(from, to, 0); }

BOOL WINAPI CopyFileA(LPCSTR from, LPCSTR to, BOOL fail_if_exists)
{
    LPWSTR wf = ansi_to_wide(from), wt = ansi_to_wide(to);
    BOOL result;
    if (!wf || !wt) { free_wide(wf); free_wide(wt); return FALSE; }
    result = CopyFileW(wf, wt, fail_if_exists); free_wide(wf); free_wide(wt); return result;
}

UINT WINAPI GetTempFileNameA(LPCSTR dir, LPCSTR prefix, UINT unique, LPSTR out)
{
    LPWSTR wd = ansi_to_wide(dir), wp = ansi_to_wide(prefix);
    WCHAR wide[K32_LOCAL_MAX];
    UINT result;
    if (!wd || !wp || !out) { free_wide(wd); free_wide(wp); *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    result = GetTempFileNameW(wd, wp, unique, wide);
    free_wide(wd); free_wide(wp);
    if (!result) return 0;
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, -1, out, 260, NULL, NULL)) return 0;
    return result;
}

BOOL WINAPI DeleteFileA(LPCSTR name)
{
    LPWSTR wide = ansi_to_wide(name);
    BOOL result;
    if (!wide) return FALSE;
    result = DeleteFileW(wide); free_wide(wide); return result;
}

BOOL WINAPI CreateDirectoryA(LPCSTR name, SECURITY_ATTRIBUTES *sa)
{
    LPWSTR wide = ansi_to_wide(name);
    BOOL result;
    if (!wide) return FALSE;
    result = CreateDirectoryW(wide, sa); free_wide(wide); return result;
}

BOOL WINAPI RemoveDirectoryA(LPCSTR name)
{
    LPWSTR wide = ansi_to_wide(name);
    BOOL result;
    if (!wide) return FALSE;
    result = RemoveDirectoryW(wide); free_wide(wide); return result;
}

BOOL WINAPI VerifyVersionInfoA(OSVERSIONINFOEXA *info, DWORD type_mask, ULONGLONG condition_mask)
{
    OSVERSIONINFOEXW wide;
    if (!info || info->dwOSVersionInfoSize != sizeof *info) {
        *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER;
        return FALSE;
    }
    memset(&wide, 0, sizeof wide);
    wide.dwOSVersionInfoSize = sizeof wide;
    wide.dwMajorVersion = info->dwMajorVersion;
    wide.dwMinorVersion = info->dwMinorVersion;
    wide.dwBuildNumber = info->dwBuildNumber;
    wide.dwPlatformId = info->dwPlatformId;
    wide.wServicePackMajor = info->wServicePackMajor;
    wide.wServicePackMinor = info->wServicePackMinor;
    wide.wSuiteMask = info->wSuiteMask;
    wide.wProductType = info->wProductType;
    return VerifyVersionInfoW(&wide, type_mask, condition_mask);
}

static DWORD copy_wide_result(UINT (WINAPI *fn)(LPWSTR, UINT), LPSTR dst, DWORD capacity)
{
    WCHAR wide[K32_LOCAL_MAX];
    DWORD n = fn(wide, K32_LOCAL_MAX), out;
    if (!n || n >= K32_LOCAL_MAX) return n;
    out = (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!out) return 0;
    if (!dst || !capacity) return out;
    if (!wide_to_ansi(wide, dst, capacity, &n)) return n + 1;
    return n;
}

UINT WINAPI GetSystemDirectoryA(LPSTR dst, UINT capacity)
{
    return copy_wide_result(GetSystemDirectoryW, dst, capacity);
}

UINT WINAPI GetWindowsDirectoryA(LPSTR dst, UINT capacity)
{
    return copy_wide_result(GetWindowsDirectoryW, dst, capacity);
}

UINT WINAPI GetSystemWindowsDirectoryA(LPSTR dst, UINT capacity)
{
    return copy_wide_result(GetSystemWindowsDirectoryW, dst, capacity);
}

DWORD WINAPI GetTempPathA(DWORD capacity, LPSTR dst)
{
    return path_result_a(GetTempPathW, capacity, dst);
}

BOOL WINAPI GetComputerNameA(LPSTR dst, LPDWORD size)
{
    WCHAR wide[256];
    DWORD n = 256, needed;
    if (!size) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (!GetComputerNameW(wide, &n)) { *size = n; return FALSE; }
    needed = (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!needed) return FALSE;
    if (!dst || *size < needed) {
        *size = needed;
        *k32_last_error_ptr() = ERROR_BUFFER_OVERFLOW;
        return FALSE;
    }
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, -1, dst, (int)*size, NULL, NULL)) return FALSE;
    *size = needed - 1;
    return TRUE;
}

DWORD WINAPI SearchPathA(LPCSTR path, LPCSTR name, LPCSTR ext, DWORD capacity, LPSTR dst, LPSTR *file_part)
{
    LPWSTR wp = path ? ansi_to_wide(path) : NULL;
    LPWSTR wn = ansi_to_wide(name), we = ext ? ansi_to_wide(ext) : NULL, wfile = NULL;
    WCHAR wide[K32_LOCAL_MAX];
    DWORD n, bytes, chars;
    if ((path && !wp) || !wn || (ext && !we)) { free_wide(wp); free_wide(wn); free_wide(we); return 0; }
    n = SearchPathW(wp, wn, we, K32_LOCAL_MAX, wide, file_part ? &wfile : NULL);
    free_wide(wp); free_wide(wn); free_wide(we);
    if (!n || n >= K32_LOCAL_MAX) return n;
    bytes = (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!bytes) return 0;
    if (!dst || !capacity) return bytes;
    if (bytes > capacity) return bytes;
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, -1, dst, (int)capacity, NULL, NULL)) return 0;
    if (file_part && wfile) {
        chars = (DWORD)(wfile - wide);
        *file_part = dst + WideCharToMultiByte(ANSI_CP, 0, wide, (int)chars, NULL, 0, NULL, NULL);
    }
    return bytes - 1;
}

HANDLE WINAPI CreateEventA(SECURITY_ATTRIBUTES *sa, BOOL manual, BOOL initial, LPCSTR name)
{
    LPWSTR wide = name ? ansi_to_wide(name) : NULL;
    HANDLE h;
    if (name && !wide) return NULL;
    h = CreateEventW(sa, manual, initial, wide); free_wide(wide); return h;
}

HANDLE WINAPI CreateMutexA(SECURITY_ATTRIBUTES *sa, BOOL owner, LPCSTR name)
{
    LPWSTR wide = name ? ansi_to_wide(name) : NULL;
    HANDLE h;
    if (name && !wide) return NULL;
    h = CreateMutexW(sa, owner, wide); free_wide(wide); return h;
}

HANDLE WINAPI CreateSemaphoreA(SECURITY_ATTRIBUTES *sa, LONG initial, LONG maximum, LPCSTR name)
{
    LPWSTR wide = name ? ansi_to_wide(name) : NULL;
    HANDLE h;
    if (name && !wide) return NULL;
    h = CreateSemaphoreW(sa, initial, maximum, wide); free_wide(wide); return h;
}

HANDLE WINAPI CreateFileMappingA(HANDLE file, SECURITY_ATTRIBUTES *sa, DWORD protect, DWORD high, DWORD low, LPCSTR name)
{
    LPWSTR wide = name ? ansi_to_wide(name) : NULL;
    HANDLE h;
    if (name && !wide) return NULL;
    h = CreateFileMappingW(file, sa, protect, high, low, wide); free_wide(wide); return h;
}

HANDLE WINAPI OpenFileMappingA(DWORD access, BOOL inherit, LPCSTR name)
{
    LPWSTR wide = ansi_to_wide(name);
    HANDLE h;
    if (!wide) return NULL;
    h = OpenFileMappingW(access, inherit, wide); free_wide(wide); return h;
}

BOOL WINAPI SetConsoleTitleA(LPCSTR title)
{
    LPWSTR wide = ansi_to_wide(title);
    BOOL ok;
    if (!wide) return FALSE;
    ok = SetConsoleTitleW(wide); free_wide(wide); return ok;
}

BOOL WINAPI FillConsoleOutputCharacterA(HANDLE h, char ch, DWORD count, COORD at, LPDWORD written)
{
    WCHAR wide;
    if (MultiByteToWideChar(ANSI_CP, 0, &ch, 1, &wide, 1) != 1) return FALSE;
    return FillConsoleOutputCharacterW(h, wide, count, at, written);
}

UINT WINAPI GetDriveTypeA(LPCSTR root)
{
    LPWSTR wide = root ? ansi_to_wide(root) : NULL;
    UINT result;
    if (root && !wide) return DRIVE_UNKNOWN;
    result = GetDriveTypeW(wide); free_wide(wide); return result;
}

DWORD WINAPI GetConsoleTitleA(LPSTR dst, DWORD capacity)
{
    WCHAR wide[K32_LOCAL_MAX];
    DWORD n = GetConsoleTitleW(wide, K32_LOCAL_MAX), out;
    if (!n) return n;
    out = (DWORD)WideCharToMultiByte(ANSI_CP, 0, wide, -1, NULL, 0, NULL, NULL);
    if (!out) return 0;
    if (!dst || !capacity) return out;
    if (out > capacity) return out;
    if (!WideCharToMultiByte(ANSI_CP, 0, wide, -1, dst, (int)capacity, NULL, NULL)) return 0;
    return out - 1;
}

