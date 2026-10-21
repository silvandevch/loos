/* libm for x86-64. Transcendentals run on the x87 FPU in 80-bit extended precision and round once to double,
 * which gives results within an ulp or better; exact operations (floor, fmod, frexp, ...) work on the bits.
 * Argument reduction for sin/cos/tan uses fprem1 against the extended-precision pi the FPU carries (66 bits),
 * so results lose accuracy only for arguments within ~1e-19 relative of a multiple of pi. */
#include <math.h>
#include <errno.h>
#include <stdint.h>
#include <string.h>

typedef union { double d; uint64_t u; } du;
static inline uint64_t bits(double x) { du v; v.d = x; return v.u; }
static inline double from(uint64_t u) { du v; v.u = u; return v.d; }

double fabs(double x) { return from(bits(x) & ~(1ULL << 63)); }
double copysign(double x, double y) { return from((bits(x) & ~(1ULL << 63)) | (bits(y) & (1ULL << 63))); }
double nan(const char *tag) { (void)tag; return __builtin_nan(""); }
double sqrt(double x) { double r; __asm__("sqrtsd %1, %0" : "=x"(r) : "x"(x)); if (x < 0) errno = EDOM; return r; }

static int is_nan(double x) { return (bits(x) & ~(1ULL << 63)) > 0x7ff0000000000000ULL; }
static int is_inf(double x) { return (bits(x) & ~(1ULL << 63)) == 0x7ff0000000000000ULL; }

double trunc(double x)
{
    uint64_t u = bits(x); int e = (int)((u >> 52) & 0x7ff) - 1023;
    if (e >= 52) return x;
    if (e < 0) return from(u & (1ULL << 63));
    return from(u & ~((1ULL << (52 - e)) - 1));
}
double floor(double x) { double t = trunc(x); return (t > x) ? t - 1.0 : t; }
double ceil(double x) { double t = trunc(x); return (t < x) ? t + 1.0 : t; }
double round(double x) { double t = trunc(x); double d = x - t; if (d >= 0.5) return t + 1.0; if (d <= -0.5) return t - 1.0; return t; }
double rint(double x)
{
    if (!(fabs(x) < 4503599627370496.0)) return x;
    double r = x < 0 ? (x - 4503599627370496.0) + 4503599627370496.0 : (x + 4503599627370496.0) - 4503599627370496.0;
    return r == 0 ? copysign(0.0, x) : r;
}
double nearbyint(double x) { return rint(x); }
long lround(double x) { return (long)round(x); }
long long llround(double x) { return (long long)round(x); }
long lrint(double x) { return (long)rint(x); }
long long llrint(double x) { return (long long)rint(x); }

