/* ntdll runtime-library helpers: counted strings, case mapping, bitmaps, image headers, time fields, memory. */
#include "../include/loos_nt.h"
#include <string.h>

/* ---------- strings ---------- */
void NTAPI RtlInitUnicodeString(UNICODE_STRING *d, PCWSTR s)
{
    size_t n = 0;
    if (!s) { d->Length = d->MaximumLength = 0; d->Buffer = NULL; return; }
    while (s[n]) n++;
    if (n > 0x7FFE) n = 0x7FFE;
    d->Buffer = (PWSTR)s; d->Length = (USHORT)(n * sizeof(WCHAR)); d->MaximumLength = (USHORT)(d->Length + sizeof(WCHAR));
}
void NTAPI RtlInitAnsiString(ANSI_STRING *d, const char *s)
{
    size_t n = s ? strlen(s) : 0;
    if (n > 0xFFFE) n = 0xFFFE;
    d->Buffer = (char *)s; d->Length = (USHORT)n; d->MaximumLength = s ? (USHORT)(n + 1) : 0;
}
void NTAPI RtlFreeUnicodeString(UNICODE_STRING *s)
{
    if (s->Buffer) RtlFreeHeap(RtlGetProcessHeap(), 0, s->Buffer);
    s->Buffer = NULL; s->Length = s->MaximumLength = 0;
}
void NTAPI RtlFreeAnsiString(ANSI_STRING *s)
{
    if (s->Buffer) RtlFreeHeap(RtlGetProcessHeap(), 0, s->Buffer);
    s->Buffer = NULL; s->Length = s->MaximumLength = 0;
}

LONG NTAPI RtlCompareUnicodeString(const UNICODE_STRING *a, const UNICODE_STRING *b, BOOLEAN ic)
{
    size_t an = a->Length / sizeof(WCHAR), bn = b->Length / sizeof(WCHAR), i, n = an < bn ? an : bn;
    for (i = 0; i < n; i++) {
        WCHAR x = a->Buffer[i], y = b->Buffer[i];
        if (ic) { x = RtlUpcaseUnicodeChar(x); y = RtlUpcaseUnicodeChar(y); }
        if (x != y) return (LONG)x - (LONG)y;
    }
    return (LONG)an - (LONG)bn;
}
BOOLEAN NTAPI RtlEqualUnicodeString(const UNICODE_STRING *a, const UNICODE_STRING *b, BOOLEAN ic)
{ return a->Length == b->Length && RtlCompareUnicodeString(a, b, ic) == 0; }

/* The ANSI page used by these two is Windows-1252, matching GetACP(). Characters outside it become '?'. */
static const unsigned short cp1252_hi[32] = {
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F,
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178 };

