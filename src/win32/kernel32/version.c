#include "k32_internal.h"
#include <string.h>

DWORD WINAPI GetVersion(void)
{
    RTL_OSVERSIONINFOW v;
    memset(&v, 0, sizeof v);
    v.dwOSVersionInfoSize = sizeof v;
    RtlGetVersion(&v);
    /* low word: major | minor<<8; high word: build (NT platform) */
    return (DWORD)(v.dwMajorVersion & 0xFF) | ((v.dwMinorVersion & 0xFF) << 8) | ((v.dwBuildNumber & 0x7FFF) << 16);
}

BOOL WINAPI GetVersionExW(OSVERSIONINFOW *info)
{
    NTSTATUS st;
    if (!info || (info->dwOSVersionInfoSize != sizeof(OSVERSIONINFOW) && info->dwOSVersionInfoSize != sizeof(OSVERSIONINFOEXW))) {
        *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER;
        return FALSE;
    }
    st = RtlGetVersion(info);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI GetVersionExA(OSVERSIONINFOA *info)
{
    RTL_OSVERSIONINFOEXW w;
    size_t i;
    int ex;
    if (!info || (info->dwOSVersionInfoSize != sizeof(OSVERSIONINFOA) && info->dwOSVersionInfoSize != sizeof(OSVERSIONINFOEXA))) {
        *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER;
        return FALSE;
    }
    ex = info->dwOSVersionInfoSize == sizeof(OSVERSIONINFOEXA);
    memset(&w, 0, sizeof w);
    w.dwOSVersionInfoSize = ex ? sizeof(RTL_OSVERSIONINFOEXW) : sizeof(RTL_OSVERSIONINFOW);
    RtlGetVersion((RTL_OSVERSIONINFOW *)&w);
    info->dwMajorVersion = w.dwMajorVersion; info->dwMinorVersion = w.dwMinorVersion;
    info->dwBuildNumber = w.dwBuildNumber; info->dwPlatformId = w.dwPlatformId;
    for (i = 0; i < 127 && w.szCSDVersion[i]; i++) info->szCSDVersion[i] = (char)(w.szCSDVersion[i] < 0x80 ? w.szCSDVersion[i] : '?');
    info->szCSDVersion[i] = 0;
    if (ex) {
        OSVERSIONINFOEXA *x = (OSVERSIONINFOEXA *)info;
        x->wServicePackMajor = w.wServicePackMajor; x->wServicePackMinor = w.wServicePackMinor;
        x->wSuiteMask = w.wSuiteMask; x->wProductType = w.wProductType; x->wReserved = 0;
    }
    return TRUE;
}

BOOL WINAPI VerifyVersionInfoW(OSVERSIONINFOEXW *info, DWORD type_mask, ULONGLONG condition_mask)
{
    NTSTATUS st = RtlVerifyVersionInfo(info, type_mask, condition_mask);
    if (st == STATUS_SUCCESS) return TRUE;
    k32_set_nt_error(st);
    return FALSE;
}