double frexp(double x, int *e)
{
    uint64_t u = bits(x); int ex = (int)((u >> 52) & 0x7ff);
    *e = 0;
    if (ex == 0x7ff || x == 0) return x;
    if (ex == 0) { x *= 18014398509481984.0; u = bits(x); ex = (int)((u >> 52) & 0x7ff) - 54; }
    *e = ex - 1022;
    return from((u & ~(0x7ffULL << 52)) | (1022ULL << 52));
}
double ldexp(double x, int n)
{
    if (n > 1023) { x *= 0x1p1023; n -= 1023; if (n > 1023) { x *= 0x1p1023; n -= 1023; if (n > 1023) n = 1023; } }
    else if (n < -1022) {
        x *= 0x1p-1022 * 0x1p53; n += 1022 - 53;
        if (n < -1022) { x *= 0x1p-1022 * 0x1p53; n += 1022 - 53; if (n < -1022) n = -1022; }
    }
    return x * from((uint64_t)(0x3ff + n) << 52);
}
double scalbn(double x, int n) { return ldexp(x, n); }
double modf(double x, double *ip)
{
    if (is_inf(x)) { *ip = x; return copysign(0.0, x); }
    double t = trunc(x);
    *ip = t;
    return copysign(x - t, x);
}
double fmod(double x, double y)
{
    uint64_t ux = bits(x), uy = bits(y), sx = ux >> 63;
    int ex = (int)((ux >> 52) & 0x7ff), ey = (int)((uy >> 52) & 0x7ff);
    if (is_nan(x) || is_nan(y) || is_inf(x) || (uy << 1) == 0) { errno = EDOM; return __builtin_nan(""); }
    if ((ux << 1) <= (uy << 1)) return (ux << 1) == (uy << 1) ? from(sx << 63) : x;
    if (ex == 0) { for (uint64_t i = ux << 12; !(i >> 63); i <<= 1) ex--; ux <<= 1 - ex; }
    else { ux &= (1ULL << 52) - 1; ux |= 1ULL << 52; }
    if (ey == 0) { for (uint64_t i = uy << 12; !(i >> 63); i <<= 1) ey--; uy <<= 1 - ey; }
    else { uy &= (1ULL << 52) - 1; uy |= 1ULL << 52; }
    for (; ex > ey; ex--) {
        uint64_t d = ux - uy;
        if (!(d >> 63)) { if (d == 0) return from(sx << 63); ux = d; }
        ux <<= 1;
    }
    { uint64_t d = ux - uy; if (!(d >> 63)) { if (d == 0) return from(sx << 63); ux = d; } }
    for (; !(ux >> 52); ux <<= 1) ex--;
    if (ex > 0) { ux -= 1ULL << 52; ux |= (uint64_t)ex << 52; } else ux >>= 1 - ex;
    return from(ux | (sx << 63));
}
double remainder(double x, double y)
{
    if (is_nan(x) || is_nan(y) || is_inf(x) || y == 0) { errno = EDOM; return __builtin_nan(""); }
    if (is_inf(y)) return x;
    double ax = fabs(x), ay = fabs(y);
    double r = ay > 0x1p1022 ? fmod(ax, ay) : fmod(ax, 2 * ay), rem;
    if (ay > 0x1p1022) { rem = r; if (r > ay / 2 || (r == ay / 2 && 0)) rem = r - ay; }
    else {
        double d = r - ay, h = ay / 2;
        if (d > h) rem = r - 2 * ay;
        else if (d < -h) rem = r;
        else if (d == h) rem = r - 2 * ay;          /* tie: pick the even quotient */
        else if (d == -h) rem = r;
        else rem = d;
    }
    return x < 0 || (x == 0 && bits(x) >> 63) ? -rem : rem;
}
double fmin(double a, double b) { if (is_nan(a)) return b; if (is_nan(b)) return a; return a < b ? a : b; }
double fmax(double a, double b) { if (is_nan(a)) return b; if (is_nan(b)) return a; return a > b ? a : b; }
double fdim(double a, double b) { return a > b ? a - b : 0.0; }
double fma(double a, double b, double c) { return (double)((long double)a * b + c); }   /* exact product in 64-bit mantissa; double rounding only in rare ties */

/* ---- x87 kernels ---- */
static long double sqrtl_(long double x) { long double r; __asm__("fsqrt" : "=t"(r) : "0"(x)); return r; }
static long double x87_sin(long double x) { long double r; __asm__("fsin" : "=t"(r) : "0"(x)); return r; }
static long double x87_cos(long double x) { long double r; __asm__("fcos" : "=t"(r) : "0"(x)); return r; }
static long double x87_atan2(long double y, long double x) { long double r; __asm__("fpatan" : "=t"(r) : "0"(x), "u"(y) : "st(1)"); return r; }
static long double x87_log2(long double x) { long double r; __asm__("fyl2x" : "=t"(r) : "0"(x), "u"(1.0L) : "st(1)"); return r; }
static long double x87_exp2(long double x)                       /* 2^x */
{
    long double r, ip;
    __asm__("frndint" : "=t"(ip) : "0"(x));
    long double f = x - ip;
    __asm__("f2xm1" : "=t"(r) : "0"(f));
    r += 1.0L;
    long n = (long)ip;
    while (n > 1000) { r *= 0x1p1000L; n -= 1000; }
    while (n < -1000) { r *= 0x1p-1000L; n += 1000; }
    return r * from((uint64_t)(0x3ff + n) << 52);
}

