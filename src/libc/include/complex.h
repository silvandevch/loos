#ifndef _COMPLEX_H
#define _COMPLEX_H
#include "features.h"
#define complex _Complex
#define _Complex_I (__extension__ 1.0iF)
#define I _Complex_I
__LOOS_BEGIN
double cabs(double complex); double carg(double complex); double creal(double complex); double cimag(double complex);
double complex conj(double complex); double complex cexp(double complex); double complex clog(double complex);
double complex csqrt(double complex); double complex cpow(double complex, double complex);
double complex csin(double complex); double complex ccos(double complex); double complex cproj(double complex);
float cabsf(float complex); float crealf(float complex); float cimagf(float complex);
__LOOS_END
#endif
