#ifndef _UCHAR_H
#define _UCHAR_H
#include <_prism_cdefs.h>
__BEGIN_DECLS
size_t mbrtoc16(char16_t *, const char *, size_t, mbstate_t *); size_t c16rtomb(char *, char16_t, mbstate_t *);
size_t mbrtoc32(char32_t *, const char *, size_t, mbstate_t *); size_t c32rtomb(char *, char32_t, mbstate_t *);
__END_DECLS
#endif
