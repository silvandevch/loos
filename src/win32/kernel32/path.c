/* DOS path -> full path -> NT path.
 *
 * Rules implemented (RtlGetFullPathName_U / RtlDosPathNameToNtPathName_U semantics):
 *  - "\\?\..." and "\??\..."  : verbatim, no normalisation, becomes "\??\..."
 *  - "\\.\NAME"               : device namespace, "\??\NAME"
 *  - "\\server\share\x"       : UNC, "\??\UNC\server\share\x"
 *  - "C:\x" / "C:x" / "\x" / "x" : resolved against the current directory
 *  - "." and ".." removed, repeated and forward slashes collapsed, trailing dots
 *    and spaces trimmed from each component, a trailing separator is kept
 *  - reserved device names (CON PRN AUX NUL COMn LPTn) as the last component,
 *    with or without extension, map to the device namespace
 */
#include "k32_internal.h"
#include <string.h>

/* ---- current directory (process-wide, spinlock-protected) ---- */
static WCHAR cwd[K32_PATH_MAX] = { 'C', ':', '\\', 0 };
static volatile int cwd_lock;
static void lock_cwd(void) { while (__atomic_exchange_n(&cwd_lock, 1, __ATOMIC_ACQUIRE)) { } }
static void unlock_cwd(void) { __atomic_store_n(&cwd_lock, 0, __ATOMIC_RELEASE); }

static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }
static WCHAR up(WCHAR c) { return (c >= 'a' && c <= 'z') ? (WCHAR)(c - 32) : c; }
static int issep(WCHAR c) { return c == '\\' || c == '/'; }
static int starts_with(const WCHAR *s, const char *lit)
{ for (; *lit; lit++, s++) if (!*s || (*s != (WCHAR)(unsigned char)*lit && !(issep(*s) && issep((WCHAR)(unsigned char)*lit)))) return 0; return 1; }

/* Is the last path component a reserved DOS device name? Returns the name length or 0. */
static size_t device_name(const WCHAR *p, size_t n, size_t *start)
{
    size_t s = n, e, len;
    static const char *names[] = { "CON", "PRN", "AUX", "NUL" };
    size_t i;
    while (s > 0 && !issep(p[s - 1]) && p[s - 1] != ':') s--;
    e = s;
    while (e < n && p[e] != '.' && p[e] != ':') e++;
    while (e > s && p[e - 1] == ' ') e--;
    len = e - s;
    if (len == 3) for (i = 0; i < 4; i++)
        if (up(p[s]) == names[i][0] && up(p[s + 1]) == names[i][1] && up(p[s + 2]) == names[i][2]) { *start = s; return 3; }
    if (len == 6 && up(p[s]) == 'C' && up(p[s + 1]) == 'O' && up(p[s + 2]) == 'N' && up(p[s + 3]) == 'I' && up(p[s + 4]) == 'N' && p[s + 5] == '$') { *start = s; return 6; }
    if (len == 7 && up(p[s]) == 'C' && up(p[s + 1]) == 'O' && up(p[s + 2]) == 'N' && up(p[s + 3]) == 'O' && up(p[s + 4]) == 'U' && up(p[s + 5]) == 'T' && p[s + 6] == '$') { *start = s; return 7; }
    if (len == 4 && p[s + 3] >= '1' && p[s + 3] <= '9' &&
        ((up(p[s]) == 'C' && up(p[s + 1]) == 'O' && up(p[s + 2]) == 'M') ||
         (up(p[s]) == 'L' && up(p[s + 1]) == 'P' && up(p[s + 2]) == 'T'))) { *start = s; return 4; }
    return 0;
}

/* Normalise `in` (everything after the root prefix) into out at *pos. `floor` is the
 * minimum *pos that ".." must not cross. Components separated by '\'. */
