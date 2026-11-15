/* Wide-character stdio over FILE (UTF-8 encoded on the stream). */
#include "prism.h"

int ungetc(int, FILE *);
int getc(FILE *);
int putc(int, FILE *);

wint_t fputwc(wint_t w, FILE *fp) {
    unsigned c = (unsigned)w;
    unsigned char b[4]; int n;
    if (c < 0x80) { b[0] = c; n = 1; }
    else if (c < 0x800) { b[0] = 0xc0 | c >> 6; b[1] = 0x80 | (c & 63); n = 2; }
    else if (c < 0x10000) { b[0] = 0xe0 | c >> 12; b[1] = 0x80 | (c >> 6 & 63); b[2] = 0x80 | (c & 63); n = 3; }
    else { b[0] = 0xf0 | c >> 18; b[1] = 0x80 | (c >> 12 & 63); b[2] = 0x80 | (c >> 6 & 63); b[3] = 0x80 | (c & 63); n = 4; }
    for (int i = 0; i < n; i++) if (putc(b[i], fp) == EOF) return EOF;
    return w;
}
wint_t putwc(wint_t w, FILE *fp) { return fputwc(w, fp); }
wint_t putwchar(wint_t w) { return fputwc(w, stdout); }

wint_t fgetwc(FILE *fp) {
    int c = getc(fp);
    if (c == EOF || c < 0x80) return c;
    int n = c >= 0xf0 ? 3 : c >= 0xe0 ? 2 : c >= 0xc0 ? 1 : 0;
    unsigned w = c & (0x3f >> n);
    while (n--) {
        int d = getc(fp);
        if (d == EOF || (d & 0xc0) != 0x80) { ungetc(d, fp); return 0xfffd; }
        w = w << 6 | (d & 63);
    }
    return (wint_t)w;
}
wint_t getwc(FILE *fp) { return fgetwc(fp); }
wint_t getwchar(void) { return fgetwc(stdin); }
int fputws(const int *s, FILE *fp) {
    for (; *s; s++) if (fputwc(*s, fp) == EOF) return -1;
    return 0;
}
