/* C-locale character classification (ASCII); also the wide variants for the BMP's ASCII/Latin-1 range. */
#include "../include/ctype.h"
#include "../include/wctype.h"

int isupper(int c) { return c >= 'A' && c <= 'Z'; }
int islower(int c) { return c >= 'a' && c <= 'z'; }
int isalpha(int c) { return isupper(c) || islower(c); }
int isdigit(int c) { return c >= '0' && c <= '9'; }
int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isxdigit(int c) { return isdigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'); }
int isblank(int c) { return c == ' ' || c == '\t'; }
int isspace(int c) { return c == ' ' || (c >= '\t' && c <= '\r'); }
int iscntrl(int c) { return (c >= 0 && c < 32) || c == 127; }
int isprint(int c) { return c >= 32 && c < 127; }
int isgraph(int c) { return c > 32 && c < 127; }
int ispunct(int c) { return isgraph(c) && !isalnum(c); }
int isascii(int c) { return (c & ~0x7f) == 0; }
int toascii(int c) { return c & 0x7f; }
int tolower(int c) { return isupper(c) ? c + 32 : c; }
int toupper(int c) { return islower(c) ? c - 32 : c; }

int iswupper(wint_t c) { return isupper(c) || (c >= 0xC0 && c <= 0xDE && c != 0xD7); }
int iswlower(wint_t c) { return islower(c) || (c >= 0xDF && c <= 0xFF && c != 0xF7); }
int iswalpha(wint_t c) { return isalpha(c) || iswupper(c) || iswlower(c) || c == 0xAA || c == 0xB5 || c == 0xBA || (c >= 0x100 && c < 0x2000) || (c >= 0x3040 && c < 0xA000); }
int iswdigit(wint_t c) { return isdigit(c); }
int iswalnum(wint_t c) { return iswalpha(c) || iswdigit(c); }
int iswxdigit(wint_t c) { return isxdigit(c); }
int iswblank(wint_t c) { return isblank(c); }
int iswspace(wint_t c) { return isspace(c) || c == 0x85 || c == 0xA0 || c == 0x2028 || c == 0x2029 || c == 0x3000 || (c >= 0x2000 && c <= 0x200A); }
int iswcntrl(wint_t c) { return iscntrl(c) || (c >= 0x80 && c < 0xA0); }
int iswprint(wint_t c) { return c >= 32 && !iswcntrl(c) && c != 0x7f && c < 0x110000; }
int iswgraph(wint_t c) { return iswprint(c) && !iswspace(c); }
int iswpunct(wint_t c) { return iswgraph(c) && !iswalnum(c); }
wint_t towlower(wint_t c) { if (isupper(c) || (c >= 0xC0 && c <= 0xDE && c != 0xD7)) return c + 32; return c; }
wint_t towupper(wint_t c) { if (islower(c) || (c >= 0xE0 && c <= 0xFE && c != 0xF7)) return c - 32; return c; }
