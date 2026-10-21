/* Code page conversion, string compare/copy helpers, simple Unicode case mapping.
 *
 * Code pages: UTF-8 (65001), Windows-1252 (the default ANSI page), IBM 437 (default OEM page),
 * US-ASCII (20127) and ISO 8859-1 (28591). Other pages report ERROR_INVALID_PARAMETER.
 * Locale-sensitive collation is not implemented: CompareStringW / lstrcmp* compare UTF-16 code
 * units, optionally after simple case folding. The folding covers Latin-1, Latin Extended-A,
 * Greek and Cyrillic only. */
#include "k32_internal.h"

static const uint16_t cp1252_hi[32] = {
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F,
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178 };

static const uint16_t cp437_hi[128] = {
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, 0x00EA, 0x00EB, 0x00E8, 0x00EF, 0x00EE, 0x00EC, 0x00C4, 0x00C5,
    0x00C9, 0x00E6, 0x00C6, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9, 0x00FF, 0x00D6, 0x00DC, 0x00A2, 0x00A3, 0x00A5, 0x20A7, 0x0192,
    0x00E1, 0x00ED, 0x00F3, 0x00FA, 0x00F1, 0x00D1, 0x00AA, 0x00BA, 0x00BF, 0x2310, 0x00AC, 0x00BD, 0x00BC, 0x00A1, 0x00AB, 0x00BB,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557, 0x255D, 0x255C, 0x255B, 0x2510,
    0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x255E, 0x255F, 0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567,
    0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256B, 0x256A, 0x2518, 0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580,
    0x03B1, 0x00DF, 0x0393, 0x03C0, 0x03A3, 0x03C3, 0x00B5, 0x03C4, 0x03A6, 0x0398, 0x03A9, 0x03B4, 0x221E, 0x03C6, 0x03B5, 0x2229,
    0x2261, 0x00B1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00F7, 0x2248, 0x00B0, 0x2219, 0x00B7, 0x221A, 0x207F, 0x00B2, 0x25A0, 0x00A0 };

UINT WINAPI GetACP(void) { return 1252; }
UINT WINAPI GetOEMCP(void) { return 437; }

static UINT resolve_cp(UINT cp)
{
    if (cp == CP_ACP || cp == CP_THREAD_ACP) return GetACP();
    if (cp == CP_OEMCP) return GetOEMCP();
    return cp;
}
static int cp_supported(UINT cp) { return cp == 437 || cp == 1252 || cp == 20127 || cp == 28591 || cp == CP_UTF8; }
BOOL WINAPI IsValidCodePage(UINT cp) { return cp_supported(cp); }

/* single-byte page: byte -> UTF-16 (0 = no mapping, 0xFFFF reserved never used) */
static int sb_to_wide(UINT cp, unsigned char b, WCHAR *out)
{
    if (b < 0x80) { *out = b; return 1; }
    switch (cp) {
    case 1252: *out = (b < 0xA0) ? cp1252_hi[b - 0x80] : b; return 1;
    case 437:  *out = cp437_hi[b - 0x80]; return 1;
    case 28591: *out = b; return 1;
    default: return 0;                                  /* 20127: no high half */
    }
}
static int wide_to_sb(UINT cp, WCHAR c, unsigned char *out)
{
    unsigned i;
    if (c < 0x80) { *out = (unsigned char)c; return 1; }
    switch (cp) {
    case 1252:
        for (i = 0; i < 32; i++) if (cp1252_hi[i] == c) { *out = (unsigned char)(0x80 + i); return 1; }
        if (c >= 0xA0 && c <= 0xFF) { *out = (unsigned char)c; return 1; }
        return 0;
    case 437:
        for (i = 0; i < 128; i++) if (cp437_hi[i] == c) { *out = (unsigned char)(0x80 + i); return 1; }
        return 0;
    case 28591: if (c <= 0xFF) { *out = (unsigned char)c; return 1; } return 0;
    default: return 0;
    }
}

