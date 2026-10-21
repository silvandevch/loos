#ifndef _MATH_H
#define _MATH_H
#include "features.h"
__LOOS_BEGIN
typedef double double_t;
typedef float float_t;
#define M_E 2.7182818284590452354
#define M_LOG2E 1.4426950408889634074
#define M_LOG10E 0.43429448190325182765
#define M_LN2 0.69314718055994530942
#define M_LN10 2.30258509299404568402
#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.78539816339744830962
#define M_1_PI 0.31830988618379067154
#define M_2_PI 0.63661977236758134308
#define M_2_SQRTPI 1.12837916709551257390
#define M_SQRT2 1.41421356237309504880
#define M_SQRT1_2 0.70710678118654752440
#define HUGE_VAL (__builtin_huge_val())
#define HUGE_VALF (__builtin_huge_valf())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))
#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4
#define fpclassify(x) __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, (x))
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)
#define isnormal(x) __builtin_isnormal(x)
#define signbit(x) __builtin_signbit(x)
#define isgreater(a, b) __builtin_isgreater(a, b)
#define isless(a, b) __builtin_isless(a, b)
#define isunordered(a, b) __builtin_isunordered(a, b)
#define MATH1(n) double n(double); float n##f(float);
MATH1(sin) MATH1(cos) MATH1(tan) MATH1(asin) MATH1(acos) MATH1(atan)
MATH1(sinh) MATH1(cosh) MATH1(tanh) MATH1(asinh) MATH1(acosh) MATH1(atanh)
MATH1(exp) MATH1(exp2) MATH1(expm1) MATH1(log) MATH1(log2) MATH1(log10) MATH1(log1p)
MATH1(sqrt) MATH1(cbrt) MATH1(fabs) MATH1(floor) MATH1(ceil) MATH1(trunc) MATH1(round) MATH1(rint) MATH1(nearbyint)
double atan2(double, double); float atan2f(float, float);
double pow(double, double); float powf(float, float);
double fmod(double, double); float fmodf(float, float);
double hypot(double, double); float hypotf(float, float);
double copysign(double, double); float copysignf(float, float);
double fmin(double, double); double fmax(double, double); double fdim(double, double);
float fminf(float, float); float fmaxf(float, float);
double ldexp(double, int); float ldexpf(float, int);
double frexp(double, int *); float frexpf(float, int *);
double modf(double, double *); float modff(float, float *);
double scalbn(double, int);
long lround(double); long long llround(double); long lrint(double); long long llrint(double);
double remainder(double, double);
double fma(double, double, double);
double nan(const char *);
__LOOS_END
#endif