static int normalise_tail(const WCHAR *in, WCHAR *out, size_t cap, size_t *pos, size_t floor_pos, int keep_trailing)
{
    size_t i = 0, n = wlen(in);
    int trailing = n > 0 && issep(in[n - 1]);
    while (i < n) {
        size_t s = i, e, comp_end;
        while (i < n && !issep(in[i])) i++;
        e = i;
        while (i < n && issep(in[i])) i++;
        comp_end = e;
        if (e - s == 1 && in[s] == '.') continue;                 /* "." */
        if (e - s == 2 && in[s] == '.' && in[s + 1] == '.') {     /* ".." */
            size_t p = *pos;
            if (p > floor_pos) {
                if (out[p - 1] == '\\') p--;
                while (p > floor_pos && out[p - 1] != '\\') p--;
                *pos = p;
            }
            continue;
        }
        while (comp_end > s && (in[comp_end - 1] == '.' || in[comp_end - 1] == ' ')) comp_end--;
        if (comp_end == s) continue;                              /* component was only dots/spaces */
        if (*pos + (comp_end - s) + 2 > cap) return 0;
        if (*pos == 0 || out[*pos - 1] != '\\') out[(*pos)++] = '\\';
        memcpy(out + *pos, in + s, (comp_end - s) * sizeof(WCHAR));
        *pos += comp_end - s;
    }
    if (trailing && keep_trailing && *pos > 0 && out[*pos - 1] != '\\') {
        if (*pos + 1 >= cap) return 0;
        out[(*pos)++] = '\\';
    }
    out[*pos] = 0;
    return 1;
}

size_t k32_full_path(LPCWSTR dos, WCHAR *out, size_t cap, size_t *file_part, DWORD *err)
{
    size_t n, pos = 0, floor_pos, dev_start;
    WCHAR local_cwd[K32_LOCAL_MAX];
    const WCHAR *tail;
    int trailing;

    *err = 0;
    if (!dos || !*dos) { *err = ERROR_PATH_NOT_FOUND; return 0; }
    n = wlen(dos);
    if (n >= K32_PATH_MAX) { *err = ERROR_FILENAME_EXCED_RANGE; return 0; }
    if (cap < 4) { *err = ERROR_INSUFFICIENT_BUFFER; return 0; }

    /* verbatim / device prefixes: copied untouched */
    if (starts_with(dos, "\\\\?\\") || starts_with(dos, "\\??\\") || starts_with(dos, "\\\\.\\")) {
        if (n + 1 > cap) { *err = ERROR_INSUFFICIENT_BUFFER; return 0; }
        memcpy(out, dos, (n + 1) * sizeof(WCHAR));
        out[1] = '\\';                               /* "\??\" -> "\\?\" */
        if (file_part) { size_t k = n; while (k > 0 && out[k - 1] != '\\') k--; *file_part = k < n ? k : 0; }
        return n;
    }

    /* a bare reserved device name (no directory part) maps to \\.\NAME */
    if (device_name(dos, n, &dev_start) && dev_start == 0) {
        size_t dl = device_name(dos, n, &dev_start), i;
        if (4 + dl + 1 > cap) { *err = ERROR_INSUFFICIENT_BUFFER; return 0; }
        memcpy(out, (const WCHAR[]){ '\\', '\\', '.', '\\' }, 4 * sizeof(WCHAR));
        for (i = 0; i < dl; i++) out[4 + i] = up(dos[i]);
        out[4 + dl] = 0;
        if (file_part) *file_part = 4;
        return 4 + dl;
    }

    trailing = issep(dos[n - 1]);
    if (issep(dos[0]) && issep(dos[1])) {                       /* UNC: \\server\share[\...] */
        size_t i = 2, srv_end, shr_end;
        while (dos[i] && !issep(dos[i])) i++;
        srv_end = i;
        if (srv_end == 2) { *err = ERROR_BAD_PATHNAME; return 0; }
        while (issep(dos[i])) i++;
        while (dos[i] && !issep(dos[i])) i++;
        shr_end = i;
        if (shr_end == srv_end || shr_end == srv_end + 1) { *err = ERROR_BAD_PATHNAME; return 0; }
        if (shr_end + 2 > cap) { *err = ERROR_INSUFFICIENT_BUFFER; return 0; }
        out[0] = out[1] = '\\';
        memcpy(out + 2, dos + 2, (srv_end - 2) * sizeof(WCHAR));
        pos = 2 + (srv_end - 2);
        out[pos++] = '\\';
        { size_t s = srv_end; while (issep(dos[s])) s++; memcpy(out + pos, dos + s, (shr_end - s) * sizeof(WCHAR)); pos += shr_end - s; }
        out[pos] = 0;
        floor_pos = pos;
        tail = dos + shr_end;
    } else {
        /* get a stable copy of the cwd */
        lock_cwd();
        { size_t cl = wlen(cwd); if (cl >= K32_LOCAL_MAX) cl = K32_LOCAL_MAX - 1; memcpy(local_cwd, cwd, cl * sizeof(WCHAR)); local_cwd[cl] = 0; }
        unlock_cwd();

        if (n >= 2 && dos[1] == ':' && ((dos[0] >= 'A' && dos[0] <= 'Z') || (dos[0] >= 'a' && dos[0] <= 'z'))) {
            out[0] = up(dos[0]); out[1] = ':'; pos = 2; floor_pos = 2;
            if (issep(dos[2])) {                                 /* C:\... absolute */
                tail = dos + 2;
            } else {                                             /* C:rel */
                tail = dos + 2;
                if (up(local_cwd[0]) == out[0] && local_cwd[1] == ':') {
                    if (!normalise_tail(local_cwd + 2, out, cap, &pos, floor_pos, 0)) { *err = ERROR_FILENAME_EXCED_RANGE; return 0; }
                }
            }
        } else if (issep(dos[0])) {                              /* \rooted on current drive */
            out[0] = local_cwd[0]; out[1] = ':'; pos = 2; floor_pos = 2; tail = dos;
        } else {                                                 /* relative */
            out[0] = local_cwd[0]; out[1] = ':'; pos = 2; floor_pos = 2; tail = dos;
            if (!normalise_tail(local_cwd + 2, out, cap, &pos, floor_pos, 0)) { *err = ERROR_FILENAME_EXCED_RANGE; return 0; }
        }
    }
    if (!normalise_tail(tail, out, cap, &pos, floor_pos, trailing)) { *err = ERROR_FILENAME_EXCED_RANGE; return 0; }
    if (pos == floor_pos && out[0] != '\\') { if (pos + 2 > cap) { *err = ERROR_INSUFFICIENT_BUFFER; return 0; } out[pos++] = '\\'; out[pos] = 0; }   /* "C:" -> "C:\" */
    if (file_part) {
        size_t k = pos;
        if (pos > 0 && out[pos - 1] == '\\') *file_part = 0;
        else { while (k > 0 && out[k - 1] != '\\') k--; *file_part = k; }
    }
    return pos;
}

