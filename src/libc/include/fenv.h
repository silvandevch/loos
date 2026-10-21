#ifndef _FENV_H
#define _FENV_H
#include "features.h"
__LOOS_BEGIN
#define FE_INVALID 0x01
#define FE_DIVBYZERO 0x04
#define FE_OVERFLOW 0x08
#define FE_UNDERFLOW 0x10
#define FE_INEXACT 0x20
#define FE_ALL_EXCEPT 0x3d
#define FE_TONEAREST 0x000
#define FE_DOWNWARD 0x400
#define FE_UPWARD 0x800
#define FE_TOWARDZERO 0xc00
typedef struct { unsigned short control, __r1, status, __r2, tag, __r3; unsigned env[4]; unsigned mxcsr; } fenv_t;
typedef unsigned short fexcept_t;
#define FE_DFL_ENV ((const fenv_t *)-1)
int feclearexcept(int);
int fetestexcept(int);
int feraiseexcept(int);
int fegetround(void);
int fesetround(int);
int fegetenv(fenv_t *);
int fesetenv(const fenv_t *);
__LOOS_END
#endif
