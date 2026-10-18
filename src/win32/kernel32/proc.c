/* Process/thread identity, last-error, and module (loader) entry points. */
#include "k32_internal.h"
#include <string.h>

DWORD WINAPI GetLastError(void)        { return *k32_last_error_ptr(); }
void  WINAPI SetLastError(DWORD e)     { *k32_last_error_ptr() = e; }

HANDLE WINAPI GetCurrentProcess(void)  { return NtCurrentProcess(); }
HANDLE WINAPI GetCurrentThread(void)   { return NtCurrentThread(); }
DWORD  WINAPI GetCurrentProcessId(void){ return k32_pid(); }
DWORD  WINAPI GetCurrentThreadId(void) { return k32_tid(); }
void   WINAPI ExitProcess(DWORD code)  { RtlExitUserProcess((NTSTATUS)code); for (;;) { } }

static void init_us(UNICODE_STRING *us, LPCWSTR s)
{
    size_t n = 0;
    while (s[n]) n++;
    us->Buffer = (PWSTR)s;
    us->Length = (USHORT)(n * sizeof(WCHAR));
    us->MaximumLength = (USHORT)((n + 1) * sizeof(WCHAR));
}

HMODULE WINAPI GetModuleHandleW(LPCWSTR name)
{
    UNICODE_STRING us;
    PVOID h = NULL;
    NTSTATUS st;
    if (!name) return (HMODULE)k32_image_base();               /* the process image */
    if (!*name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    init_us(&us, name);
    st = LdrGetDllHandle(NULL, NULL, &us, &h);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return (HMODULE)h;
}

HMODULE WINAPI LoadLibraryW(LPCWSTR name)
{
    UNICODE_STRING us;
    PVOID h = NULL;
    NTSTATUS st;
    if (!name || !*name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    init_us(&us, name);
    st = LdrLoadDll(NULL, NULL, &us, &h);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return (HMODULE)h;
}

BOOL WINAPI FreeLibrary(HMODULE m)
{
    NTSTATUS st = LdrUnloadDll(m);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

FARPROC WINAPI GetProcAddress(HMODULE m, const char *name)
{
    PVOID addr = NULL;
    NTSTATUS st;
    if (!m || !name) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    if ((uintptr_t)name < 0x10000) {                           /* MAKEINTRESOURCE-style ordinal */
        st = LdrGetProcedureAddress(m, NULL, (ULONG)(uintptr_t)name, &addr);
    } else {
        ANSI_STRING as;
        size_t n = strlen(name);
        if (n > 0xFFFE) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
        as.Buffer = (char *)name; as.Length = (USHORT)n; as.MaximumLength = (USHORT)(n + 1);
        st = LdrGetProcedureAddress(m, &as, 0, &addr);
    }
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return (FARPROC)addr;
}
