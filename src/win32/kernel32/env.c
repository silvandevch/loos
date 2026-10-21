/* Process environment: command line, startup info, standard handles, environment variables.
 * All of it lives in RTL_USER_PROCESS_PARAMETERS, which the process creator fills in.
 *
 * The environment block is a run of "NAME=VALUE\0" entries ended by an extra "\0", kept sorted
 * by upper-cased name. The initial block belongs to the creator; the first modification copies
 * it into the process heap and later modifications replace that copy. Mutation is serialised by a
 * spinlock; readers copy under the same lock. */
#include "k32_internal.h"
#include <string.h>

static volatile int env_lock;
static PVOID owned_env;                 /* the heap copy we allocated, if any */
static void lock_env(void) { while (__atomic_exchange_n(&env_lock, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause(); }
static void unlock_env(void) { __atomic_store_n(&env_lock, 0, __ATOMIC_RELEASE); }

static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }

LPWSTR WINAPI GetCommandLineW(void)
{
    static WCHAR empty[1];
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    return p && p->CommandLine.Buffer ? p->CommandLine.Buffer : empty;
}

/* convert once into the process heap; the result lives for the life of the process */
static LPSTR volatile cl_cache, desk_cache, title_cache;
static LPSTR cached_ansi(LPSTR volatile *slot, const WCHAR *w)
{
    LPSTR cur = __atomic_load_n(slot, __ATOMIC_ACQUIRE), fresh;
    int n;
    if (cur) return cur;
    n = WideCharToMultiByte(CP_ACP, 0, w, -1, NULL, 0, NULL, NULL);
    if (n <= 0) return NULL;
    fresh = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)n);
    if (!fresh) return NULL;
    if (WideCharToMultiByte(CP_ACP, 0, w, -1, fresh, n, NULL, NULL) <= 0) { RtlFreeHeap(RtlGetProcessHeap(), 0, fresh); return NULL; }
    if (!__atomic_compare_exchange_n(slot, &cur, fresh, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) { RtlFreeHeap(RtlGetProcessHeap(), 0, fresh); return cur; }
    return fresh;
}

LPSTR WINAPI GetCommandLineA(void)
{
    static char empty[1];
    LPSTR s = cached_ansi(&cl_cache, GetCommandLineW());
    return s ? s : empty;
}

void WINAPI GetStartupInfoW(STARTUPINFOW *si)
{
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    memset(si, 0, sizeof *si);
    si->cb = sizeof *si;
    if (!p) return;
    si->lpDesktop = p->DesktopInfo.Buffer; si->lpTitle = p->WindowTitle.Buffer;
    si->dwX = p->StartingX; si->dwY = p->StartingY; si->dwXSize = p->CountX; si->dwYSize = p->CountY;
    si->dwXCountChars = p->CountCharsX; si->dwYCountChars = p->CountCharsY; si->dwFillAttribute = p->FillAttribute;
    si->dwFlags = p->WindowFlags; si->wShowWindow = (WORD)p->ShowWindowFlags;
    si->hStdInput = p->StandardInput; si->hStdOutput = p->StandardOutput; si->hStdError = p->StandardError;
}

void WINAPI GetStartupInfoA(STARTUPINFOA *si)
{
    STARTUPINFOW w;
    GetStartupInfoW(&w);
    memset(si, 0, sizeof *si);
    si->cb = sizeof *si;
    si->lpDesktop = w.lpDesktop ? cached_ansi(&desk_cache, w.lpDesktop) : NULL;
    si->lpTitle = w.lpTitle ? cached_ansi(&title_cache, w.lpTitle) : NULL;
    si->dwX = w.dwX; si->dwY = w.dwY; si->dwXSize = w.dwXSize; si->dwYSize = w.dwYSize;
    si->dwXCountChars = w.dwXCountChars; si->dwYCountChars = w.dwYCountChars; si->dwFillAttribute = w.dwFillAttribute;
    si->dwFlags = w.dwFlags; si->wShowWindow = w.wShowWindow;
    si->hStdInput = w.hStdInput; si->hStdOutput = w.hStdOutput; si->hStdError = w.hStdError;
}