/* UTF-8 decoder. Returns bytes consumed (>0); *cp = code point, or 0xFFFFFFFF for an invalid unit
 * (which still consumes >0 bytes: the maximal ill-formed subsequence). */
static size_t utf8_next(const unsigned char *s, size_t n, uint32_t *cp)
{
    unsigned char b = s[0];
    size_t need, i;
    uint32_t v;
    unsigned char lo = 0x80, hi = 0xBF;
    if (b < 0x80) { *cp = b; return 1; }
    if (b >= 0xC2 && b <= 0xDF) { need = 1; v = b & 0x1F; }
    else if (b >= 0xE0 && b <= 0xEF) { need = 2; v = b & 0x0F; if (b == 0xE0) lo = 0xA0; if (b == 0xED) hi = 0x9F; }
    else if (b >= 0xF0 && b <= 0xF4) { need = 3; v = b & 0x07; if (b == 0xF0) lo = 0x90; if (b == 0xF4) hi = 0x8F; }
    else { *cp = 0xFFFFFFFFu; return 1; }
    for (i = 1; i <= need; i++) {
        if (i >= n || s[i] < (i == 1 ? lo : 0x80) || s[i] > (i == 1 ? hi : 0xBF)) { *cp = 0xFFFFFFFFu; return i; }
        v = (v << 6) | (s[i] & 0x3F);
    }
    *cp = v;
    return need + 1;
}

