/* CreateProcessW / CreateProcessA over NtCreateUserProcess.
 *
 * The kernel maps the image and starts the first thread; everything the child will later read through
 * its process parameters (command line, environment, current directory, standard handles, window
 * title, console flags) is assembled here by RtlCreateProcessParametersEx.
 *
 * LoOS console flags travel in RTL_USER_PROCESS_PARAMETERS.ConsoleFlags: 1 = CREATE_NEW_CONSOLE,
 * 2 = DETACHED_PROCESS, 4 = CREATE_NO_WINDOW, 8 = CREATE_NEW_PROCESS_GROUP. */
#include "k32_internal.h"
#include <string.h>

static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }
static int ext_missing(const WCHAR *s, size_t n)
{
    while (n > 0 && s[n - 1] != '\\' && s[n - 1] != '/') { if (s[n - 1] == '.') return 0; n--; }
    return 1;
}
static int has_dir(const WCHAR *s, size_t n) { size_t i; for (i = 0; i < n; i++) if (s[i] == '\\' || s[i] == '/' || s[i] == ':') return 1; return 0; }

static int file_exists(const WCHAR *p)
{
    DWORD a = GetFileAttributesW(p);
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

/* Resolve one candidate file name (n WCHARs, no quotes) to a full path in out. */
static int resolve_candidate(const WCHAR *name, size_t n, WCHAR *out)
{
    static const WCHAR exe[] = { '.', 'e', 'x', 'e', 0 };
    WCHAR tmp[K32_LOCAL_MAX];
    if (n == 0 || n + 5 >= K32_LOCAL_MAX) return 0;
    memcpy(tmp, name, n * sizeof(WCHAR)); tmp[n] = 0;
    if (has_dir(tmp, n)) {
        WCHAR full[K32_LOCAL_MAX];
        if (ext_missing(tmp, n)) { memcpy(tmp + n, exe, sizeof exe); n += 4; }
        if (!GetFullPathNameW(tmp, K32_LOCAL_MAX, full, NULL) || !file_exists(full)) return 0;
        memcpy(out, full, (wlen(full) + 1) * sizeof(WCHAR));
        return 1;
    }
    return SearchPathW(NULL, tmp, ext_missing(tmp, n) ? exe : NULL, K32_LOCAL_MAX, out, NULL) != 0;
}

/* Work out which file to run. Returns 0 and sets *err on failure. */
static int find_image(LPCWSTR app, LPCWSTR cmd, WCHAR *out, DWORD *err)
{
    *err = ERROR_FILE_NOT_FOUND;
    if (app && *app) {
        /* an explicit application name is used as given: relative names resolve against the current directory only */
        static const WCHAR exe[] = { '.', 'e', 'x', 'e', 0 };
        WCHAR tmp[K32_LOCAL_MAX], full[K32_LOCAL_MAX];
        size_t n = wlen(app);
        if (n + 5 >= K32_LOCAL_MAX) { *err = ERROR_FILENAME_EXCED_RANGE; return 0; }
        memcpy(tmp, app, (n + 1) * sizeof(WCHAR));
        if (ext_missing(tmp, n)) { memcpy(tmp + n, exe, sizeof exe); }
        if (!GetFullPathNameW(tmp, K32_LOCAL_MAX, full, NULL)) return 0;
        if (!file_exists(full)) return 0;
        memcpy(out, full, (wlen(full) + 1) * sizeof(WCHAR));
        return 1;
    }
    if (!cmd || !*cmd) { *err = ERROR_INVALID_PARAMETER; return 0; }
    if (cmd[0] == '"') {                                          /* quoted: up to the closing quote */
        size_t e = 1;
        while (cmd[e] && cmd[e] != '"') e++;
        return resolve_candidate(cmd + 1, e - 1, out);
    }
    {   /* unquoted: try each whitespace-delimited prefix, so "C:\Program Files\app.exe /x" works */
        size_t e = 0;
        for (;;) {
            while (cmd[e] && cmd[e] != ' ' && cmd[e] != '\t') e++;
            if (resolve_candidate(cmd, e, out)) return 1;
            if (!cmd[e]) return 0;
            e++;
        }
    }
}

/* ANSI environment block -> UTF-16 block (heap-allocated, double NUL terminated) */
static WCHAR *widen_env(const char *env)
{
    size_t n = 0;
    int wn;
    WCHAR *w;
    while (env[n] || env[n + 1]) n++;
    n += 2;
    wn = MultiByteToWideChar(CP_ACP, 0, env, (int)n, NULL, 0);
    if (wn <= 0) return NULL;
    w = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)wn * sizeof(WCHAR));
    if (w && MultiByteToWideChar(CP_ACP, 0, env, (int)n, w, wn) <= 0) { RtlFreeHeap(RtlGetProcessHeap(), 0, w); return NULL; }
    return w;
}

