/* Well-known directories, executable search, module file names. */
#include "k32_internal.h"
#include <string.h>

#define SYSTEM_DIR  "C:\\Windows\\System32"
#define WINDOWS_DIR "C:\\Windows"

static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }

static UINT copy_ascii_dir(const char *dir, LPWSTR buf, UINT len)
{
    size_t n = strlen(dir), i;
    if (n + 1 > len) return (UINT)(n + 1);                      /* required size including the NUL */
    for (i = 0; i <= n; i++) buf[i] = (WCHAR)(unsigned char)dir[i];
    return (UINT)n;
}
UINT WINAPI GetSystemDirectoryW(LPWSTR buf, UINT len) { return copy_ascii_dir(SYSTEM_DIR, buf, len); }
UINT WINAPI GetWindowsDirectoryW(LPWSTR buf, UINT len) { return copy_ascii_dir(WINDOWS_DIR, buf, len); }
UINT WINAPI GetSystemWindowsDirectoryW(LPWSTR buf, UINT len) { return copy_ascii_dir(WINDOWS_DIR, buf, len); }

DWORD WINAPI GetTempPathW(DWORD len, LPWSTR buf)
{
    static const WCHAR tmp[] = { 'T', 'M', 'P', 0 }, temp[] = { 'T', 'E', 'M', 'P', 0 }, prof[] = { 'U', 'S', 'E', 'R', 'P', 'R', 'O', 'F', 'I', 'L', 'E', 0 };
    WCHAR dir[K32_LOCAL_MAX];
    DWORD n = 0;
    size_t i;
    if (!(n = GetEnvironmentVariableW(tmp, dir, K32_LOCAL_MAX)) || n >= K32_LOCAL_MAX)
        if (!(n = GetEnvironmentVariableW(temp, dir, K32_LOCAL_MAX)) || n >= K32_LOCAL_MAX)
            if (!(n = GetEnvironmentVariableW(prof, dir, K32_LOCAL_MAX)) || n >= K32_LOCAL_MAX) {
                n = (DWORD)strlen(WINDOWS_DIR "\\Temp");
                for (i = 0; i <= n; i++) dir[i] = (WCHAR)(unsigned char)(WINDOWS_DIR "\\Temp")[i];
            }
    if (n && dir[n - 1] != '\\') dir[n++] = '\\';                // always ends with a separator
    dir[n] = 0;
    if (n + 1 > len) return n + 1;
    memcpy(buf, dir, (n + 1) * sizeof(WCHAR));
    return n;
}

BOOL WINAPI GetComputerNameW(LPWSTR buf, LPDWORD len)
{
    static const WCHAR var[] = { 'C', 'O', 'M', 'P', 'U', 'T', 'E', 'R', 'N', 'A', 'M', 'E', 0 };
    WCHAR name[64];
    DWORD n = GetEnvironmentVariableW(var, name, 64);
    if (!len) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (!n || n >= 64) { static const WCHAR dflt[] = { 'L', 'O', 'O', 'S', '-', 'P', 'C', 0 }; memcpy(name, dflt, sizeof dflt); n = 7; }
    if (!buf || *len < n + 1) { *len = n + 1; *k32_last_error_ptr() = ERROR_BUFFER_OVERFLOW; return FALSE; }
    memcpy(buf, name, (n + 1) * sizeof(WCHAR));
    *len = n;
    return TRUE;
}