DWORD k32_dos_to_nt(LPCWSTR dos, WCHAR *out, size_t cap, UNICODE_STRING *us)
{
    WCHAR *full = out + cap / 2;              /* scratch: upper half of the caller's buffer */
    size_t half = cap / 2, n;
    DWORD err;
    static const WCHAR nt_prefix[] = { '\\', '?', '?', '\\' };
    static const WCHAR unc_prefix[] = { '\\', '?', '?', '\\', 'U', 'N', 'C', '\\' };

    n = k32_full_path(dos, full, half, NULL, &err);
    if (!n) return err ? err : ERROR_PATH_NOT_FOUND;
    if (full[0] == '\\' && full[1] == '\\' && (full[2] == '?' || full[2] == '.') && full[3] == '\\') {
        /* \\?\X or \\.\X -> \??\X */
        if (n - 4 + 4 + 1 > cap / 2) return ERROR_FILENAME_EXCED_RANGE;
        memcpy(out, nt_prefix, sizeof nt_prefix);
        memmove(out + 4, full + 4, (n - 4 + 1) * sizeof(WCHAR));
        n = n - 4 + 4;
    } else if (full[0] == '\\' && full[1] == '\\') {
        if (n - 2 + 8 + 1 > cap / 2) return ERROR_FILENAME_EXCED_RANGE;
        memmove(out + 8, full + 2, (n - 2 + 1) * sizeof(WCHAR));
        memcpy(out, unc_prefix, sizeof unc_prefix);
        n = n - 2 + 8;
    } else {
        if (n + 4 + 1 > cap / 2) return ERROR_FILENAME_EXCED_RANGE;
        memmove(out + 4, full, (n + 1) * sizeof(WCHAR));
        memcpy(out, nt_prefix, sizeof nt_prefix);
        n += 4;
    }
    us->Buffer = out;
    us->Length = (USHORT)(n * sizeof(WCHAR));
    us->MaximumLength = (USHORT)((n + 1) * sizeof(WCHAR));
    return 0;
}