NTSTATUS NTAPI RtlAnsiStringToUnicodeString(UNICODE_STRING *dst, const ANSI_STRING *src, BOOLEAN allocate)
{
    size_t i, bytes = (size_t)src->Length * sizeof(WCHAR);
    if (bytes + sizeof(WCHAR) > 0xFFFF) return STATUS_INVALID_PARAMETER;
    if (allocate) {
        dst->Buffer = RtlAllocateHeap(RtlGetProcessHeap(), 0, bytes + sizeof(WCHAR));
        if (!dst->Buffer) return STATUS_NO_MEMORY;
        dst->MaximumLength = (USHORT)(bytes + sizeof(WCHAR));
    } else if (bytes > dst->MaximumLength) return STATUS_BUFFER_OVERFLOW;
    for (i = 0; i < src->Length; i++) {
        unsigned char b = (unsigned char)src->Buffer[i];
        dst->Buffer[i] = (b >= 0x80 && b < 0xA0) ? cp1252_hi[b - 0x80] : b;
    }
    dst->Length = (USHORT)bytes;
    if (bytes + sizeof(WCHAR) <= dst->MaximumLength) dst->Buffer[i] = 0;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI RtlUnicodeStringToAnsiString(ANSI_STRING *dst, const UNICODE_STRING *src, BOOLEAN allocate)
{
    size_t i, k, n = src->Length / sizeof(WCHAR);
    if (n + 1 > 0xFFFF) return STATUS_INVALID_PARAMETER;
    if (allocate) {
        dst->Buffer = RtlAllocateHeap(RtlGetProcessHeap(), 0, n + 1);
        if (!dst->Buffer) return STATUS_NO_MEMORY;
        dst->MaximumLength = (USHORT)(n + 1);
    } else if (n > dst->MaximumLength) return STATUS_BUFFER_OVERFLOW;
    for (i = 0; i < n; i++) {
        WCHAR c = src->Buffer[i];
        unsigned char out = '?';
        if (c < 0x80 || (c >= 0xA0 && c <= 0xFF)) out = (unsigned char)c;
        else for (k = 0; k < 32; k++) if (cp1252_hi[k] == c) { out = (unsigned char)(0x80 + k); break; }
        dst->Buffer[i] = (char)out;
    }
    dst->Length = (USHORT)n;
    if (n + 1 <= dst->MaximumLength) dst->Buffer[n] = 0;
    return STATUS_SUCCESS;
}

/* ---------- simple case mapping: Latin-1, Latin Extended-A, Greek, Cyrillic (not the full Unicode tables) ---------- */
WCHAR NTAPI RtlUpcaseUnicodeChar(WCHAR c)
{
    if (c < 0x80) return (c >= 'a' && c <= 'z') ? (WCHAR)(c - 32) : c;
    if (c == 0xB5) return 0x39C;
    if (c >= 0xE0 && c <= 0xFE && c != 0xF7) return (WCHAR)(c - 32);
    if (c == 0xFF) return 0x178;
    if (c >= 0x100 && c <= 0x17F) {
        if (c == 0x131) return 'I';
        if (c == 0x17F) return 'S';
        if ((c >= 0x101 && c <= 0x137 && (c & 1)) || (c >= 0x14B && c <= 0x177 && (c & 1))) return (WCHAR)(c - 1);
        if ((c >= 0x13A && c <= 0x148 && !(c & 1)) || (c >= 0x17A && c <= 0x17E && !(c & 1))) return (WCHAR)(c - 1);
        return c;
    }
    if (c >= 0x3B1 && c <= 0x3CB) return c == 0x3C2 ? (WCHAR)0x3A3 : (WCHAR)(c - 32);
    if (c == 0x3AC) return 0x386;
    if (c >= 0x3AD && c <= 0x3AF) return (WCHAR)(c - 37);
    if (c == 0x3CC) return 0x38C;
    if (c == 0x3CD || c == 0x3CE) return (WCHAR)(c - 63);
    if (c >= 0x430 && c <= 0x44F) return (WCHAR)(c - 32);
    if (c >= 0x450 && c <= 0x45F) return (WCHAR)(c - 80);
    return c;
}

WCHAR NTAPI RtlDowncaseUnicodeChar(WCHAR c)
{
    if (c < 0x80) return (c >= 'A' && c <= 'Z') ? (WCHAR)(c + 32) : c;
    if (c >= 0xC0 && c <= 0xDE && c != 0xD7) return (WCHAR)(c + 32);
    if (c == 0x178) return 0xFF;
    if (c >= 0x100 && c <= 0x17F) {
        if (c == 0x130) return 'i';
        if ((c >= 0x100 && c <= 0x136 && !(c & 1)) || (c >= 0x14A && c <= 0x176 && !(c & 1))) return (WCHAR)(c + 1);
        if ((c >= 0x139 && c <= 0x147 && (c & 1)) || (c >= 0x179 && c <= 0x17D && (c & 1))) return (WCHAR)(c + 1);
        return c;
    }
    if ((c >= 0x391 && c <= 0x3A1) || (c >= 0x3A3 && c <= 0x3AB)) return (WCHAR)(c + 32);
    if (c == 0x386) return 0x3AC;
    if (c >= 0x388 && c <= 0x38A) return (WCHAR)(c + 37);
    if (c == 0x38C) return 0x3CC;
    if (c == 0x38E || c == 0x38F) return (WCHAR)(c + 63);
    if (c >= 0x410 && c <= 0x42F) return (WCHAR)(c + 32);
    if (c >= 0x400 && c <= 0x40F) return (WCHAR)(c + 80);
    return c;
}

/* ---------- bitmaps ---------- */
static int bit(const RTL_BITMAP *b, ULONG i) { return (b->Buffer[i >> 5] >> (i & 31)) & 1; }
static void put_bit(RTL_BITMAP *b, ULONG i, int v) { if (v) b->Buffer[i >> 5] |= 1u << (i & 31); else b->Buffer[i >> 5] &= ~(1u << (i & 31)); }
static size_t words(const RTL_BITMAP *b) { return (b->SizeOfBitMap + 31) / 32; }

void NTAPI RtlInitializeBitMap(RTL_BITMAP *b, ULONG *buf, ULONG bits) { b->SizeOfBitMap = bits; b->Buffer = buf; }
void NTAPI RtlClearAllBits(RTL_BITMAP *b) { memset(b->Buffer, 0, words(b) * 4); }
void NTAPI RtlSetAllBits(RTL_BITMAP *b) { memset(b->Buffer, 0xFF, words(b) * 4); }
void NTAPI RtlSetBits(RTL_BITMAP *b, ULONG start, ULONG n) { ULONG i; if (start > b->SizeOfBitMap || n > b->SizeOfBitMap - start) return; for (i = 0; i < n; i++) put_bit(b, start + i, 1); }
void NTAPI RtlClearBits(RTL_BITMAP *b, ULONG start, ULONG n) { ULONG i; if (start > b->SizeOfBitMap || n > b->SizeOfBitMap - start) return; for (i = 0; i < n; i++) put_bit(b, start + i, 0); }
BOOLEAN NTAPI RtlTestBit(RTL_BITMAP *b, ULONG i) { return i < b->SizeOfBitMap && bit(b, i); }
static BOOLEAN all(RTL_BITMAP *b, ULONG start, ULONG n, int v)
{
    ULONG i;
    if (start > b->SizeOfBitMap || n > b->SizeOfBitMap - start) return FALSE;
    for (i = 0; i < n; i++) if (bit(b, start + i) != v) return FALSE;
    return TRUE;
}
BOOLEAN NTAPI RtlAreBitsClear(RTL_BITMAP *b, ULONG start, ULONG n) { return all(b, start, n, 0); }
BOOLEAN NTAPI RtlAreBitsSet(RTL_BITMAP *b, ULONG start, ULONG n) { return all(b, start, n, 1); }

/* first run of n bits equal to v at or after hint (wrapping once); 0xFFFFFFFF if none */
static ULONG find_run(RTL_BITMAP *b, ULONG n, ULONG hint, int v)
{
    ULONG pass, i, run, start = 0, total = b->SizeOfBitMap;
    if (n == 0 || n > total) return 0xFFFFFFFFu;
    if (hint >= total) hint = 0;
    for (pass = 0; pass < 2; pass++) {
        ULONG from = pass ? 0 : hint, to = pass ? hint + n - 1 : total;
        if (to > total) to = total;
        for (run = 0, i = from; i < to; i++) {
            if (bit(b, i) == v) { if (run == 0) start = i; if (++run == n) return start; }
            else run = 0;
        }
    }
    return 0xFFFFFFFFu;
}
ULONG NTAPI RtlFindClearBits(RTL_BITMAP *b, ULONG n, ULONG hint) { return find_run(b, n, hint, 0); }
ULONG NTAPI RtlFindSetBits(RTL_BITMAP *b, ULONG n, ULONG hint) { return find_run(b, n, hint, 1); }
ULONG NTAPI RtlFindClearBitsAndSet(RTL_BITMAP *b, ULONG n, ULONG hint) { ULONG r = find_run(b, n, hint, 0); if (r != 0xFFFFFFFFu) RtlSetBits(b, r, n); return r; }
ULONG NTAPI RtlFindSetBitsAndClear(RTL_BITMAP *b, ULONG n, ULONG hint) { ULONG r = find_run(b, n, hint, 1); if (r != 0xFFFFFFFFu) RtlClearBits(b, r, n); return r; }
ULONG NTAPI RtlNumberOfSetBits(RTL_BITMAP *b) { ULONG i, c = 0; for (i = 0; i < b->SizeOfBitMap; i++) c += (ULONG)bit(b, i); return c; }
ULONG NTAPI RtlNumberOfClearBits(RTL_BITMAP *b) { return b->SizeOfBitMap - RtlNumberOfSetBits(b); }

/* ---------- image headers (for an image already mapped in memory) ---------- */
static uint32_t rd32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }

