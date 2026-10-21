/* FormatMessage: system message text for Win32 error codes, insert substitution, line-break handling.
 *
 * Message texts for FORMAT_MESSAGE_FROM_SYSTEM are the built-in English table below (a few dozen of the
 * errors programs actually show), reproduced in our own wording. Message tables embedded in modules
 * (FORMAT_MESSAGE_FROM_HMODULE) are not implemented yet and fail with ERROR_MR_MID_NOT_FOUND.
 *
 * Inserts: %1..%99 with an optional !printf-style! format (s S c C d i u x X o p, width/precision/flags,
 * h l ll w I I32 I64 z sizes); %n %r %t %0 %% and the literal escapes. Arguments come from an array of
 * pointer-sized values (FORMAT_MESSAGE_ARGUMENT_ARRAY) or from the caller's va_list, which on the Windows
 * x64 ABI is exactly such an array. */
#include "k32_internal.h"
#include <string.h>

static const struct { DWORD id; const char *text; } sys_msgs[] = {
    { 0, "The operation completed successfully." }, { 1, "Incorrect function." }, { 2, "The system cannot find the file specified." },
    { 3, "The system cannot find the path specified." }, { 4, "The system cannot open the file." }, { 5, "Access is denied." },
    { 6, "The handle is invalid." }, { 8, "Not enough memory resources are available to process this command." },
    { 10, "The environment is incorrect." }, { 11, "An attempt was made to load a program with an incorrect format." },
    { 13, "The data is invalid." }, { 14, "Not enough memory resources are available to complete this operation." },
    { 15, "The system cannot find the drive specified." }, { 16, "The directory cannot be removed." },
    { 17, "The system cannot move the file to a different disk drive." }, { 18, "There are no more files." },
    { 19, "The media is write protected." }, { 20, "The system cannot find the device specified." }, { 21, "The device is not ready." },
    { 23, "Data error (cyclic redundancy check)." }, { 29, "The system cannot write to the specified device." },
    { 30, "The system cannot read from the specified device." }, { 31, "A device attached to the system is not functioning." },
    { 32, "The process cannot access the file because it is being used by another process." },
    { 33, "The process cannot access the file because another process has locked a portion of the file." },
    { 38, "Reached the end of the file." }, { 39, "The disk is full." }, { 50, "The request is not supported." },
    { 53, "The network path was not found." }, { 54, "The network is busy." }, { 65, "Network access is denied." },
    { 67, "The network name cannot be found." }, { 80, "The file exists." }, { 82, "The directory or file cannot be created." },
    { 87, "The parameter is incorrect." }, { 109, "The pipe has been ended." }, { 110, "The system cannot open the device or file specified." },
    { 111, "The file name is too long." }, { 112, "There is not enough space on the disk." },
    { 120, "This function is not supported on this system." }, { 122, "The data area passed to a system call is too small." },
    { 123, "The filename, directory name, or volume label syntax is incorrect." }, { 126, "The specified module could not be found." },
    { 127, "The specified procedure could not be found." }, { 128, "There are no child processes to wait for." },
    { 129, "%1 is not a valid Win32 application." }, { 131, "An attempt was made to move the file pointer before the beginning of the file." },
    { 145, "The directory is not empty." }, { 161, "The specified path is invalid." }, { 170, "The requested resource is in use." },
    { 182, "The operating system cannot run %1." }, { 183, "Cannot create a file when that file already exists." },
    { 193, "%1 is not a valid Win32 application." }, { 206, "The filename or extension is too long." }, { 232, "The pipe is being closed." },
    { 234, "More data is available." }, { 258, "The wait operation timed out." }, { 259, "No more data is available." },
    { 267, "The directory name is invalid." }, { 288, "Attempt to release mutex not owned by caller." },
    { 298, "Too many posts were made to a semaphore." }, { 317, "The system cannot find message text for message number 0x%1 in the message file for %2." },
    { 487, "Attempt to access invalid address." }, { 740, "The requested operation requires elevation." },
    { 995, "The I/O operation has been aborted because of either a thread exit or an application request." },
    { 997, "Overlapped I/O operation is in progress." }, { 998, "Invalid access to memory location." }, { 1004, "Invalid flags." },
    { 1113, "No mapping for the Unicode character exists in the target multi-byte code page." },
    { 1150, "The specified program requires a newer version of Windows." }, { 1168, "Element not found." },
    { 1314, "A required privilege is not held by the client." }, { 1460, "This operation returned because the timeout period expired." },
    { 1722, "The RPC server is unavailable." }, { 1814, "The specified resource name cannot be found in the image file." },
};

typedef struct { WCHAR *p; size_t n, cap; int oom; } sbuf;

