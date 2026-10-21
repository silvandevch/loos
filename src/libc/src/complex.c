#include <complex.h>
#include <math.h>
double creal(double complex z) { return __real__ z; }
double cimag(double complex z) { return __imag__ z; }
float crealf(float complex z) { return __real__ z; }
float cimagf(float complex z) { return __imag__ z; }
double cabs(double complex z) { return hypot(creal(z), cimag(z)); }
float cabsf(float complex z) { return (float)hypot(crealf(z), cimagf(z)); }
double carg(double complex z) { return atan2(cimag(z), creal(z)); }
double complex conj(double complex z) { return creal(z) - cimag(z) * I; }
double complex cproj(double complex z) { if (isinf(creal(z)) || isinf(cimag(z))) return INFINITY + copysign(0.0, cimag(z)) * I; return z; }
double complex cexp(double complex z) { double e = exp(creal(z)); return e * cos(cimag(z)) + e * sin(cimag(z)) * I; }
double complex clog(double complex z) { return log(cabs(z)) + carg(z) * I; }
double complex csqrt(double complex z)
{
    double a = creal(z), b = cimag(z), m = cabs(z);
    if (m == 0) return 0.0 + b * I;
    double r = sqrt((m + fabs(a)) / 2);
    if (a >= 0) return r + (b / (2 * r)) * I;
    return fabs(b) / (2 * r) + copysign(r, b) * I;
}
double complex cpow(double complex z, double complex w)
{
    if (creal(z) == 0 && cimag(z) == 0) return creal(w) == 0 && cimag(w) == 0 ? 1.0 : 0.0;
    return cexp(w * clog(z));
}
double complex csin(double complex z) { double x = creal(z), y = cimag(z); return sin(x) * cosh(y) + cos(x) * sinh(y) * I; }
double complex ccos(double complex z) { double x = creal(z), y = cimag(z); return cos(x) * cosh(y) - sin(x) * sinh(y) * I; }