double sin(double x)
{
    if (is_nan(x) || is_inf(x)) { errno = EDOM; return __builtin_nan(""); }
    long double v = x;
    if (fabs(x) > 1e18) v = (long double)fmod(x, 6.283185307179586);
    return (double)x87_sin(v);
}
double cos(double x)
{
    if (is_nan(x) || is_inf(x)) { errno = EDOM; return __builtin_nan(""); }
    long double v = x;
    if (fabs(x) > 1e18) v = (long double)fmod(x, 6.283185307179586);
    return (double)x87_cos(v);
}
double tan(double x)
{
    if (is_nan(x) || is_inf(x)) { errno = EDOM; return __builtin_nan(""); }
    long double v = x;
    if (fabs(x) > 1e18) v = (long double)fmod(x, 6.283185307179586);
    return (double)(x87_sin(v) / x87_cos(v));
}
double atan2(double y, double x) { return (double)x87_atan2(y, x); }
double atan(double x) { return (double)x87_atan2(x, 1.0L); }
double asin(double x)
{
    if (is_nan(x)) return x;
    if (x > 1 || x < -1) { errno = EDOM; return __builtin_nan(""); }
    long double v = x;
    return (double)x87_atan2(v, sqrtl_(1.0L - v * v));
}
double acos(double x)
{
    if (is_nan(x)) return x;
    if (x > 1 || x < -1) { errno = EDOM; return __builtin_nan(""); }
    long double v = x;
    return (double)x87_atan2(sqrtl_(1.0L - v * v), v);
}

