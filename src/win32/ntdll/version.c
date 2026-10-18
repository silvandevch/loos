/* RtlGetVersion / RtlVerifyVersionInfo / VerSetConditionMask.
 *
 * The reported version is whatever the compat layer resolved for this process
 * (rtl_set_version, called once at process start with compat_version_of(claim.era);
 * real ntdll mirrors it into PEB OSMajorVersion & co). Nothing here consults the
 * real kernel version, by design: see src/win32/compat/compat.h. */
#include "../include/loos_win.h"
#include "../compat/compat.h"
#include <string.h>

static compat_version cur;               /* set at process start, read-only afterwards */
static int cur_set;

void rtl_set_version(const compat_version *v) { cur = *v; cur_set = 1; }

static const compat_version *current(void)
{
    if (!cur_set) rtl_set_version(compat_version_of(COMPAT_NATIVE_ERA));
    return &cur;
}

NTSTATUS NTAPI RtlGetVersion(RTL_OSVERSIONINFOW *p)
{
    const compat_version *v = current();
    size_t i;
    int ex;
    if (!p) return STATUS_INVALID_PARAMETER;
    if (p->dwOSVersionInfoSize != sizeof(RTL_OSVERSIONINFOW) && p->dwOSVersionInfoSize != sizeof(RTL_OSVERSIONINFOEXW))
        return STATUS_INVALID_PARAMETER;
    ex = p->dwOSVersionInfoSize == sizeof(RTL_OSVERSIONINFOEXW);
    p->dwMajorVersion = v->major; p->dwMinorVersion = v->minor;
    p->dwBuildNumber = v->build; p->dwPlatformId = v->platform_id;
    memset(p->szCSDVersion, 0, sizeof p->szCSDVersion);
    for (i = 0; v->csd[i] && i < 127; i++) p->szCSDVersion[i] = (WCHAR)(unsigned char)v->csd[i];
    if (ex) {
        RTL_OSVERSIONINFOEXW *x = (RTL_OSVERSIONINFOEXW *)p;
        x->wServicePackMajor = v->sp_major; x->wServicePackMinor = v->sp_minor;
        x->wSuiteMask = v->suite_mask; x->wProductType = v->product_type; x->wReserved = 0;
    }
    return STATUS_SUCCESS;
}

/* ---- condition masks: 3 bits per type, indexed by log2(type bit); bit 63 marks the new style ---- */
#define NEW_STYLE_BIT 0x8000000000000000ull

static unsigned type_index(ULONG type_mask)       /* index of the highest set bit */
{
    unsigned i = 0;
    while (type_mask >>= 1) i++;
    return i;
}

ULONGLONG NTAPI VerSetConditionMask(ULONGLONG mask, ULONG type_mask, UCHAR condition)
{
    if (!type_mask) return 0;
    return mask | NEW_STYLE_BIT | ((ULONGLONG)(condition & 7) << (type_index(type_mask) * 3));
}

static unsigned condition_of(ULONGLONG mask, ULONG type)
{
    return (unsigned)((mask >> (type_index(type) * 3)) & 7);
}

/* does `cur <cond> req` hold? */
static int cmp(unsigned cond, LONGLONG req, LONGLONG have)
{
    switch (cond) {
    case VER_EQUAL:         return have == req;
    case VER_GREATER:       return have > req;
    case VER_GREATER_EQUAL: return have >= req;
    case VER_LESS:          return have < req;
    case VER_LESS_EQUAL:    return have <= req;
    default:                return 0;
    }
}

NTSTATUS NTAPI RtlVerifyVersionInfo(RTL_OSVERSIONINFOEXW *want, ULONG types, ULONGLONG cmask)
{
    RTL_OSVERSIONINFOEXW have;
    NTSTATUS st;
    unsigned cond = VER_EQUAL;
    int result = 1, decided = 0, i;
    struct { ULONG type; LONGLONG req, cur; } chain[4];
    int nchain = 0;

    if (!want || !types) return STATUS_INVALID_PARAMETER;
    memset(&have, 0, sizeof have);
    have.dwOSVersionInfoSize = sizeof have;
    if ((st = RtlGetVersion((RTL_OSVERSIONINFOW *)&have)) != STATUS_SUCCESS) return st;

    if ((types & VER_SUITENAME) && want->wSuiteMask) {
        unsigned sc = condition_of(cmask, VER_SUITENAME);
        int found = 0;
        for (i = 0; i < 16; i++) {
            if (!(want->wSuiteMask & (1u << i))) continue;
            if (sc == VER_AND) { if (!(have.wSuiteMask & (1u << i))) return STATUS_REVISION_MISMATCH; }
            else if (sc == VER_OR) { if (have.wSuiteMask & (1u << i)) found = 1; }
            else return STATUS_INVALID_PARAMETER;
        }
        if (sc == VER_OR && !found) return STATUS_REVISION_MISMATCH;
    }

    /* major -> minor -> service pack: a lexicographic walk. The first non-EQUAL condition
     * governs the remaining fields; a field that differs decides the result on its own. */
    if (types & VER_MAJORVERSION)     { chain[nchain].type = VER_MAJORVERSION;     chain[nchain].req = want->dwMajorVersion;    chain[nchain++].cur = have.dwMajorVersion; }
    if (types & VER_MINORVERSION)     { chain[nchain].type = VER_MINORVERSION;     chain[nchain].req = want->dwMinorVersion;    chain[nchain++].cur = have.dwMinorVersion; }
    if (types & VER_SERVICEPACKMAJOR) { chain[nchain].type = VER_SERVICEPACKMAJOR; chain[nchain].req = want->wServicePackMajor; chain[nchain++].cur = have.wServicePackMajor; }
    if (types & VER_SERVICEPACKMINOR) { chain[nchain].type = VER_SERVICEPACKMINOR; chain[nchain].req = want->wServicePackMinor; chain[nchain++].cur = have.wServicePackMinor; }
    for (i = 0; i < nchain && !decided; i++) {
        if (cond == VER_EQUAL) cond = condition_of(cmask, chain[i].type);
        result = cmp(cond, chain[i].req, chain[i].cur);
        if (chain[i].req != chain[i].cur) decided = 1;
    }
    if (!result) return STATUS_REVISION_MISMATCH;

    if ((types & VER_BUILDNUMBER) && !cmp(condition_of(cmask, VER_BUILDNUMBER), want->dwBuildNumber, have.dwBuildNumber)) return STATUS_REVISION_MISMATCH;
    if ((types & VER_PLATFORMID) && !cmp(condition_of(cmask, VER_PLATFORMID), want->dwPlatformId, have.dwPlatformId)) return STATUS_REVISION_MISMATCH;
    if ((types & VER_PRODUCT_TYPE) && !cmp(condition_of(cmask, VER_PRODUCT_TYPE), want->wProductType, have.wProductType)) return STATUS_REVISION_MISMATCH;
    return STATUS_SUCCESS;
}