PVOID NTAPI RtlImageNtHeader(PVOID base)
{
    const uint8_t *b = base;
    uint32_t lf;
    if (!b || b[0] != 'M' || b[1] != 'Z') return NULL;
    lf = rd32(b + 0x3c);
    if (lf < 0x40 || lf > 0x10000000u || rd32(b + lf) != 0x00004550) return NULL;
    return (PVOID)(b + lf);
}

PVOID NTAPI RtlImageDirectoryEntryToData(PVOID base, BOOLEAN mapped_as_image, USHORT dir, ULONG *size)
{
    const uint8_t *nt = RtlImageNtHeader(base), *opt;
    uint16_t magic;
    uint32_t nrva, rva;
    (void)mapped_as_image;                                         /* only mapped layouts are supported */
    if (!nt) return NULL;
    opt = nt + 24;
    magic = (uint16_t)(opt[0] | opt[1] << 8);
    nrva = rd32(opt + (magic == 0x20b ? 108 : 92));
    if (dir >= nrva || dir >= 16) return NULL;
    { const uint8_t *e = opt + (magic == 0x20b ? 112 : 96) + (size_t)dir * 8; rva = rd32(e); if (size) *size = rd32(e + 4); }
    return rva ? (PVOID)((const uint8_t *)base + rva) : NULL;
}