int WINAPI MultiByteToWideChar(UINT cp, DWORD flags, LPCSTR src, int cb, LPWSTR dst, int cch)
{
    const unsigned char *s = (const unsigned char *)src;
    size_t n, i = 0, out = 0;
    int utf8;

    cp = resolve_cp(cp);
    if (!src || cb == 0 || cb < -1 || cch < 0 || (!dst && cch != 0)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (!cp_supported(cp)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    utf8 = cp == CP_UTF8;
    if (flags & ~(utf8 ? (DWORD)MB_ERR_INVALID_CHARS : 0xFu)) { *k32_last_error_ptr() = ERROR_INVALID_FLAGS; return 0; }
    if (cb == -1) { n = 0; while (s[n]) n++; n++; } else n = (size_t)cb;       /* -1 includes the terminator */

    while (i < n) {
        WCHAR w[2]; int nw;
        if (utf8) {
            uint32_t c; size_t used = utf8_next(s + i, n - i, &c);
            i += used;
            if (c == 0xFFFFFFFFu) {
                if (flags & MB_ERR_INVALID_CHARS) { *k32_last_error_ptr() = ERROR_NO_UNICODE_TRANSLATION; return 0; }
                w[0] = 0xFFFD; nw = 1;
            } else if (c >= 0x10000) { c -= 0x10000; w[0] = (WCHAR)(0xD800 + (c >> 10)); w[1] = (WCHAR)(0xDC00 + (c & 0x3FF)); nw = 2; }
            else { w[0] = (WCHAR)c; nw = 1; }
        } else {
            if (!sb_to_wide(cp, s[i], &w[0])) {
                if (flags & MB_ERR_INVALID_CHARS) { *k32_last_error_ptr() = ERROR_NO_UNICODE_TRANSLATION; return 0; }
                w[0] = 0xFFFD;
            }
            i++; nw = 1;
        }
        if (dst) {
            if (out + (size_t)nw > (size_t)cch) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
            dst[out] = w[0]; if (nw == 2) dst[out + 1] = w[1];
        }
        out += (size_t)nw;
    }
    if (out > 0x7FFFFFFF) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    return (int)out;
}

int WINAPI WideCharToMultiByte(UINT cp, DWORD flags, LPCWSTR src, int cch, LPSTR dst, int cb, LPCSTR defchar, LPBOOL used_default)
{
    size_t n, i = 0, out = 0;
    int utf8;
    unsigned char defb = '?';

    cp = resolve_cp(cp);
    if (used_default) *used_default = FALSE;
    if (!src || cch == 0 || cch < -1 || cb < 0 || (!dst && cb != 0)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (!cp_supported(cp)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    utf8 = cp == CP_UTF8;
    if (utf8 && (defchar || used_default)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (flags & ~(utf8 ? (DWORD)WC_ERR_INVALID_CHARS : 0x6F0u | WC_NO_BEST_FIT_CHARS)) { *k32_last_error_ptr() = ERROR_INVALID_FLAGS; return 0; }
    if (defchar) defb = (unsigned char)defchar[0];
    if (cch == -1) { n = 0; while (src[n]) n++; n++; } else n = (size_t)cch;

    while (i < n) {
        unsigned char buf[4]; int nb; uint32_t c = src[i++];
        if (utf8) {
            if (c >= 0xD800 && c <= 0xDBFF && i < n && src[i] >= 0xDC00 && src[i] <= 0xDFFF) { c = 0x10000 + ((c - 0xD800) << 10) + (src[i] - 0xDC00); i++; }
            else if (c >= 0xD800 && c <= 0xDFFF) {
                if (flags & WC_ERR_INVALID_CHARS) { *k32_last_error_ptr() = ERROR_NO_UNICODE_TRANSLATION; return 0; }
                c = 0xFFFD;
            }
            if (c < 0x80) { buf[0] = (unsigned char)c; nb = 1; }
            else if (c < 0x800) { buf[0] = (unsigned char)(0xC0 | (c >> 6)); buf[1] = (unsigned char)(0x80 | (c & 0x3F)); nb = 2; }
            else if (c < 0x10000) { buf[0] = (unsigned char)(0xE0 | (c >> 12)); buf[1] = (unsigned char)(0x80 | ((c >> 6) & 0x3F)); buf[2] = (unsigned char)(0x80 | (c & 0x3F)); nb = 3; }
            else { buf[0] = (unsigned char)(0xF0 | (c >> 18)); buf[1] = (unsigned char)(0x80 | ((c >> 12) & 0x3F)); buf[2] = (unsigned char)(0x80 | ((c >> 6) & 0x3F)); buf[3] = (unsigned char)(0x80 | (c & 0x3F)); nb = 4; }
        } else {
            if (c >= 0xD800 && c <= 0xDBFF && i < n && src[i] >= 0xDC00 && src[i] <= 0xDFFF) { i++; c = 0x10000; }     /* a pair is one unmappable char */
            if (c > 0xFFFF || !wide_to_sb(cp, (WCHAR)c, &buf[0])) { buf[0] = defb; if (used_default) *used_default = TRUE; }
            nb = 1;
        }
        if (dst) {
            int k;
            if (out + (size_t)nb > (size_t)cb) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
            for (k = 0; k < nb; k++) dst[out + (size_t)k] = (char)buf[k];
        }
        out += (size_t)nb;
    }
    if (out > 0x7FFFFFFF) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    return (int)out;
}

/* ---------- compare / copy ---------- */
static size_t wlen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }

static int cmp_wide(const WCHAR *a, size_t an, const WCHAR *b, size_t bn, int fold)
{
    size_t i, n = an < bn ? an : bn;
    for (i = 0; i < n; i++) {
        WCHAR x = a[i], y = b[i];
        if (fold) { x = k32_towupper(x); y = k32_towupper(y); }
        if (x != y) return x < y ? CSTR_LESS_THAN : CSTR_GREATER_THAN;
    }
    return an == bn ? CSTR_EQUAL : (an < bn ? CSTR_LESS_THAN : CSTR_GREATER_THAN);
}

int WINAPI CompareStringOrdinal(LPCWSTR a, int ca, LPCWSTR b, int cb, BOOL ignore_case)
{
    if (!a || !b || ca < -1 || cb < -1) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    return cmp_wide(a, ca == -1 ? wlen(a) : (size_t)ca, b, cb == -1 ? wlen(b) : (size_t)cb, ignore_case);
}

int WINAPI CompareStringW(DWORD locale, DWORD flags, LPCWSTR a, int ca, LPCWSTR b, int cb)
{
    (void)locale;
    if (!a || !b || ca < -1 || cb < -1) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    return cmp_wide(a, ca == -1 ? wlen(a) : (size_t)ca, b, cb == -1 ? wlen(b) : (size_t)cb, (flags & NORM_IGNORECASE) != 0);
}

int WINAPI LCMapStringW(DWORD locale, DWORD flags, LPCWSTR src, int cch, LPWSTR dst, int cdst)
{
    size_t n, i;
    (void)locale;
    if (!src || cch < -1 || cdst < 0 || (!dst && cdst != 0) || ((flags & LCMAP_UPPERCASE) && (flags & LCMAP_LOWERCASE))) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0;
    }
    n = cch == -1 ? wlen(src) + 1 : (size_t)cch;
    if (cdst == 0) return (int)n;
    if (n > (size_t)cdst) { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    for (i = 0; i < n; i++)
        dst[i] = (flags & LCMAP_UPPERCASE) ? k32_towupper(src[i]) : (flags & LCMAP_LOWERCASE) ? k32_towlower(src[i]) : src[i];
    return (int)n;
}

int WINAPI lstrcmpW(LPCWSTR a, LPCWSTR b) { return cmp_wide(a, wlen(a), b, wlen(b), 0) - CSTR_EQUAL; }
int WINAPI lstrcmpiW(LPCWSTR a, LPCWSTR b) { return cmp_wide(a, wlen(a), b, wlen(b), 1) - CSTR_EQUAL; }

static int cmp_ansi(const char *a, const char *b, int fold)
{
    for (;; a++, b++) {
        unsigned char x = (unsigned char)*a, y = (unsigned char)*b;
        if (fold) { if (x >= 'a' && x <= 'z') x = (unsigned char)(x - 32); if (y >= 'a' && y <= 'z') y = (unsigned char)(y - 32); }
        if (x != y) return x < y ? -1 : 1;
        if (!x) return 0;
    }
}
int WINAPI lstrcmpA(LPCSTR a, LPCSTR b) { return cmp_ansi(a, b, 0); }
int WINAPI lstrcmpiA(LPCSTR a, LPCSTR b) { return cmp_ansi(a, b, 1); }       /* ASCII folding only */

int WINAPI lstrlenW(LPCWSTR s) { return s ? (int)wlen(s) : 0; }
int WINAPI lstrlenA(LPCSTR s) { int n = 0; if (!s) return 0; while (s[n]) n++; return n; }
LPWSTR WINAPI lstrcpyW(LPWSTR d, LPCWSTR s) { LPWSTR r = d; while ((*d++ = *s++)) { } return r; }
LPSTR  WINAPI lstrcpyA(LPSTR d, LPCSTR s) { LPSTR r = d; while ((*d++ = *s++)) { } return r; }
LPWSTR WINAPI lstrcpynW(LPWSTR d, LPCWSTR s, int n) { int i; if (n <= 0) return d; for (i = 0; i + 1 < n && s[i]; i++) d[i] = s[i]; d[i] = 0; return d; }
LPSTR  WINAPI lstrcpynA(LPSTR d, LPCSTR s, int n) { int i; if (n <= 0) return d; for (i = 0; i + 1 < n && s[i]; i++) d[i] = s[i]; d[i] = 0; return d; }
LPWSTR WINAPI lstrcatW(LPWSTR d, LPCWSTR s) { lstrcpyW(d + wlen(d), s); return d; }
LPSTR  WINAPI lstrcatA(LPSTR d, LPCSTR s) { lstrcpyA(d + lstrlenA(d), s); return d; }
