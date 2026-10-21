/* Small utilities: tick count, MulDiv, pointer probing, system information, wall-clock time. */
#include "k32_internal.h"
#include <string.h>

DWORD WINAPI GetTickCount(void) { return (DWORD)GetTickCount64(); }

int WINAPI MulDiv(int number, int numerator, int denominator)
{
    uint64_t p, d, r;
    int neg;
    if (denominator == 0) return -1;
    neg = ((number < 0) != (numerator < 0)) != (denominator < 0);
    p = (uint64_t)(number < 0 ? -(int64_t)number : (int64_t)number) * (uint64_t)(numerator < 0 ? -(int64_t)numerator : (int64_t)numerator);
    d = (uint64_t)(denominator < 0 ? -(int64_t)denominator : (int64_t)denominator);
    r = (p + d / 2) / d;                                     /* round half away from zero */
    if (r > (neg ? 0x80000000ull : 0x7FFFFFFFull)) return -1;
    return neg ? (int)(-(int64_t)r) : (int)r;
}

/* ---------- pointer probing ---------- */
static int prot_ok(DWORD prot, int write)
{
    if (prot & (PAGE_NOACCESS | PAGE_GUARD)) return 0;
    if (write) return (prot & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | 0x80)) != 0;
    return (prot & (PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | 0x80)) != 0;
}

/* end (exclusive) of the accessible region containing `a`, or 0 */
static uintptr_t access_end(uintptr_t a, int write)
{
    MEMORY_BASIC_INFORMATION mbi;
    uintptr_t end;
    if (VirtualQuery((LPCVOID)a, &mbi, sizeof mbi) != sizeof mbi) return 0;
    if (mbi.State != MEM_COMMIT || !prot_ok(mbi.Protect, write)) return 0;
    end = (uintptr_t)mbi.BaseAddress + mbi.RegionSize;
    return end > a ? end : 0;
}

static BOOL bad_range(const void *p, ULONG_PTR n, int write)
{
    uintptr_t a = (uintptr_t)p, last;
    if (n == 0) return FALSE;
    if (!p) return TRUE;
    last = a + n - 1;
    if (last < a) return TRUE;                               /* wraps */
    while (a <= last) {
        uintptr_t e = access_end(a, write);
        if (!e) return TRUE;
        if (e == 0 || e - 1 >= last) return FALSE;
        a = e;
    }
    return FALSE;
}
BOOL WINAPI IsBadReadPtr(LPCVOID p, ULONG_PTR n) { return bad_range(p, n, 0); }
BOOL WINAPI IsBadWritePtr(LPVOID p, ULONG_PTR n) { return bad_range(p, n, 1); }

static BOOL bad_string(const void *p, ULONG_PTR max, size_t unit)
{
    uintptr_t a = (uintptr_t)p, checked_to = 0;
    ULONG_PTR i;
    if (max == 0) return FALSE;
    if (!p) return TRUE;
    for (i = 0; i < max; i++, a += unit) {
        if (a + unit > checked_to) {
            uintptr_t e = access_end(a, 0);
            if (!e || e < a + unit) return TRUE;
            checked_to = e;
        }
        if (unit == 1 ? *(const char *)a == 0 : *(const WCHAR *)a == 0) return FALSE;
    }
    return FALSE;
}
BOOL WINAPI IsBadStringPtrA(LPCSTR p, ULONG_PTR max) { return bad_string(p, max, 1); }
BOOL WINAPI IsBadStringPtrW(LPCWSTR p, ULONG_PTR max) { return bad_string(p, max, 2); }

/* ---------- system information ---------- */
void WINAPI GetNativeSystemInfo(SYSTEM_INFO *si)
{
    SYSTEM_BASIC_INFORMATION b;
    SYSTEM_PROCESSOR_INFORMATION pi;
    memset(si, 0, sizeof *si);
    memset(&b, 0, sizeof b); memset(&pi, 0, sizeof pi);
    NtQuerySystemInformation(SystemBasicInformation, &b, sizeof b, NULL);
    NtQuerySystemInformation(SystemProcessorInformation, &pi, sizeof pi, NULL);
    si->wProcessorArchitecture = PROCESSOR_ARCHITECTURE_AMD64;
    si->dwPageSize = b.PageSize ? b.PageSize : 4096;
    si->lpMinimumApplicationAddress = (LPVOID)b.MinimumUserModeAddress;
    si->lpMaximumApplicationAddress = (LPVOID)b.MaximumUserModeAddress;
    si->dwActiveProcessorMask = b.ActiveProcessorsAffinityMask;
    si->dwNumberOfProcessors = b.NumberOfProcessors ? b.NumberOfProcessors : 1;
    si->dwProcessorType = 8664;
    si->dwAllocationGranularity = b.AllocationGranularity ? b.AllocationGranularity : 0x10000;
    si->wProcessorLevel = pi.ProcessorLevel; si->wProcessorRevision = pi.ProcessorRevision;
}
void WINAPI GetSystemInfo(SYSTEM_INFO *si) { GetNativeSystemInfo(si); }      /* no WOW64: native and process view agree */