/* ---------- time ---------- */
#define TICKS_PER_SEC 10000000ll
#define EPOCH_1601_TO_1970_SECS 11644473600ll
BOOLEAN NTAPI RtlTimeToSecondsSince1970(LARGE_INTEGER *t, ULONG *secs)
{
    LONGLONG s = t->QuadPart / TICKS_PER_SEC - EPOCH_1601_TO_1970_SECS;
    if (s < 0 || s > 0xFFFFFFFFll) return FALSE;
    *secs = (ULONG)s;
    return TRUE;
}
void NTAPI RtlSecondsSince1970ToTime(ULONG secs, LARGE_INTEGER *t) { t->QuadPart = ((LONGLONG)secs + EPOCH_1601_TO_1970_SECS) * TICKS_PER_SEC; }

static int64_t days_from_civil(int y, unsigned m, unsigned d)
{
    int64_t era, yoe, doy, doe;
    y -= m <= 2;
    era = (y >= 0 ? y : y - 399) / 400;
    yoe = y - era * 400;
    doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}
void NTAPI RtlTimeToTimeFields(LARGE_INTEGER *t, TIME_FIELDS *f)
{
    LONGLONG total = t->QuadPart, ms_total = total / 10000, secs = ms_total / 1000, days = secs / 86400;
    int64_t z = days - 134774 + 719468, era, doe, yoe, doy, mp;
    era = (z >= 0 ? z : z - 146096) / 146097; doe = z - era * 146097;
    yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; doy = doe - (365 * yoe + yoe / 4 - yoe / 100); mp = (5 * doy + 2) / 153;
    f->Day = (SHORT)(doy - (153 * mp + 2) / 5 + 1);
    f->Month = (SHORT)(mp < 10 ? mp + 3 : mp - 9);
    f->Year = (SHORT)(yoe + era * 400 + (f->Month <= 2));
    f->Hour = (SHORT)((secs / 3600) % 24); f->Minute = (SHORT)((secs / 60) % 60); f->Second = (SHORT)(secs % 60);
    f->Milliseconds = (SHORT)(ms_total % 1000); f->Weekday = (SHORT)((days + 1) % 7);
}
BOOLEAN NTAPI RtlTimeFieldsToTime(TIME_FIELDS *f, LARGE_INTEGER *t)
{
    static const unsigned md[12] = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 };
    int leap;
    unsigned dim;
    if (f->Year < 1601 || f->Month < 1 || f->Month > 12 || f->Day < 1 || f->Hour < 0 || f->Hour > 23 || f->Minute < 0 || f->Minute > 59 ||
        f->Second < 0 || f->Second > 59 || f->Milliseconds < 0 || f->Milliseconds > 999) return FALSE;
    leap = (f->Year % 4 == 0 && f->Year % 100 != 0) || f->Year % 400 == 0;
    dim = md[f->Month - 1] + (f->Month == 2 && leap);
    if ((unsigned)f->Day > dim) return FALSE;
    t->QuadPart = (((days_from_civil(f->Year, (unsigned)f->Month, (unsigned)f->Day) + 134774) * 86400ll + f->Hour * 3600ll + f->Minute * 60ll + f->Second) * 1000ll + f->Milliseconds) * 10000ll;
    return TRUE;
}

/* ---------- memory ---------- */
SIZE_T NTAPI RtlCompareMemory(const void *a, const void *b, SIZE_T n)
{
    const unsigned char *x = a, *y = b;
    SIZE_T i = 0;
    while (i < n && x[i] == y[i]) i++;
    return i;
}
void NTAPI RtlFillMemory(PVOID d, SIZE_T n, UCHAR fill) { memset(d, fill, n); }
void NTAPI RtlZeroMemory(PVOID d, SIZE_T n) { memset(d, 0, n); }
void NTAPI RtlMoveMemory(PVOID d, const void *s, SIZE_T n) { memmove(d, s, n); }
void NTAPI RtlCopyMemory(PVOID d, const void *s, SIZE_T n) { memcpy(d, s, n); }
