#include <fenv.h>
/* x87 status word holds exception flags (and mxcsr mirrors them for SSE math); rounding mode must match in both. */
static unsigned mxcsr_get(void) { unsigned v; __asm__ volatile("stmxcsr %0" : "=m"(v)); return v; }
static void mxcsr_set(unsigned v) { __asm__ volatile("ldmxcsr %0" : : "m"(v)); }
int feclearexcept(int e)
{
    e &= FE_ALL_EXCEPT;
    unsigned char env[28];
    __asm__ volatile("fnstenv %0" : "=m"(env));
    env[4] &= (unsigned char)~e;
    __asm__ volatile("fldenv %0" : : "m"(env));
    mxcsr_set(mxcsr_get() & ~(unsigned)e);
    return 0;
}
int fetestexcept(int e)
{
    unsigned short sw;
    __asm__ volatile("fnstsw %0" : "=a"(sw));
    return (int)((sw | mxcsr_get()) & (unsigned)e & FE_ALL_EXCEPT);
}
int feraiseexcept(int e)
{
    mxcsr_set(mxcsr_get() | ((unsigned)e & FE_ALL_EXCEPT));
    return 0;
}
int fegetround(void) { return (int)(mxcsr_get() >> 3 & 0xc00); }
int fesetround(int r)
{
    if (r & ~0xc00) return 1;
    unsigned short cw;
    __asm__ volatile("fnstcw %0" : "=m"(cw));
    cw = (unsigned short)((cw & ~0xc00) | r);
    __asm__ volatile("fldcw %0" : : "m"(cw));
    mxcsr_set((mxcsr_get() & ~0x6000u) | ((unsigned)r << 3));
    return 0;
}
int fegetenv(fenv_t *e)
{
    __asm__ volatile("fnstenv %0" : "=m"(*(char (*)[28])e));
    e->mxcsr = mxcsr_get();
    return 0;
}
int fesetenv(const fenv_t *e)
{
    if (e == FE_DFL_ENV) { __asm__ volatile("fnclex; fldcw %0" : : "m"((unsigned short){0x37f})); mxcsr_set(0x1f80); return 0; }
    __asm__ volatile("fldenv %0" : : "m"(*(const char (*)[28])e));
    mxcsr_set(e->mxcsr);
    return 0;
}
