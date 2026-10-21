/* The few libc functions the shell needs beyond string.h (integer-only printf family, atoi). */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

struct out { char *buf; size_t cap, n; };

static void put(struct out *o, char c)
{
    if (o->n + 1 < o->cap) o->buf[o->n] = c;
    o->n++;
}

static void pad(struct out *o, int count, char c) { while (count-- > 0) put(o, c); }

int vsnprintf(char *buf, size_t cap, const char *fmt, va_list ap)
{
    struct out o = { buf, cap, 0 };
    for (; *fmt; fmt++) {
        if (*fmt != '%') { put(&o, *fmt); continue; }
        fmt++;
        int left = 0, zero = 0, width = 0, prec = -1, lng = 0;
        for (;; fmt++) {
            if (*fmt == '-') left = 1; else if (*fmt == '0') zero = 1; else if (*fmt == '+' || *fmt == ' ' || *fmt == '#') {} else break;
        }
        if (*fmt == '*') { width = va_arg(ap, int); if (width < 0) { left = 1; width = -width; } fmt++; }
        else while (*fmt >= '0' && *fmt <= '9') width = width * 10 + (*fmt++ - '0');
        if (*fmt == '.') {
            fmt++; prec = 0;
            if (*fmt == '*') { prec = va_arg(ap, int); fmt++; }
            else while (*fmt >= '0' && *fmt <= '9') prec = prec * 10 + (*fmt++ - '0');
        }
        for (;; fmt++) {
            if (*fmt == 'l') lng++; else if (*fmt == 'z' || *fmt == 'j' || *fmt == 't') lng = 2; else if (*fmt == 'h') {} else break;
        }
        char conv = *fmt;
        if (!conv) break;
        if (conv == '%') { put(&o, '%'); continue; }
        if (conv == 'c') { char c = (char)va_arg(ap, int); if (!left) pad(&o, width - 1, ' '); put(&o, c); if (left) pad(&o, width - 1, ' '); continue; }
        if (conv == 's') {
            const char *s = va_arg(ap, const char *);
            if (!s) s = "(null)";
            size_t n = 0;
            while (s[n] && (prec < 0 || (int)n < prec)) n++;
            if (!left) pad(&o, width - (int)n, ' ');
            for (size_t i = 0; i < n; i++) put(&o, s[i]);
            if (left) pad(&o, width - (int)n, ' ');
            continue;
        }
        uint64_t v; int neg = 0; unsigned base = 10; const char *digits = "0123456789abcdef";
        if (conv == 'd' || conv == 'i') {
            int64_t s = lng >= 2 ? va_arg(ap, long long) : lng == 1 ? va_arg(ap, long) : va_arg(ap, int);
            neg = s < 0; v = neg ? 0 - (uint64_t)s : (uint64_t)s;
        } else {
            v = lng >= 2 ? va_arg(ap, unsigned long long) : lng == 1 ? va_arg(ap, unsigned long) : va_arg(ap, unsigned);
            if (conv == 'x') base = 16; else if (conv == 'X') { base = 16; digits = "0123456789ABCDEF"; }
            else if (conv == 'p') { base = 16; v = (uint64_t)(uintptr_t)va_arg(ap, void *); put(&o, '0'); put(&o, 'x'); }
            else if (conv != 'u') { put(&o, '%'); put(&o, conv); continue; }
        }
        char tmp[24]; int n = 0;
        do { tmp[n++] = digits[v % base]; v /= base; } while (v);
        int total = n + neg;
        int zeros = prec > n ? prec - n : 0;
        if (zero && !left && prec < 0 && width > total) zeros = width - total;
        if (!left) pad(&o, width - total - zeros, ' ');
        if (neg) put(&o, '-');
        pad(&o, zeros, '0');
        while (n) put(&o, tmp[--n]);
        if (left) pad(&o, width - total - zeros, ' ');
    }
    if (cap) o.buf[o.n < cap ? o.n : cap - 1] = 0;
    return (int)o.n;
}

int snprintf(char *buf, size_t cap, const char *fmt, ...)
{
    va_list ap; va_start(ap, fmt);
    int n = vsnprintf(buf, cap, fmt, ap);
    va_end(ap);
    return n;
}

int atoi(const char *s)
{
    int neg = 0, v = 0;
    while (*s == ' ' || *s == '\t') s++;
    if (*s == '-' || *s == '+') neg = *s++ == '-';
    while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
    return neg ? -v : v;
}