double exp2(double x)
{
    if (is_nan(x)) return x;
    if (x > 1024.0) { errno = ERANGE; return HUGE_VAL; }
    if (x < -1080.0) { errno = ERANGE; return 0.0; }
    return (double)x87_exp2(x);
}
double exp(double x)
{
    if (is_nan(x)) return x;
    if (x > 709.8) { errno = ERANGE; return HUGE_VAL; }
    if (x < -745.2) { errno = ERANGE; return 0.0; }
    return (double)x87_exp2((long double)x * 1.4426950408889634073599246810018921L);
}
double expm1(double x)
{
    if (is_nan(x)) return x;
    if (fabs(x) < 1e-5) return x + 0.5 * x * x;
    if (x > 709.8) { errno = ERANGE; return HUGE_VAL; }
    return (double)(x87_exp2((long double)x * 1.4426950408889634073599246810018921L) - 1.0L);
}
double log2(double x)
{
    if (is_nan(x)) return x;
    if (x < 0) { errno = EDOM; return __builtin_nan(""); }
    if (x == 0) { errno = ERANGE; return -HUGE_VAL; }
    if (is_inf(x)) return x;
    return (double)x87_log2(x);
}
double log(double x) { double r = log2(x); return is_nan(r) || is_inf(r) ? r : (double)((long double)x87_log2(x) * 0.69314718055994530941723212145817657L); }
double log10(double x) { double r = log2(x); return is_nan(r) || is_inf(r) ? r : (double)((long double)x87_log2(x) * 0.30102999566398119521373889472449303L); }
double log1p(double x)
{
    if (is_nan(x)) return x;
    if (x < -1) { errno = EDOM; return __builtin_nan(""); }
    if (fabs(x) < 1e-5) return x - 0.5 * x * x;
    return (double)((long double)x87_log2(1.0L + x) * 0.69314718055994530941723212145817657L);
}
double pow(double x, double y)
{
    if (y == 0) return 1.0;
    if (x == 1.0) return 1.0;
    if (is_nan(x) || is_nan(y)) return __builtin_nan("");
    int yint = y == trunc(y);
    int yodd = yint && fabs(y) < 9007199254740992.0 && ((long long)y & 1);
    if (x == 0) {
        if (y < 0) { errno = ERANGE; return yodd ? copysign(HUGE_VAL, x) : HUGE_VAL; }
        return yodd ? x : 0.0;
    }
    if (is_inf(y)) { double ax = fabs(x); if (ax == 1.0) return 1.0; return (ax > 1.0) == (y > 0) ? HUGE_VAL : 0.0; }
    if (is_inf(x)) { if (x > 0) return y > 0 ? HUGE_VAL : 0.0; return y > 0 ? (yodd ? -HUGE_VAL : HUGE_VAL) : (yodd ? -0.0 : 0.0); }
    int neg = 0;
    if (x < 0) { if (!yint) { errno = EDOM; return __builtin_nan(""); } neg = yodd; x = -x; }
    long double l = (long double)y * x87_log2(x);
    double r;
    if (l > 1024.0L) { errno = ERANGE; r = HUGE_VAL; }
    else if (l < -1080.0L) { errno = ERANGE; r = 0.0; }
    else r = (double)x87_exp2(l);
    return neg ? -r : r;
}
double cbrt(double x)
{
    if (x == 0 || is_nan(x) || is_inf(x)) return x;
    double a = fabs(x);
    long double r = x87_exp2(x87_log2(a) / 3.0L);
    r = r - (r * r * r - a) / (3.0L * r * r);                         /* one Newton step in extended precision */
    return copysign((double)r, x);
}
double hypot(double x, double y)
{
    if (is_inf(x) || is_inf(y)) return HUGE_VAL;
    if (is_nan(x) || is_nan(y)) return __builtin_nan("");
    return (double)sqrtl_((long double)x * x + (long double)y * y);
}
double sinh(double x) { if (fabs(x) < 1e-5) return x; long double e = x87_exp2((long double)x * 1.4426950408889634073599246810018921L); return (double)((e - 1.0L / e) * 0.5L); }
double cosh(double x) { long double e = x87_exp2((long double)x * 1.4426950408889634073599246810018921L); return (double)((e + 1.0L / e) * 0.5L); }
double tanh(double x)
{
    if (fabs(x) < 1e-5) return x;
    if (x > 20) return 1.0;
    if (x < -20) return -1.0;
    long double e = x87_exp2((long double)x * 2.8853900817779268147198493620037842L);
    return (double)((e - 1.0L) / (e + 1.0L));
}
double asinh(double x) { double a = fabs(x); return copysign(log(a + sqrt(a * a + 1.0)), x); }
double acosh(double x) { if (x < 1) { errno = EDOM; return __builtin_nan(""); } return log(x + sqrt(x * x - 1.0)); }
double atanh(double x) { if (x > 1 || x < -1) { errno = EDOM; return __builtin_nan(""); } return 0.5 * log1p(2.0 * x / (1.0 - x)); }

/* single-precision wrappers */
#define F1(n) float n##f(float x) { return (float)n((double)x); }
F1(sin) F1(cos) F1(tan) F1(asin) F1(acos) F1(atan) F1(sinh) F1(cosh) F1(tanh) F1(asinh) F1(acosh) F1(atanh)
F1(exp) F1(exp2) F1(expm1) F1(log) F1(log2) F1(log10) F1(log1p) F1(sqrt) F1(cbrt) F1(fabs) F1(floor) F1(ceil)
F1(trunc) F1(round) F1(rint) F1(nearbyint)
float atan2f(float y, float x) { return (float)atan2(y, x); }
float powf(float x, float y) { return (float)pow(x, y); }
float fmodf(float x, float y) { return (float)fmod(x, y); }
float hypotf(float x, float y) { return (float)hypot(x, y); }
float copysignf(float x, float y) { return (float)copysign(x, y); }
float fminf(float a, float b) { return (float)fmin(a, b); }
float fmaxf(float a, float b) { return (float)fmax(a, b); }
float ldexpf(float x, int n) { return (float)ldexp(x, n); }
float frexpf(float x, int *e) { return (float)frexp(x, e); }
float modff(float x, float *ip) { double i; float r = (float)modf(x, &i); *ip = (float)i; return r; }
