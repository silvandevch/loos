#ifndef _FENV_H
#define _FENV_H
#include <_prism_cdefs.h>
#if defined(__x86_64__)
typedef struct { unsigned short __control, __status; unsigned int __mxcsr; char __reserved[8]; } fenv_t;
typedef unsigned short fexcept_t;
#define FE_INEXACT 0x20
#define FE_UNDERFLOW 0x10
#define FE_OVERFLOW 0x08
#define FE_DIVBYZERO 0x04
#define FE_INVALID 0x01
#define FE_TONEAREST 0x0000
#define FE_DOWNWARD 0x0400
#define FE_UPWARD 0x0800
#define FE_TOWARDZERO 0x0c00
#else
typedef struct { unsigned long long __fpsr, __fpcr; } fenv_t;
typedef unsigned short fexcept_t;
#define FE_INEXACT 0x10
#define FE_UNDERFLOW 0x08
#define FE_OVERFLOW 0x04
#define FE_DIVBYZERO 0x02
#define FE_INVALID 0x01
#define FE_TONEAREST 0x00000000
#define FE_UPWARD 0x00400000
#define FE_DOWNWARD 0x00800000
#define FE_TOWARDZERO 0x00C00000
#endif
#define FE_ALL_EXCEPT (FE_INEXACT | FE_UNDERFLOW | FE_OVERFLOW | FE_DIVBYZERO | FE_INVALID)
__BEGIN_DECLS
extern const fenv_t _FE_DFL_ENV;
#define FE_DFL_ENV (&_FE_DFL_ENV)
int feclearexcept(int); int fegetexceptflag(fexcept_t *, int); int feraiseexcept(int);
int fesetexceptflag(const fexcept_t *, int); int fetestexcept(int); int fegetround(void); int fesetround(int);
int fegetenv(fenv_t *); int feholdexcept(fenv_t *); int fesetenv(const fenv_t *); int feupdateenv(const fenv_t *);
__END_DECLS
#endif
