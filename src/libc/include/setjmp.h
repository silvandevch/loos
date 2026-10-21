#ifndef _SETJMP_H
#define _SETJMP_H
#include "features.h"
__LOOS_BEGIN
typedef unsigned long jmp_buf[8];
typedef unsigned long sigjmp_buf[8];
int setjmp(jmp_buf) __attribute__((returns_twice));
int _setjmp(jmp_buf) __attribute__((returns_twice));
int sigsetjmp(sigjmp_buf, int) __attribute__((returns_twice));
__LOOS_NORETURN void longjmp(jmp_buf, int);
__LOOS_NORETURN void _longjmp(jmp_buf, int);
__LOOS_NORETURN void siglongjmp(sigjmp_buf, int);
#define setjmp(e) setjmp(e)
__LOOS_END
#endif