static void sb_put(sbuf *b, const WCHAR *s, size_t n)
{
    if (b->oom) return;
    if (b->n + n + 1 > b->cap) {
        size_t nc = b->cap ? b->cap * 2 : 128;
        WCHAR *np;
        while (nc < b->n + n + 1) nc *= 2;
        np = b->p ? RtlReAllocateHeap(RtlGetProcessHeap(), 0, b->p, nc * sizeof(WCHAR)) : RtlAllocateHeap(RtlGetProcessHeap(), 0, nc * sizeof(WCHAR));
        if (!np) { b->oom = 1; return; }
        b->p = np; b->cap = nc;
    }
    memcpy(b->p + b->n, s, n * sizeof(WCHAR));
    b->n += n;
    b->p[b->n] = 0;
}
static void sb_putc(sbuf *b, WCHAR c) { sb_put(b, &c, 1); }

/* one !format! conversion of an argument value into the buffer */
static void fmt_arg(sbuf *b, const WCHAR *spec, size_t speclen, DWORD_PTR v, int ansi_default)
{
    int left = 0, plus = 0, space = 0, zero = 0, alt = 0, width = 0, prec = -1, is64 = 0, wide_override = 0, narrow_override = 0;
    size_t i = 0;
    WCHAR conv;
    WCHAR tmp[80];
    size_t tn = 0;
    int neg = 0;
    uint64_t u;
    const char *digits = "0123456789abcdef";
    int upper = 0, base = 10, is_signed = 0;
    WCHAR *body = tmp;
    size_t blen = 0, pad;

    for (; i < speclen; i++) {
        if (spec[i] == '-') left = 1; else if (spec[i] == '+') plus = 1; else if (spec[i] == ' ') space = 1; else if (spec[i] == '0') zero = 1; else if (spec[i] == '#') alt = 1; else break;
    }
    while (i < speclen && spec[i] >= '0' && spec[i] <= '9') width = width * 10 + (spec[i++] - '0');
    if (i < speclen && spec[i] == '.') { i++; prec = 0; while (i < speclen && spec[i] >= '0' && spec[i] <= '9') prec = prec * 10 + (spec[i++] - '0'); }
    for (; i < speclen; i++) {                                    /* size prefixes */
        if (spec[i] == 'h') narrow_override = 1;
        else if (spec[i] == 'l' && i + 1 < speclen && spec[i + 1] == 'l') { is64 = 1; i++; }
        else if (spec[i] == 'l' || spec[i] == 'w') wide_override = 1;
        else if (spec[i] == 'z' || spec[i] == 't' || spec[i] == 'j') is64 = 1;
        else if (spec[i] == 'I') { if (i + 2 < speclen && spec[i + 1] == '6' && spec[i + 2] == '4') { is64 = 1; i += 2; } else if (i + 2 < speclen && spec[i + 1] == '3' && spec[i + 2] == '2') i += 2; else is64 = 1; }
        else break;
    }
    conv = i < speclen ? spec[i] : 's';
    (void)alt;

    switch (conv) {
    case 's': case 'S': {
        int ansi = (conv == 'S') ? !wide_override : (narrow_override || (ansi_default && !wide_override));
        if (conv == 'S' && narrow_override) ansi = 1;
        if (ansi) {
            const char *s = (const char *)v;
            size_t n = 0;
            if (!s) s = "(null)";
            while (s[n] && (prec < 0 || (int)n < prec)) n++;
            pad = (size_t)width > n ? (size_t)width - n : 0;
            if (!left) while (pad--) sb_putc(b, ' ');
            { size_t k; for (k = 0; k < n; k++) sb_putc(b, (WCHAR)(unsigned char)s[k]); }
            if (left) { pad = (size_t)width > n ? (size_t)width - n : 0; while (pad--) sb_putc(b, ' '); }
        } else {
            const WCHAR *s = (const WCHAR *)v;
            static const WCHAR nullw[] = { '(', 'n', 'u', 'l', 'l', ')', 0 };
            size_t n = 0;
            if (!s) s = nullw;
            while (s[n] && (prec < 0 || (int)n < prec)) n++;
            pad = (size_t)width > n ? (size_t)width - n : 0;
            if (!left) while (pad--) sb_putc(b, ' ');
            sb_put(b, s, n);
            if (left) { pad = (size_t)width > n ? (size_t)width - n : 0; while (pad--) sb_putc(b, ' '); }
        }
        return;
    }
    case 'c': case 'C': {
        WCHAR ch = (conv == 'C' || narrow_override) ? (WCHAR)(unsigned char)v : (WCHAR)v;
        pad = width > 1 ? (size_t)width - 1 : 0;
        if (!left) while (pad--) sb_putc(b, ' ');
        sb_putc(b, ch);
        if (left) { pad = width > 1 ? (size_t)width - 1 : 0; while (pad--) sb_putc(b, ' '); }
        return;
    }
    case 'd': case 'i': is_signed = 1; base = 10; break;
    case 'u': base = 10; break;
    case 'x': base = 16; break;
    case 'X': base = 16; upper = 1; break;
    case 'o': base = 8; break;
    case 'p': base = 16; upper = 1; is64 = 1; prec = 16; break;
    default: sb_putc(b, '%'); return;
    }
    if (is_signed) {
        int64_t sv = is64 ? (int64_t)v : (int64_t)(int32_t)(uint32_t)v;
        neg = sv < 0; u = neg ? (uint64_t)0 - (uint64_t)sv : (uint64_t)sv;
    } else u = is64 ? (uint64_t)v : (uint64_t)(uint32_t)v;
    if (upper) digits = "0123456789ABCDEF";
    {
        WCHAR rev[70]; size_t rn = 0;
        if (u == 0) rev[rn++] = '0';
        while (u && rn < 69) { rev[rn++] = (WCHAR)(unsigned char)digits[u % (unsigned)base]; u /= (unsigned)base; }
        while (prec >= 0 && (int)rn < prec && rn < 69) rev[rn++] = '0';
        if (neg) tmp[tn++] = '-'; else if (plus && is_signed) tmp[tn++] = '+'; else if (space && is_signed) tmp[tn++] = ' ';
        while (rn) tmp[tn++] = rev[--rn];
    }
    body = tmp; blen = tn;
    pad = (size_t)width > blen ? (size_t)width - blen : 0;
    if (left) { sb_put(b, body, blen); while (pad--) sb_putc(b, ' '); }
    else if (zero && prec < 0) {                                  /* zero padding goes after the sign */
        size_t sign = (blen && (body[0] == '-' || body[0] == '+' || body[0] == ' ')) ? 1 : 0;
        sb_put(b, body, sign); while (pad--) sb_putc(b, '0'); sb_put(b, body + sign, blen - sign);
    } else { while (pad--) sb_putc(b, ' '); sb_put(b, body, blen); }
}

