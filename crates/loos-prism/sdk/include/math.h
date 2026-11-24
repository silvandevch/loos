#ifndef _MATH_H
#define _MATH_H
#include <_prism_cdefs.h>
typedef float float_t;
typedef double double_t;
#define HUGE_VAL __builtin_huge_val()
#define HUGE_VALF __builtin_huge_valf()
#define HUGE_VALL __builtin_huge_vall()
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define FP_NAN 1
#define FP_INFINITE 2
#define FP_ZERO 3
#define FP_NORMAL 4
#define FP_SUBNORMAL 5
#define FP_ILOGB0 (-2147483647 - 1)
#define FP_ILOGBNAN (-2147483647 - 1)
#define MATH_ERRNO 1
#define MATH_ERREXCEPT 2
#define math_errhandling MATH_ERREXCEPT
#define fpclassify(x) __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, x)
#define isfinite(x) __builtin_isfinite(x)
#define isinf(x) __builtin_isinf(x)
#define isnan(x) __builtin_isnan(x)
#define isnormal(x) __builtin_isnormal(x)
#define signbit(x) __builtin_signbit(x)
#define isgreater(x, y) __builtin_isgreater(x, y)
#define isgreaterequal(x, y) __builtin_isgreaterequal(x, y)
#define isless(x, y) __builtin_isless(x, y)
#define islessequal(x, y) __builtin_islessequal(x, y)
#define islessgreater(x, y) __builtin_islessgreater(x, y)
#define isunordered(x, y) __builtin_isunordered(x, y)
#define M_PI 3.14159265358979323846264338327950288
#define M_E 2.71828182845904523536028747135266250
#define M_LN2 0.693147180559945309417232121458176568
#define M_LN10 2.30258509299404568401799145468436421
#define M_SQRT2 1.41421356237309504880168872420969808
__BEGIN_DECLS
#define _P_F1(n) double n(double); float n##f(float); long double n##l(long double);
#define _P_F2(n) double n(double, double); float n##f(float, float); long double n##l(long double, long double);
_P_F1(acos) _P_F1(asin) _P_F1(atan) _P_F2(atan2) _P_F1(cos) _P_F1(sin) _P_F1(tan) _P_F1(acosh) _P_F1(asinh)
_P_F1(atanh) _P_F1(cosh) _P_F1(sinh) _P_F1(tanh) _P_F1(exp) _P_F1(exp2) _P_F1(expm1) _P_F1(log) _P_F1(log10)
_P_F1(log2) _P_F1(log1p) _P_F1(logb) _P_F1(cbrt) _P_F1(fabs) _P_F2(hypot) _P_F2(pow) _P_F1(sqrt) _P_F1(erf)
_P_F1(erfc) _P_F1(lgamma) _P_F1(tgamma) _P_F1(ceil) _P_F1(floor) _P_F1(nearbyint) _P_F1(rint) _P_F1(round)
_P_F1(trunc) _P_F2(fmod) _P_F2(remainder) _P_F2(copysign) _P_F2(nextafter) _P_F2(fdim) _P_F2(fmax) _P_F2(fmin)
#undef _P_F1
#undef _P_F2
double frexp(double, int *); float frexpf(float, int *); long double frexpl(long double, int *);
double ldexp(double, int); float ldexpf(float, int); long double ldexpl(long double, int);
double modf(double, double *); float modff(float, float *); long double modfl(long double, long double *);
double scalbn(double, int); float scalbnf(float, int); long double scalbnl(long double, int);
double scalbln(double, long); float scalblnf(float, long); long double scalblnl(long double, long);
int ilogb(double); int ilogbf(float); int ilogbl(long double);
long lrint(double); long lrintf(float); long lrintl(long double);
long long llrint(double); long long llrintf(float); long long llrintl(long double);
long lround(double); long lroundf(float); long lroundl(long double);
long long llround(double); long long llroundf(float); long long llroundl(long double);
double remquo(double, double, int *); float remquof(float, float, int *); long double remquol(long double, long double, int *);
double nan(const char *); float nanf(const char *); long double nanl(const char *);
double nexttoward(double, long double); float nexttowardf(float, long double); long double nexttowardl(long double, long double);
double fma(double, double, double); float fmaf(float, float, float); long double fmal(long double, long double, long double);
__END_DECLS
#endif
