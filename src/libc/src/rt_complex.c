/* Compiler-runtime helpers for complex multiply/divide (C99 Annex G recovery of infinities and NaNs). */
#include <math.h>

static int isnan_(double x) { return x != x; }

double _Complex __muldc3(double a, double b, double c, double d)
{
    double ac = a * c, bd = b * d, ad = a * d, bc = b * c, x = ac - bd, y = ad + bc;
    if (isnan_(x) && isnan_(y)) {
        int recalc = 0;
        if (isinf(a) || isinf(b)) { a = copysign(isinf(a) ? 1 : 0, a); b = copysign(isinf(b) ? 1 : 0, b); if (isnan_(c)) c = copysign(0, c); if (isnan_(d)) d = copysign(0, d); recalc = 1; }
        if (isinf(c) || isinf(d)) { c = copysign(isinf(c) ? 1 : 0, c); d = copysign(isinf(d) ? 1 : 0, d); if (isnan_(a)) a = copysign(0, a); if (isnan_(b)) b = copysign(0, b); recalc = 1; }
        if (!recalc && (isinf(ac) || isinf(bd) || isinf(ad) || isinf(bc))) {
            if (isnan_(a)) a = copysign(0, a); if (isnan_(b)) b = copysign(0, b); if (isnan_(c)) c = copysign(0, c); if (isnan_(d)) d = copysign(0, d);
            recalc = 1;
        }
        if (recalc) { x = INFINITY * (a * c - b * d); y = INFINITY * (a * d + b * c); }
    }
    double _Complex r; __real__ r = x; __imag__ r = y; return r;
}

double _Complex __divdc3(double a, double b, double c, double d)
{
    int ilogb_ = 0; double m = fmax(fabs(c), fabs(d));
    if (m != 0 && isfinite(m)) { frexp(m, &ilogb_); ilogb_--; c = ldexp(c, -ilogb_); d = ldexp(d, -ilogb_); } else ilogb_ = 0;
    double den = c * c + d * d, x = ldexp((a * c + b * d) / den, -ilogb_), y = ldexp((b * c - a * d) / den, -ilogb_);
    if (isnan_(x) && isnan_(y)) {
        if (den == 0 && (!isnan_(a) || !isnan_(b))) { x = copysign(INFINITY, c) * a; y = copysign(INFINITY, c) * b; }
        else if ((isinf(a) || isinf(b)) && isfinite(c) && isfinite(d)) { a = copysign(isinf(a) ? 1 : 0, a); b = copysign(isinf(b) ? 1 : 0, b); x = INFINITY * (a * c + b * d); y = INFINITY * (b * c - a * d); }
        else if (isinf(m) && isfinite(a) && isfinite(b)) { c = copysign(isinf(c) ? 1 : 0, c); d = copysign(isinf(d) ? 1 : 0, d); x = 0 * (a * c + b * d); y = 0 * (b * c - a * d); }
    }
    double _Complex r; __real__ r = x; __imag__ r = y; return r;
}

float _Complex __mulsc3(float a, float b, float c, float d)
{
    double _Complex r = __muldc3(a, b, c, d);
    float _Complex f; __real__ f = (float)__real__ r; __imag__ f = (float)__imag__ r; return f;
}
float _Complex __divsc3(float a, float b, float c, float d)
{
    double _Complex r = __divdc3(a, b, c, d);
    float _Complex f; __real__ f = (float)__real__ r; __imag__ f = (float)__imag__ r; return f;
}
