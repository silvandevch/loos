/* scanf family (variadic, so C per docs/LANGUAGE.md §C.6). Reads from a string or a FILE. */
#include "prism.h"

int ungetc(int, FILE *);
int getc(FILE *);

struct in { const unsigned char *s; FILE *fp; int n; };

static int get(struct in *in) {
    int c;
    if (in->fp) c = getc(in->fp);
    else c = *in->s ? *in->s++ : EOF;
    if (c != EOF) in->n++;
    return c;
}

static void unget(struct in *in, int c) {
    if (c == EOF) return;
    in->n--;
    if (in->fp) ungetc(c, in->fp);
    else in->s--;
}

static int space(int c) { return c == ' ' || (c >= '\t' && c <= '\r'); }

static int digit_val(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return 99;
}

/* Integer of `base` (0: auto) within `width` chars; returns 0 when no digits matched. */
static int scan_int(struct in *in, int base, int width, unsigned long long *out, int *neg) {
    int c = get(in), any = 0;
    *neg = 0;
    if (width <= 0) width = 1 << 30;
    if ((c == '-' || c == '+') && width > 1) { *neg = c == '-'; c = get(in); width--; }
    if ((base == 0 || base == 16) && c == '0' && width > 1) {
        int d = get(in);
        if ((d == 'x' || d == 'X') && width > 2) { base = 16; c = get(in); width -= 2; }
        else { unget(in, d); if (base == 0) base = 8; any = 1; c = '0'; }
    }
    if (base == 0) base = 10;
    unsigned long long v = 0;
    while (width-- > 0 && digit_val(c) < base) { v = v * base + digit_val(c); any = 1; c = get(in); }
    unget(in, c);
    *out = v;
    return any;
}

static int scan_float(struct in *in, int width, double *out) {
    int c = get(in), any = 0, neg = 0, ex = 0, eneg = 0, fd = 0;
    double v = 0;
    if (width <= 0) width = 1 << 30;
    if ((c == '-' || c == '+') && width > 1) { neg = c == '-'; c = get(in); width--; }
    while (width > 0 && c >= '0' && c <= '9') { v = v * 10 + (c - '0'); any = 1; c = get(in); width--; }
    if (width > 0 && c == '.') {
        c = get(in); width--;
        while (width > 0 && c >= '0' && c <= '9') { v = v * 10 + (c - '0'); fd++; any = 1; c = get(in); width--; }
    }
    if (any && width > 1 && (c == 'e' || c == 'E')) {
        int d = get(in); width--;
        if (d == '-' || d == '+') { eneg = d == '-'; d = get(in); width--; }
        while (width > 0 && d >= '0' && d <= '9') { ex = ex * 10 + (d - '0'); d = get(in); width--; }
        c = d;
    }
    unget(in, c);
    if (!any) return 0;
    int e = (eneg ? -ex : ex) - fd;
    double p = 1;
    for (int i = e < 0 ? -e : e; i > 0; i--) p *= 10;
    v = e < 0 ? v / p : v * p;
    *out = neg ? -v : v;
    return 1;
}