static HANDLE *std_slot(RTL_USER_PROCESS_PARAMETERS *p, DWORD which)
{
    if (!p) return NULL;
    return which == STD_INPUT_HANDLE ? &p->StandardInput : which == STD_OUTPUT_HANDLE ? &p->StandardOutput
         : which == STD_ERROR_HANDLE ? &p->StandardError : NULL;
}
HANDLE WINAPI GetStdHandle(DWORD which)
{
    HANDLE *s = std_slot(k32_params(), which);
    if (!s) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return INVALID_HANDLE_VALUE; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return *s;
}
BOOL WINAPI SetStdHandle(DWORD which, HANDLE h)
{
    HANDLE *s = std_slot(k32_params(), which);
    if (!s) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    *s = h;
    return TRUE;
}

/* ---------- environment variables ---------- */
static int name_cmp(const WCHAR *a, size_t an, const WCHAR *b, size_t bn)     /* upper-cased code unit order */
{
    size_t i, n = an < bn ? an : bn;
    for (i = 0; i < n; i++) {
        WCHAR x = k32_towupper(a[i]), y = k32_towupper(b[i]);
        if (x != y) return x < y ? -1 : 1;
    }
    return an == bn ? 0 : (an < bn ? -1 : 1);
}

static size_t entry_name_len(const WCHAR *e, size_t l);

/* locate entry for `name`; returns entry start or NULL. *after = first entry that sorts after it. */
static WCHAR *find_entry(WCHAR *env, const WCHAR *name, size_t nlen, WCHAR **after)
{
    WCHAR *e = env;
    *after = NULL;
    if (!e) return NULL;
    while (*e) {
        size_t l = wlen(e), nl = entry_name_len(e, l);
        int c = name_cmp(e, nl, name, nlen);
        if (c == 0) return e;
        if (c > 0 && !*after) *after = e;
        e += l + 1;
    }
    if (!*after) *after = e;                                        /* the terminator */
    return NULL;
}

static size_t block_size(const WCHAR *env)              /* in WCHARs, including the final terminator */
{
    const WCHAR *e = env;
    if (!env) return 1;
    while (*e) e += wlen(e) + 1;
    return (size_t)(e - env) + 1;
}

DWORD WINAPI GetEnvironmentVariableW(LPCWSTR name, LPWSTR buf, DWORD size)
{
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    size_t nlen = name ? wlen(name) : 0, vlen;
    WCHAR *after, *e;
    DWORD ret;
    if (!name || !nlen || !p) { *k32_last_error_ptr() = ERROR_ENVVAR_NOT_FOUND; return 0; }
    lock_env();
    e = find_entry(p->Environment, name, nlen, &after);
    if (!e) { unlock_env(); *k32_last_error_ptr() = ERROR_ENVVAR_NOT_FOUND; return 0; }
    e += nlen + 1;                                                  /* skip "NAME=" */
    vlen = wlen(e);
    if (buf && size > vlen) { memcpy(buf, e, (vlen + 1) * sizeof(WCHAR)); ret = (DWORD)vlen; }
    else ret = (DWORD)(vlen + 1);
    unlock_env();
    *k32_last_error_ptr() = ERROR_SUCCESS;        /* callers tell "empty" from "missing" by last-error */
    return ret;
}

static size_t entry_name_len(const WCHAR *e, size_t l)
{
    size_t nl = 0;
    while (nl < l && (e[nl] != '=' || nl == 0)) nl++;               /* a leading '=' belongs to the name ("=C:") */
    return nl;
}

