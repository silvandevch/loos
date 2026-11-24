#ifndef _SETJMP_H
#define _SETJMP_H
#include <_prism_cdefs.h>
#if defined(__x86_64__)
typedef int jmp_buf[37]; typedef int sigjmp_buf[37 + 1];
#else
typedef int jmp_buf[48]; typedef int sigjmp_buf[48 + 1];
#endif
__BEGIN_DECLS
int setjmp(jmp_buf); void longjmp(jmp_buf, int) __attribute__((__noreturn__));
int _setjmp(jmp_buf); void _longjmp(jmp_buf, int) __attribute__((__noreturn__));
int sigsetjmp(sigjmp_buf, int); void siglongjmp(sigjmp_buf, int) __attribute__((__noreturn__));
__END_DECLS
#endif