/* ---------- SearchPathW ---------- */
static int has_sep(const WCHAR *s) { for (; *s; s++) if (*s == '\\' || *s == '/' || *s == ':') return 1; return 0; }
static int has_ext(const WCHAR *s)
{
    size_t n = wlen(s);
    while (n > 0 && s[n - 1] != '\\' && s[n - 1] != '/') { if (s[n - 1] == '.') return 1; n--; }
    return 0;
}
static int is_file(const WCHAR *path)
{
    DWORD a = GetFileAttributesW(path);
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

/* try dir\name[ext]; on success put the full path in out */
static int try_dir(const WCHAR *dir, size_t dlen, const WCHAR *name, const WCHAR *ext, WCHAR *out)
{
    WCHAR cand[K32_LOCAL_MAX];
    size_t nl = wlen(name), el = (ext && !has_ext(name)) ? wlen(ext) : 0, pos = 0;
    DWORD n;
    if (dlen + nl + el + 2 >= K32_LOCAL_MAX) return 0;
    if (dlen) { memcpy(cand, dir, dlen * sizeof(WCHAR)); pos = dlen; if (cand[pos - 1] != '\\' && cand[pos - 1] != '/') cand[pos++] = '\\'; }
    memcpy(cand + pos, name, nl * sizeof(WCHAR)); pos += nl;
    if (el) { memcpy(cand + pos, ext, el * sizeof(WCHAR)); pos += el; }
    cand[pos] = 0;
    if (!is_file(cand)) return 0;
    n = GetFullPathNameW(cand, K32_LOCAL_MAX, out, NULL);
    return n != 0 && n < K32_LOCAL_MAX;
}

static DWORD finish_search(const WCHAR *found, DWORD len, LPWSTR buf, LPWSTR *file_part)
{
    size_t n = wlen(found), fp = n;
    while (fp > 0 && found[fp - 1] != '\\') fp--;
    if (n + 1 > len) return (DWORD)(n + 1);
    memcpy(buf, found, (n + 1) * sizeof(WCHAR));
    if (file_part) *file_part = buf + fp;
    return (DWORD)n;
}

DWORD WINAPI SearchPathW(LPCWSTR path, LPCWSTR name, LPCWSTR ext, DWORD len, LPWSTR buf, LPWSTR *file_part)
{
    WCHAR found[K32_LOCAL_MAX], dir[K32_LOCAL_MAX];
    WCHAR sysdir[64], windir[64], var[8192];
    static const WCHAR pathname[] = { 'P', 'A', 'T', 'H', 0 };
    const WCHAR *p;
    size_t i;

    if (!name || !*name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (has_sep(name)) {                                         /* explicit directory: no searching */
        if (try_dir(NULL, 0, name, ext, found)) return finish_search(found, len, buf, file_part);
        *k32_last_error_ptr() = ERROR_FILE_NOT_FOUND;
        return 0;
    }
    if (path) {                                                  /* caller-supplied list */
        for (p = path; *p; ) {
            for (i = 0; p[i] && p[i] != ';'; i++) { }
            if (i && i < K32_LOCAL_MAX) { memcpy(dir, p, i * sizeof(WCHAR)); if (try_dir(dir, i, name, ext, found)) return finish_search(found, len, buf, file_part); }
            p += i; if (*p == ';') p++;
        }
        *k32_last_error_ptr() = ERROR_FILE_NOT_FOUND;
        return 0;
    }
    {   /* the standard order: application directory, current directory, system, windows, then PATH */
        RTL_USER_PROCESS_PARAMETERS *pp = k32_params();
        if (pp && pp->ImagePathName.Buffer) {
            size_t l = pp->ImagePathName.Length / sizeof(WCHAR);
            while (l > 0 && pp->ImagePathName.Buffer[l - 1] != '\\') l--;
            if (l && l < K32_LOCAL_MAX && try_dir(pp->ImagePathName.Buffer, l, name, ext, found)) return finish_search(found, len, buf, file_part);
        }
    }
    if (try_dir(NULL, 0, name, ext, found)) return finish_search(found, len, buf, file_part);          /* relative: current directory */
    if (GetSystemDirectoryW(sysdir, 64) < 64 && try_dir(sysdir, wlen(sysdir), name, ext, found)) return finish_search(found, len, buf, file_part);
    if (GetWindowsDirectoryW(windir, 64) < 64 && try_dir(windir, wlen(windir), name, ext, found)) return finish_search(found, len, buf, file_part);
    {
        DWORD vn = GetEnvironmentVariableW(pathname, var, 8192);
        if (vn && vn < 8192)
            for (p = var; *p; ) {
                for (i = 0; p[i] && p[i] != ';'; i++) { }
                if (i && i < K32_LOCAL_MAX) { memcpy(dir, p, i * sizeof(WCHAR)); if (try_dir(dir, i, name, ext, found)) return finish_search(found, len, buf, file_part); }
                p += i; if (*p == ';') p++;
            }
    }
    *k32_last_error_ptr() = ERROR_FILE_NOT_FOUND;
    return 0;
}

/* ---------- module file names ---------- */
DWORD WINAPI GetModuleFileNameW(HMODULE m, LPWSTR buf, DWORD len)
{
    WCHAR full[K32_LOCAL_MAX];
    UNICODE_STRING us;
    size_t n;
    if (!buf || len == 0) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    if (!m) {                                                    /* the executable: from the process parameters */
        RTL_USER_PROCESS_PARAMETERS *pp = k32_params();
        if (!pp || !pp->ImagePathName.Buffer) { *k32_last_error_ptr() = ERROR_MOD_NOT_FOUND; return 0; }
        n = pp->ImagePathName.Length / sizeof(WCHAR);
        if (n >= K32_LOCAL_MAX) n = K32_LOCAL_MAX - 1;
        memcpy(full, pp->ImagePathName.Buffer, n * sizeof(WCHAR)); full[n] = 0;
    } else {
        NTSTATUS st;
        us.Buffer = full; us.Length = 0; us.MaximumLength = sizeof full;
        st = LdrGetDllFullName(m, &us);
        if (!NT_SUCCESS(st)) { *k32_last_error_ptr() = st == STATUS_INVALID_HANDLE ? ERROR_MOD_NOT_FOUND : RtlNtStatusToDosError(st); return 0; }
        n = us.Length / sizeof(WCHAR);
    }
    if (n >= len) {                                              /* truncate like Windows: fill the buffer, report the size, set the error */
        memcpy(buf, full, (len - 1) * sizeof(WCHAR)); buf[len - 1] = 0;
        *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER;
        return len;
    }
    memcpy(buf, full, (n + 1) * sizeof(WCHAR));
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return (DWORD)n;
}

DWORD WINAPI GetModuleFileNameA(HMODULE m, LPSTR buf, DWORD len)
{
    WCHAR w[K32_LOCAL_MAX];
    DWORD n = GetModuleFileNameW(m, w, K32_LOCAL_MAX);
    int bytes;
    if (!buf || len == 0) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    if (n == 0) return 0;
    bytes = WideCharToMultiByte(CP_ACP, 0, w, (int)n, NULL, 0, NULL, NULL);
    if (bytes <= 0) return 0;
    if ((DWORD)bytes >= len) {
        WideCharToMultiByte(CP_ACP, 0, w, (int)n, buf, (int)len - 1, NULL, NULL);
        buf[len - 1] = 0; *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return len;
    }
    WideCharToMultiByte(CP_ACP, 0, w, (int)n, buf, bytes, NULL, NULL);
    buf[bytes] = 0;
    return (DWORD)bytes;
}