static int vscan(struct in *in, const char *f, va_list ap) {
    int done = 0, c;
    for (; *f; f++) {
        if (space((unsigned char)*f)) {
            while (space(c = get(in))) {}
            unget(in, c);
            continue;
        }
        if (*f != '%' || f[1] == '%') {
            if (*f == '%') f++;
            c = get(in);
            if (c != (unsigned char)*f) { unget(in, c); goto out; }
            continue;
        }
        f++;
        int skip = 0, width = 0, size = 0; /* size: -2 hh, -1 h, 0, 1 l, 2 ll/j/q/z/t */
        if (*f == '*') { skip = 1; f++; }
        while (*f >= '0' && *f <= '9') width = width * 10 + (*f++ - '0');
        for (;; f++) {
            if (*f == 'h') size = size == -1 ? -2 : -1;
            else if (*f == 'l') size = size == 1 ? 2 : 1;
            else if (*f == 'L' || *f == 'j' || *f == 'q' || *f == 'z' || *f == 't') size = 2;
            else break;
        }
        int conv = *f;
        if (!conv) break;
        if (conv != 'c' && conv != '[' && conv != 'n') {
            while (space(c = get(in))) {}
            if (c == EOF) goto out;
            unget(in, c);
        }
        switch (conv) {
        case 'n':
            if (!skip) {
                int *p = va_arg(ap, int *);
                *p = in->n;
            }
            break;
        case 'd': case 'i': case 'u': case 'x': case 'X': case 'o': case 'p': {
            unsigned long long v; int neg;
            int base = conv == 'i' ? 0 : conv == 'o' ? 8 : (conv == 'x' || conv == 'X' || conv == 'p') ? 16 : 10;
            if (!scan_int(in, base, width, &v, &neg)) goto out;
            if (neg) v = -v;
            if (skip) break;
            if (conv == 'p') size = 2;
            void *p = va_arg(ap, void *);
            if (size == -2) *(char *)p = (char)v;
            else if (size == -1) *(short *)p = (short)v;
            else if (size == 0) *(int *)p = (int)v;
            else *(long long *)p = (long long)v;
            done++;
            break;
        }
        case 'f': case 'e': case 'g': case 'E': case 'G': case 'a': {
            double v;
            if (!scan_float(in, width, &v)) goto out;
            if (skip) break;
            if (size >= 1) *va_arg(ap, double *) = v;
            else *va_arg(ap, float *) = (float)v;
            done++;
            break;
        }
        case 's': case 'c': case '[': {
            unsigned char set[256];
            int inv = 0, i;
            if (conv == '[') {
                f++;
                if (*f == '^') { inv = 1; f++; }
                for (i = 0; i < 256; i++) set[i] = 0;
                if (*f == ']') { set[']'] = 1; f++; }
                for (; *f && *f != ']'; f++) {
                    if (f[1] == '-' && f[2] && f[2] != ']') { for (i = (unsigned char)f[0]; i <= (unsigned char)f[2]; i++) set[i] = 1; f += 2; }
                    else set[(unsigned char)*f] = 1;
                }
                if (!*f) goto out;
            }
            if (width == 0) width = conv == 'c' ? 1 : 1 << 30;
            char *p = skip ? NULL : va_arg(ap, char *);
            int got = 0;
            while (got < width) {
                c = get(in);
                if (c == EOF) break;
                if (conv == 's' && space(c)) { unget(in, c); break; }
                if (conv == '[' && set[c] == inv) { unget(in, c); break; }
                if (p) p[got] = (char)c;
                got++;
            }
            if (got == 0 || (conv == 'c' && got < width)) goto out;
            if (p && conv != 'c') p[got] = 0;
            if (!skip) done++;
            break;
        }
        default:
            goto out;
        }
    }
out:
    /* EOF before the first conversion: EOF, as C requires. */
    if (done == 0 && !in->fp && !*in->s) return EOF;
    return done;
}

int vsscanf(const char *s, const char *f, va_list ap) {
    struct in in = {(const unsigned char *)s, NULL, 0};
    return vscan(&in, f, ap);
}
int sscanf(const char *s, const char *f, ...) {
    va_list ap; va_start(ap, f);
    int r = vsscanf(s, f, ap);
    va_end(ap); return r;
}
int vfscanf(FILE *fp, const char *f, va_list ap) {
    struct in in = {NULL, fp, 0};
    return vscan(&in, f, ap);
}
int fscanf(FILE *fp, const char *f, ...) {
    va_list ap; va_start(ap, f);
    int r = vfscanf(fp, f, ap);
    va_end(ap); return r;
}
int vscanf(const char *f, va_list ap) { return vfscanf(stdin, f, ap); }
int scanf(const char *f, ...) {
    va_list ap; va_start(ap, f);
    int r = vfscanf(stdin, f, ap);
    va_end(ap); return r;
}