/* expand template (UTF-16) into out; returns 0 on bad arguments */
static int expand(sbuf *out, const WCHAR *t, const DWORD_PTR *av, int ansi_default, int ignore_width)
{
    size_t i = 0;
    while (t[i]) {
        WCHAR c = t[i];
        if (c == '\r' && t[i + 1] == '\n' && ignore_width) { sb_putc(out, ' '); i += 2; continue; }
        if ((c == '\n' || c == '\r') && ignore_width) { sb_putc(out, ' '); i++; continue; }
        if (c != '%') { sb_putc(out, c); i++; continue; }
        i++;
        c = t[i];
        if (c >= '1' && c <= '9') {
            unsigned n = (unsigned)(c - '0');
            size_t specstart, speclen = 0;
            static const WCHAR dflt[] = { 's' };
            i++;
            if (t[i] >= '0' && t[i] <= '9') { n = n * 10 + (unsigned)(t[i] - '0'); i++; }
            if (!av) return 0;
            if (t[i] == '!') {                                    /* !format! */
                size_t j = i + 1;
                while (t[j] && t[j] != '!') j++;
                if (t[j] == '!') { specstart = i + 1; speclen = j - i - 1; i = j + 1; fmt_arg(out, t + specstart, speclen, av[n - 1], ansi_default); continue; }
            }
            fmt_arg(out, dflt, 1, av[n - 1], ansi_default);
            continue;
        }
        switch (c) {
        case 'n': sb_putc(out, '\r'); sb_putc(out, '\n'); i++; break;
        case 'r': sb_putc(out, '\r'); i++; break;
        case 't': sb_putc(out, '\t'); i++; break;
        case '0': return 1;                                       /* %0: end of message, no trailing newline */
        case 0: sb_putc(out, '%'); break;
        default: sb_putc(out, c); i++; break;                     /* %% %. %! %space and unknown escapes: the character itself */
        }
    }
    return 1;
}

static const char *sys_text(DWORD id)
{
    size_t i;
    for (i = 0; i < sizeof sys_msgs / sizeof sys_msgs[0]; i++) if (sys_msgs[i].id == id) return sys_msgs[i].text;
    return NULL;
}