static void init_us(UNICODE_STRING *u, const WCHAR *s)
{
    size_t n = s ? wlen(s) : 0;
    u->Buffer = (PWSTR)s; u->Length = (USHORT)(n * sizeof(WCHAR)); u->MaximumLength = (USHORT)(u->Length + sizeof(WCHAR));
}

BOOL WINAPI CreateProcessW(LPCWSTR app, LPWSTR cmdline, SECURITY_ATTRIBUTES *psa, SECURITY_ATTRIBUTES *tsa, BOOL inherit,
                           DWORD flags, LPVOID env, LPCWSTR cwd, STARTUPINFOW *si, PROCESS_INFORMATION *pi)
{
    static WCHAR image[K32_LOCAL_MAX], nt_buf[K32_PATH_MAX], full_cwd[K32_LOCAL_MAX];
    static volatile int busy;                 /* serialises callers: the big buffers live in .bss, not on the thread stack */
    UNICODE_STRING nt_image, us_image, us_cwd, us_cmd, us_title, us_desktop;
    RTL_USER_PROCESS_PARAMETERS *params = NULL, *self = k32_params();
    OBJECT_ATTRIBUTES poa, toa;
    PS_ATTRIBUTE_LIST attrs;
    PS_CREATE_INFO info;
    CLIENT_ID cid;
    HANDLE hp = NULL, ht = NULL;
    WCHAR *env_block = NULL, *own_env = NULL;
    NTSTATUS st;
    DWORD err;
    BOOL ok = FALSE;

    if (!pi || !si) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (flags & (DEBUG_PROCESS | DEBUG_ONLY_THIS_PROCESS)) { *k32_last_error_ptr() = ERROR_NOT_SUPPORTED; return FALSE; }
    memset(pi, 0, sizeof *pi);
    while (__atomic_exchange_n(&busy, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause();

    if (!find_image(app, cmdline, image, &err)) { *k32_last_error_ptr() = err; goto out; }
    if ((err = k32_dos_to_nt(image, nt_buf, K32_PATH_MAX, &nt_image)) != 0) { *k32_last_error_ptr() = err; goto out; }
    if (cwd && *cwd) {
        if (!GetFullPathNameW(cwd, K32_LOCAL_MAX, full_cwd, NULL)) goto out;
        { DWORD a = GetFileAttributesW(full_cwd);
          if (a == INVALID_FILE_ATTRIBUTES) goto out;
          if (!(a & FILE_ATTRIBUTE_DIRECTORY)) { *k32_last_error_ptr() = ERROR_DIRECTORY; goto out; } }
    } else if (!GetCurrentDirectoryW(K32_LOCAL_MAX, full_cwd)) goto out;
    {   /* the current-directory field always ends with a separator */
        size_t cl = wlen(full_cwd);
        if (cl && full_cwd[cl - 1] != '\\' && cl + 2 < K32_LOCAL_MAX) { full_cwd[cl] = '\\'; full_cwd[cl + 1] = 0; }
    }

    /* environment: explicit (wide or ANSI) or a copy of ours */
    if (env) {
        if (flags & CREATE_UNICODE_ENVIRONMENT) env_block = env;
        else if (!(own_env = env_block = widen_env(env))) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; goto out; }
    } else if (!(own_env = env_block = GetEnvironmentStringsW())) goto out;

    init_us(&us_image, image);
    init_us(&us_cwd, full_cwd);
    init_us(&us_cmd, (cmdline && *cmdline) ? cmdline : image);
    init_us(&us_title, si->lpTitle ? si->lpTitle : image);
    init_us(&us_desktop, si->lpDesktop);
    st = RtlCreateProcessParametersEx(&params, &us_image, NULL, &us_cwd, &us_cmd, env_block, &us_title, &us_desktop, NULL, NULL, 0);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); goto out; }

    /* startup information, standard handles (explicit, or inherited from this process) */
    params->StartingX = si->dwX; params->StartingY = si->dwY; params->CountX = si->dwXSize; params->CountY = si->dwYSize;
    params->CountCharsX = si->dwXCountChars; params->CountCharsY = si->dwYCountChars; params->FillAttribute = si->dwFillAttribute;
    params->WindowFlags = si->dwFlags; params->ShowWindowFlags = si->wShowWindow;
    if (si->dwFlags & STARTF_USESTDHANDLES) {
        params->StandardInput = si->hStdInput; params->StandardOutput = si->hStdOutput; params->StandardError = si->hStdError;
    } else if (self) {
        params->StandardInput = self->StandardInput; params->StandardOutput = self->StandardOutput; params->StandardError = self->StandardError;
    }
    if (self) params->ConsoleHandle = self->ConsoleHandle;
    params->ConsoleFlags = ((flags & CREATE_NEW_CONSOLE) ? 1u : 0u) | ((flags & DETACHED_PROCESS) ? 2u : 0u) |
                           ((flags & CREATE_NO_WINDOW) ? 4u : 0u) | ((flags & CREATE_NEW_PROCESS_GROUP) ? 8u : 0u);

    memset(&attrs, 0, sizeof attrs); memset(&cid, 0, sizeof cid); memset(&info, 0, sizeof info);
    info.Size = sizeof info;
    attrs.TotalLength = sizeof(SIZE_T) + 2 * sizeof(PS_ATTRIBUTE);
    attrs.Attributes[0].Attribute = PS_ATTRIBUTE_IMAGE_NAME; attrs.Attributes[0].Size = nt_image.Length; attrs.Attributes[0].ValuePtr = nt_image.Buffer;
    attrs.Attributes[1].Attribute = PS_ATTRIBUTE_CLIENT_ID; attrs.Attributes[1].Size = sizeof cid; attrs.Attributes[1].ValuePtr = &cid;
    InitializeObjectAttributes(&poa, NULL, (psa && psa->bInheritHandle) ? OBJ_INHERIT : 0, NULL, psa ? psa->lpSecurityDescriptor : NULL);
    InitializeObjectAttributes(&toa, NULL, (tsa && tsa->bInheritHandle) ? OBJ_INHERIT : 0, NULL, tsa ? tsa->lpSecurityDescriptor : NULL);

    st = NtCreateUserProcess(&hp, &ht, PROCESS_ALL_ACCESS, THREAD_ALL_ACCESS, &poa, &toa,
                             inherit ? PROCESS_CREATE_INHERIT_HANDLES : 0, (flags & CREATE_SUSPENDED) ? THREAD_CREATE_SUSPENDED : 0,
                             params, &info, &attrs);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); goto out; }
    pi->hProcess = hp; pi->hThread = ht;
    pi->dwProcessId = (DWORD)(uintptr_t)cid.UniqueProcess; pi->dwThreadId = (DWORD)(uintptr_t)cid.UniqueThread;
    ok = TRUE;