DWORD WINAPI GetFullPathNameW(LPCWSTR name, DWORD len, LPWSTR buf, LPWSTR *file_part)
{
    size_t n, fp = 0;
    DWORD err;
    /* use a heap-free path: compute into buf if it fits, else probe size via a local buffer */
    {
        WCHAR local[K32_LOCAL_MAX];
        if (!name || !*name) { *k32_last_error_ptr() = ERROR_PATH_NOT_FOUND; return 0; }
        n = k32_full_path(name, local, K32_LOCAL_MAX, &fp, &err);
        if (!n) {
            if (err == ERROR_INSUFFICIENT_BUFFER) err = ERROR_FILENAME_EXCED_RANGE;
            *k32_last_error_ptr() = err;
            return 0;
        }
        if (n + 1 > len) return (DWORD)(n + 1);             /* required size incl. NUL */
        memcpy(buf, local, (n + 1) * sizeof(WCHAR));
        if (file_part) *file_part = fp ? buf + fp : NULL;
        return (DWORD)n;
    }
}

DWORD WINAPI GetCurrentDirectoryW(DWORD len, LPWSTR buf)
{
    size_t n;
    lock_cwd();
    n = wlen(cwd);
    if (n + 1 <= len) { memcpy(buf, cwd, (n + 1) * sizeof(WCHAR)); unlock_cwd(); return (DWORD)n; }
    unlock_cwd();
    return (DWORD)(n + 1);
}

BOOL WINAPI SetCurrentDirectoryW(LPCWSTR dir)
{
    WCHAR full[K32_LOCAL_MAX];
    size_t n;
    DWORD err, attr;
    n = k32_full_path(dir, full, K32_LOCAL_MAX, NULL, &err);
    if (!n) { *k32_last_error_ptr() = err; return FALSE; }
    attr = GetFileAttributesW(full);
    if (attr == INVALID_FILE_ATTRIBUTES) return FALSE;
    if (!(attr & FILE_ATTRIBUTE_DIRECTORY)) { *k32_last_error_ptr() = ERROR_DIRECTORY; return FALSE; }
    if (full[n - 1] != '\\' && n + 2 < K32_LOCAL_MAX) { full[n++] = '\\'; full[n] = 0; }
    lock_cwd();
    memcpy(cwd, full, (n + 1) * sizeof(WCHAR));
    unlock_cwd();
    return TRUE;
}

/* Named kernel objects live under \BaseNamedObjects\ (single session for now). */
static size_t session_prefix_len(LPCWSTR n)
{
    static const char *const pre[] = { "local\\", "global\\" };
    size_t i, k;
    for (i = 0; i < 2; i++) {
        for (k = 0; pre[i][k] && n[k] && (pre[i][k] == '\\' ? n[k] == '\\' : (n[k] | 0x20) == (WCHAR)(unsigned char)pre[i][k]); k++) ;
        if (!pre[i][k]) return k;
    }
    return 0;
}

DWORD k32_object_attrs(SECURITY_ATTRIBUTES *sa, LPCWSTR name, WCHAR *buf, size_t cap,
                       UNICODE_STRING *us, OBJECT_ATTRIBUTES *oa)
{
    static const char prefix[] = "\\BaseNamedObjects\\";
    ULONG attrs = OBJ_CASE_INSENSITIVE;
    if (sa && sa->bInheritHandle) attrs |= OBJ_INHERIT;
    if (name && *name) {
        size_t pl = sizeof prefix - 1, nl, i;
        name += session_prefix_len(name);   /* Local\\ and Global\\ name the same directory: LoOS has one session */
        nl = wlen(name);
        if (pl + nl + 1 > cap || (pl + nl) * 2 > 0xFFFE) return ERROR_FILENAME_EXCED_RANGE;
        for (i = 0; i < nl; i++) if (name[i] == '\\') return ERROR_PATH_NOT_FOUND;   /* names may not contain '\' */
        for (i = 0; i < pl; i++) buf[i] = (WCHAR)(unsigned char)prefix[i];
        memcpy(buf + pl, name, (nl + 1) * sizeof(WCHAR));
        us->Buffer = buf; us->Length = (USHORT)((pl + nl) * 2); us->MaximumLength = (USHORT)(us->Length + 2);
        InitializeObjectAttributes(oa, us, attrs, NULL, sa ? sa->lpSecurityDescriptor : NULL);
    } else {
        InitializeObjectAttributes(oa, NULL, attrs, NULL, sa ? sa->lpSecurityDescriptor : NULL);
    }
    return 0;
}