/* shared worker; produces a heap-allocated, NUL-terminated UTF-16 result */
static WCHAR *format_to_wide(DWORD flags, LPCVOID source, DWORD id, PVOID *args, int ansi_default, size_t *out_len)
{
    sbuf b = {0};
    WCHAR *tmpl = NULL, *owned = NULL;
    const DWORD_PTR *av = NULL;
    int ignore_width = (flags & FORMAT_MESSAGE_MAX_WIDTH_MASK) == FORMAT_MESSAGE_MAX_WIDTH_MASK, from_system = 0;

    if (flags & FORMAT_MESSAGE_FROM_STRING) {
        if (!source) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
        if (ansi_default) {                                       /* FormatMessageA: the template is ANSI */
            int n = MultiByteToWideChar(CP_ACP, 0, (LPCSTR)source, -1, NULL, 0);
            if (n <= 0) return NULL;
            owned = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)n * sizeof(WCHAR));
            if (!owned) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return NULL; }
            MultiByteToWideChar(CP_ACP, 0, (LPCSTR)source, -1, owned, n);
            tmpl = owned;
        } else tmpl = (WCHAR *)source;
    } else if (flags & (FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_FROM_HMODULE)) {
        const char *t = (flags & FORMAT_MESSAGE_FROM_SYSTEM) ? sys_text(id & 0xFFFF) : NULL;
        size_t n, k;
        if (!t) { *k32_last_error_ptr() = ERROR_MR_MID_NOT_FOUND; return NULL; }
        n = strlen(t);
        owned = RtlAllocateHeap(RtlGetProcessHeap(), 0, (n + 1) * sizeof(WCHAR));
        if (!owned) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return NULL; }
        for (k = 0; k <= n; k++) owned[k] = (WCHAR)(unsigned char)t[k];
        tmpl = owned; from_system = 1;
    } else { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }

    if (flags & FORMAT_MESSAGE_IGNORE_INSERTS) {
        { size_t tl = 0; while (tmpl[tl]) tl++; sb_put(&b, tmpl, tl); }
    } else {
        if (args) av = (flags & FORMAT_MESSAGE_ARGUMENT_ARRAY) ? (const DWORD_PTR *)args : (const DWORD_PTR *)*args;
        if (!expand(&b, tmpl, av, ansi_default, ignore_width)) {
            if (b.p) RtlFreeHeap(RtlGetProcessHeap(), 0, b.p);
            if (owned) RtlFreeHeap(RtlGetProcessHeap(), 0, owned);
            *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
            return NULL;
        }
    }
    /* system messages are stored without their line ending; Windows text ends in CR LF unless widths are ignored */
    if (from_system && !ignore_width) { sb_putc(&b, '\r'); sb_putc(&b, '\n'); }
    if (owned) RtlFreeHeap(RtlGetProcessHeap(), 0, owned);
    if (b.oom || !b.p) { if (b.p) RtlFreeHeap(RtlGetProcessHeap(), 0, b.p); *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return NULL; }
    *out_len = b.n;
    return b.p;
}

DWORD WINAPI FormatMessageW(DWORD flags, LPCVOID source, DWORD id, DWORD lang, LPWSTR buf, DWORD size, PVOID *args)
{
    size_t n = 0;
    WCHAR *r;
    (void)lang;
    if (!(flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) && !buf) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (!(r = format_to_wide(flags, source, id, args, 0, &n))) return 0;
    if (flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) {
        *(LPWSTR *)buf = r;                                       /* freed by the caller with LocalFree */
        return (DWORD)n;
    }
    if (n + 1 > size) { RtlFreeHeap(RtlGetProcessHeap(), 0, r); *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    memcpy(buf, r, (n + 1) * sizeof(WCHAR));
    RtlFreeHeap(RtlGetProcessHeap(), 0, r);
    return (DWORD)n;
}

DWORD WINAPI FormatMessageA(DWORD flags, LPCVOID source, DWORD id, DWORD lang, LPSTR buf, DWORD size, PVOID *args)
{
    size_t n = 0;
    WCHAR *r;
    int bytes;
    (void)lang;
    if (!(flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) && !buf) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return 0; }
    if (!(r = format_to_wide(flags, source, id, args, 1, &n))) return 0;
    bytes = WideCharToMultiByte(CP_ACP, 0, r, (int)n, NULL, 0, NULL, NULL);
    if (flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) {
        char *out = RtlAllocateHeap(RtlGetProcessHeap(), 0, (SIZE_T)bytes + 1);
        if (out) { WideCharToMultiByte(CP_ACP, 0, r, (int)n, out, bytes, NULL, NULL); out[bytes] = 0; *(LPSTR *)buf = out; }
        else *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
        RtlFreeHeap(RtlGetProcessHeap(), 0, r);
        return out ? (DWORD)bytes : 0;
    }
    if ((DWORD)bytes + 1 > size) { RtlFreeHeap(RtlGetProcessHeap(), 0, r); *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    WideCharToMultiByte(CP_ACP, 0, r, (int)n, buf, bytes, NULL, NULL);
    buf[bytes] = 0;
    RtlFreeHeap(RtlGetProcessHeap(), 0, r);
    return (DWORD)bytes;
}