BOOL WINAPI SetEnvironmentVariableW(LPCWSTR name, LPCWSTR value)
{
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    size_t nlen = name ? wlen(name) : 0, vlen = value ? wlen(value) : 0, total, i;
    WCHAR *old, *after, *hit, *fresh, *w, *e;
    int inserted = 0;
    HANDLE heap = RtlGetProcessHeap();

    if (!name || !nlen || !p) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    for (i = 1; i < nlen; i++) if (name[i] == '=') { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (name[0] == '=' && nlen == 1) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }

    lock_env();
    old = p->Environment;
    hit = find_entry(old, name, nlen, &after);
    if (!value && !hit) { unlock_env(); *k32_last_error_ptr() = ERROR_ENVVAR_NOT_FOUND; return FALSE; }

    total = block_size(old) + (value ? nlen + 1 + vlen + 1 : 0) - (hit ? wlen(hit) + 1 : 0);
    fresh = RtlAllocateHeap(heap, 0, total * sizeof(WCHAR));
    if (!fresh) { unlock_env(); *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return FALSE; }

    /* one pass over the sorted old block: drop the replaced entry, slot the new one in before the
     * first entry that sorts after it (or at the end) */
    w = fresh;
    for (e = old; e && *e; ) {
        size_t l = wlen(e);
        if (e != hit) {
            if (value && !inserted && name_cmp(e, entry_name_len(e, l), name, nlen) > 0) {
                memcpy(w, name, nlen * sizeof(WCHAR)); w += nlen; *w++ = '=';
                memcpy(w, value, vlen * sizeof(WCHAR)); w += vlen; *w++ = 0;
                inserted = 1;
            }
            memcpy(w, e, (l + 1) * sizeof(WCHAR)); w += l + 1;
        }
        e += l + 1;
    }
    if (value && !inserted) {
        memcpy(w, name, nlen * sizeof(WCHAR)); w += nlen; *w++ = '=';
        memcpy(w, value, vlen * sizeof(WCHAR)); w += vlen; *w++ = 0;
    }
    *w++ = 0;
    p->Environment = fresh;
    p->EnvironmentSize = (SIZE_T)(w - fresh) * sizeof(WCHAR);
    if (old && old == owned_env) RtlFreeHeap(heap, 0, old);
    owned_env = fresh;
    unlock_env();
    return TRUE;
}

LPWSTR WINAPI GetEnvironmentStringsW(void)
{
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    size_t n;
    WCHAR *copy;
    if (!p) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    lock_env();
    n = block_size(p->Environment);
    copy = RtlAllocateHeap(RtlGetProcessHeap(), 0, n * sizeof(WCHAR));
    if (copy) { if (p->Environment) memcpy(copy, p->Environment, n * sizeof(WCHAR)); else copy[0] = 0; }
    unlock_env();
    if (!copy) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return copy;
}

BOOL WINAPI FreeEnvironmentStringsW(LPWSTR block)
{
    if (!block) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    return RtlFreeHeap(RtlGetProcessHeap(), 0, block) != 0;
}

DWORD WINAPI ExpandEnvironmentStringsW(LPCWSTR src, LPWSTR dst, DWORD size)
{
    size_t need = 0, i = 0;
    if (!src) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    for (;;) {                                  /* pass 0 measures, pass 1 writes (only if it fits) */
        int pass;
        for (pass = 0; pass < 2; pass++) {
            size_t out = 0;
            i = 0;
            while (src[i]) {
                if (src[i] == '%') {
                    size_t j = i + 1;
                    while (src[j] && src[j] != '%') j++;
                    if (src[j] == '%' && j > i + 1) {
                        WCHAR name[256], val[4096];
                        DWORD got;
                        size_t nl = j - i - 1;
                        if (nl < 256) {
                            memcpy(name, src + i + 1, nl * sizeof(WCHAR)); name[nl] = 0;
                            got = GetEnvironmentVariableW(name, val, 4096);
                            if (got && got < 4096) {                    /* defined: substitute */
                                if (pass) memcpy(dst + out, val, got * sizeof(WCHAR));
                                out += got; i = j + 1; continue;
                            }
                        }
                        /* undefined: keep the %NAME% text as written */
                        if (pass) memcpy(dst + out, src + i, (j - i + 1) * sizeof(WCHAR));
                        out += j - i + 1; i = j + 1; continue;
                    }
                }
                if (pass) dst[out] = src[i];
                out++; i++;
            }
            if (pass) dst[out] = 0;
            else { need = out + 1; if (!dst || need > size) return (DWORD)need; }
        }
        return (DWORD)need;
    }
}

HMODULE WINAPI GetModuleHandleA(LPCSTR name)
{
    WCHAR w[300];
    if (!name) return GetModuleHandleW(NULL);
    if (MultiByteToWideChar(CP_ACP, 0, name, -1, w, 300) <= 0) return NULL;
    return GetModuleHandleW(w);
}

HMODULE WINAPI LoadLibraryA(LPCSTR name)
{
    WCHAR w[300];
    if (!name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    if (MultiByteToWideChar(CP_ACP, 0, name, -1, w, 300) <= 0) return NULL;
    return LoadLibraryW(w);
}

#ifdef LOOS_HOST_TEST
void k32_env_test_reset(void) { cl_cache = desk_cache = title_cache = NULL; owned_env = NULL; }
#endif