out:
    if (params) RtlDestroyProcessParameters(params);
    if (own_env) RtlFreeHeap(RtlGetProcessHeap(), 0, own_env);
    __atomic_store_n(&busy, 0, __ATOMIC_RELEASE);
    return ok;
}

BOOL WINAPI CreateProcessA(LPCSTR app, LPSTR cmdline, SECURITY_ATTRIBUTES *psa, SECURITY_ATTRIBUTES *tsa, BOOL inherit,
                           DWORD flags, LPVOID env, LPCSTR cwd, STARTUPINFOA *sia, PROCESS_INFORMATION *pi)
{
    WCHAR wapp[K32_LOCAL_MAX], wcmd[32768], wcwd[K32_LOCAL_MAX], wtitle[K32_LOCAL_MAX], wdesk[K32_LOCAL_MAX];
    STARTUPINFOW si;
    WCHAR *wenv = NULL;
    BOOL ok;

    if (!sia) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (app && MultiByteToWideChar(CP_ACP, 0, app, -1, wapp, K32_LOCAL_MAX) <= 0) return FALSE;
    if (cmdline && MultiByteToWideChar(CP_ACP, 0, cmdline, -1, wcmd, 32768) <= 0) return FALSE;
    if (cwd && MultiByteToWideChar(CP_ACP, 0, cwd, -1, wcwd, K32_LOCAL_MAX) <= 0) return FALSE;
    memset(&si, 0, sizeof si);
    si.cb = sizeof si;
    si.dwX = sia->dwX; si.dwY = sia->dwY; si.dwXSize = sia->dwXSize; si.dwYSize = sia->dwYSize;
    si.dwXCountChars = sia->dwXCountChars; si.dwYCountChars = sia->dwYCountChars; si.dwFillAttribute = sia->dwFillAttribute;
    si.dwFlags = sia->dwFlags; si.wShowWindow = sia->wShowWindow;
    si.hStdInput = sia->hStdInput; si.hStdOutput = sia->hStdOutput; si.hStdError = sia->hStdError;
    if (sia->lpTitle) { if (MultiByteToWideChar(CP_ACP, 0, sia->lpTitle, -1, wtitle, K32_LOCAL_MAX) <= 0) return FALSE; si.lpTitle = wtitle; }
    if (sia->lpDesktop) { if (MultiByteToWideChar(CP_ACP, 0, sia->lpDesktop, -1, wdesk, K32_LOCAL_MAX) <= 0) return FALSE; si.lpDesktop = wdesk; }
    if (env && !(flags & CREATE_UNICODE_ENVIRONMENT)) {
        wenv = widen_env(env);
        if (!wenv) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return FALSE; }
        env = wenv; flags |= CREATE_UNICODE_ENVIRONMENT;
    }
    ok = CreateProcessW(app ? wapp : NULL, cmdline ? wcmd : NULL, psa, tsa, inherit, flags, env, cwd ? wcwd : NULL, &si, pi);
    if (wenv) RtlFreeHeap(RtlGetProcessHeap(), 0, wenv);
    return ok;
}