/* ---------- time (UTC until time zones exist) ---------- */
#define EPOCH_1601_TO_1970_DAYS 134774ll

static int64_t days_from_civil(int y, unsigned m, unsigned d)       /* days since 1970-01-01 */
{
    int64_t era, yoe, doy, doe;
    y -= m <= 2;
    era = (y >= 0 ? y : y - 399) / 400;
    yoe = y - era * 400;
    doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}
static void civil_from_days(int64_t z, int *y, unsigned *m, unsigned *d)
{
    int64_t era, doe, yoe, doy, mp;
    z += 719468;
    era = (z >= 0 ? z : z - 146096) / 146097;
    doe = z - era * 146097;
    yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    mp = (5 * doy + 2) / 153;
    *d = (unsigned)(doy - (153 * mp + 2) / 5 + 1);
    *m = (unsigned)(mp < 10 ? mp + 3 : mp - 9);
    *y = (int)(yoe + era * 400 + (*m <= 2));
}

void WINAPI GetSystemTimeAsFileTime(FILETIME *ft)
{
    LARGE_INTEGER t;
    t.QuadPart = 0;
    NtQuerySystemTime(&t);
    ft->dwLowDateTime = (DWORD)t.QuadPart; ft->dwHighDateTime = (DWORD)((uint64_t)t.QuadPart >> 32);
}

BOOL WINAPI FileTimeToSystemTime(const FILETIME *ft, SYSTEMTIME *st)
{
    uint64_t t = ((uint64_t)ft->dwHighDateTime << 32) | ft->dwLowDateTime, ms, secs;
    int64_t days;
    int y; unsigned m, d;
    if (t >> 63) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    ms = t / 10000; secs = ms / 1000; days = (int64_t)(secs / 86400);
    civil_from_days(days - EPOCH_1601_TO_1970_DAYS, &y, &m, &d);
    st->wYear = (WORD)y; st->wMonth = (WORD)m; st->wDay = (WORD)d;
    st->wDayOfWeek = (WORD)((days + 1) % 7);                      /* 1601-01-01 was a Monday */
    st->wHour = (WORD)((secs / 3600) % 24); st->wMinute = (WORD)((secs / 60) % 60); st->wSecond = (WORD)(secs % 60);
    st->wMilliseconds = (WORD)(ms % 1000);
    return TRUE;
}

BOOL WINAPI SystemTimeToFileTime(const SYSTEMTIME *st, FILETIME *ft)
{
    static const unsigned mdays[12] = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 };
    unsigned dim;
    int64_t days;
    uint64_t t;
    int leap;
    if (st->wYear < 1601 || st->wYear > 30827 || st->wMonth < 1 || st->wMonth > 12 || st->wDay < 1 ||
        st->wHour > 23 || st->wMinute > 59 || st->wSecond > 59 || st->wMilliseconds > 999) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE;
    }
    leap = (st->wYear % 4 == 0 && st->wYear % 100 != 0) || st->wYear % 400 == 0;
    dim = mdays[st->wMonth - 1] + (st->wMonth == 2 && leap);
    if (st->wDay > dim) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    days = days_from_civil(st->wYear, st->wMonth, st->wDay) + EPOCH_1601_TO_1970_DAYS;
    t = ((uint64_t)days * 86400 + st->wHour * 3600u + st->wMinute * 60u + st->wSecond) * 1000u + st->wMilliseconds;
    t *= 10000;
    ft->dwLowDateTime = (DWORD)t; ft->dwHighDateTime = (DWORD)(t >> 32);
    return TRUE;
}

void WINAPI GetSystemTime(SYSTEMTIME *st) { FILETIME ft; GetSystemTimeAsFileTime(&ft); FileTimeToSystemTime(&ft, st); }
void WINAPI GetLocalTime(SYSTEMTIME *st) { GetSystemTime(st); }

/* ---------- delay-load entry points ---------- */
PVOID WINAPI ResolveDelayLoadedAPI(PVOID base, const void *descriptor, PVOID dll_hook, PVOID sys_hook, PVOID *thunk, ULONG flags)
{ return LdrResolveDelayLoadedAPI(base, descriptor, dll_hook, sys_hook, thunk, flags); }

/* called by delay-load helpers when a DLL or function cannot be resolved; the default hook declines */
FARPROC WINAPI DelayLoadFailureHook(LPCSTR dll, LPCSTR proc) { (void)dll; (void)proc; return NULL; }
