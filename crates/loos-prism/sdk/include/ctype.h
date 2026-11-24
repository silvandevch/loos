#ifndef _CTYPE_H
#define _CTYPE_H
#include <_prism_cdefs.h>
#define _CTYPE_A 0x00000100L
#define _CTYPE_C 0x00000200L
#define _CTYPE_D 0x00000400L
#define _CTYPE_G 0x00000800L
#define _CTYPE_L 0x00001000L
#define _CTYPE_P 0x00002000L
#define _CTYPE_S 0x00004000L
#define _CTYPE_U 0x00008000L
#define _CTYPE_X 0x00010000L
#define _CTYPE_B 0x00020000L
#define _CTYPE_R 0x00040000L
#define _CTYPE_I 0x00080000L
#define _CTYPE_T 0x00100000L
#define _CTYPE_Q 0x00200000L
#define _CTYPE_SW0 0x20000000L
#define _CTYPE_SW1 0x40000000L
#define _CTYPE_SW2 0x80000000L
#define _CTYPE_SW3 0xc0000000L
#define _CTYPE_SWM 0xe0000000L
#define _CTYPE_SWS 30
#define _CACHED_RUNES 256
typedef struct {
    char __magic[8];
    char __encoding[32];
    rune_t (*__sgetrune)(const char *, size_t, char const **);
    int (*__sputrune)(rune_t, char *, size_t, char **);
    rune_t __invalid_rune;
    uint32_t __runetype[_CACHED_RUNES];
    rune_t __maplower[_CACHED_RUNES];
    rune_t __mapupper[_CACHED_RUNES];
    void *__runetype_ext[3];
    void *__variable;
    int __variable_len;
    int __ncharclasses;
    void *__charclasses;
} _RuneLocale;
__BEGIN_DECLS
extern _RuneLocale _DefaultRuneLocale;
int isalnum(int); int isalpha(int); int isblank(int); int iscntrl(int); int isdigit(int);
int isgraph(int); int islower(int); int isprint(int); int ispunct(int); int isspace(int);
int isupper(int); int isxdigit(int); int tolower(int); int toupper(int); int isascii(int);
__END_DECLS
#include <xlocale.h>
#endif
