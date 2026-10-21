/* printf family: differential tests against the host C library for everything that is portable
 * (integers, strings, floating point down to the last digit), plus msvcrt-specific behaviour. */
#define LOOS_HOST_TEST 1
#include "../../../src/win32/include/loos_nt.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

#define DIFF(fmt, ...) do { char a_[4096], b_[4096]; int ra_ = loos_sprintf(a_, fmt, __VA_ARGS__), rb_ = snprintf(b_, sizeof b_, fmt, __VA_ARGS__); \
    if (ra_ != rb_ || strcmp(a_, b_)) { printf("FAIL %s:%d: %s -> \"%s\" (%d), want \"%s\" (%d)\n", __FILE__, __LINE__, fmt, a_, ra_, b_, rb_); fails++; } } while (0)

static void wide(WCHAR *d, const char *s) { while ((*d++ = (WCHAR)(unsigned char)*s++)) { } }
static void narrow(char *d, const WCHAR *s) { while ((*d++ = (char)*s++)) { } }

static uint64_t rng = 0x9E3779B97F4A7C15ull;
static uint64_t next(void) { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; return rng; }

static void test_ints(void)
{
    DIFF("%d|%i|%u|%x|%X|%o", -42, 42, 4000000000u, 0xdeadbeefu, 0xdeadbeefu, 0777u);
    DIFF("[%5d][%-5d][%05d][%+d][% d][%.3d][%8.3d][%-8.3d]", 42, 42, 42, 42, 42, 7, 7, 7);
    DIFF("[%#x][%#X][%#o][%#x][%.0d][%.0d]", 255u, 255u, 8u, 0u, 0, 1);
    DIFF("[%*d][%-*d][%.*d]", 6, 12, 6, 12, 4, 12);
    DIFF("[%*d]", -6, 12);
    DIFF("%lld %llu %llx %lli", -9223372036854775807ll - 1, 18446744073709551615ull, 0xfedcba9876543210ull, 5ll);
    DIFF("%hd %hu %hhd %hhu", 70000, 70000, 300, 300);
    DIFF("%c%c%c|%5c|%-5c|", 'a', 'b', 'c', 'x', 'y');
    DIFF("100%% %s", "done");
    DIFF("%d%%", 5);
    { char b[64]; loos_sprintf(b, "%I64d %I32d %Ix %zu", -5ll, -6, (uintptr_t)255, (size_t)7); CHECK(!strcmp(b, "-5 -6 ff 7")); }
    { char b[64]; loos_sprintf(b, "%ld %lu %lx", -1, 4294967295u, 0xabcdu); CHECK(!strcmp(b, "-1 4294967295 abcd")); }   /* long is 32-bit */
    { char b[64]; loos_sprintf(b, "%p|%p", (void *)0, (void *)0x7ff612345678ull); CHECK(!strcmp(b, "0000000000000000|00007FF612345678")); }
}

static void test_strings(void)
{
    DIFF("[%s][%10s][%-10s][%.3s][%10.3s][%.0s]", "hello", "hello", "hello", "hello", "hello", "hello");
    DIFF("[%*s][%.*s]", 8, "ab", 2, "abcdef");
    { char b[64]; loos_sprintf(b, "%s", (char *)0); CHECK(!strcmp(b, "(null)")); }
    { char b[64]; WCHAR w[16]; wide(w, "wide"); loos_sprintf(b, "%ws|%ls|%S", w, w, w); CHECK(!strcmp(b, "wide|wide|wide")); }
    { char b[64]; WCHAR w[2] = { 0x4E2D, 0 }; loos_sprintf(b, "%ls", w); CHECK(!strcmp(b, "?")); }
}

static void test_floats(void)
{
    static const char *const fmts[] = { "%f", "%.0f", "%.1f", "%.3f", "%.10f", "%e", "%.0e", "%.2e", "%E", "%g", "%.0g", "%.3g", "%.10g", "%G", "%#g", "%#.0f", "%#.0e",
                                         "%+.2f", "% .2f", "%12.3f", "%-12.3f|", "%012.3f", "%+012.3e", "%.20g", "%.17g", "%.15e", "%.30f" };
    static const double fixed[] = { 0, -0.0, 1, -1, 0.5, 1.5, 2.5, 0.125, 0.1, 0.2, 0.3, 1e-5, 1e-4, 123456789.0, 1e15, 1e16, 1e21, 1e22, 1e100, 1.7976931348623157e308,
                                     4.9406564584124654e-324, 2.2250738585072014e-308, 9.9999995, 99999.95, 0.99999995, 5e-324, 123.456, 1e-10, 0.000123456, 999999.5, 9.5, 0.05 };
    size_t i, j;
    for (i = 0; i < sizeof fmts / sizeof fmts[0]; i++) {
        for (j = 0; j < sizeof fixed / sizeof fixed[0]; j++) {
            char a[4096], b[4096]; int ra, rb;
            if (!strcmp(fmts[i], "%#g") && fixed[j] == 999999.5) continue;   /* glibc prints 1.e+06 here, contrary to C11 7.21.6.1 */
            ra = loos_sprintf(a, fmts[i], fixed[j]); rb = snprintf(b, sizeof b, fmts[i], fixed[j]);
            if (ra != rb || strcmp(a, b)) { printf("FAIL float %s of %.17g -> \"%s\", want \"%s\"\n", fmts[i], fixed[j], a, b); fails++; }
        }
        for (j = 0; j < 400; j++) {                               /* random bit patterns span every exponent */
            uint64_t bits = next(); double v; char a[4096], b[4096]; int ra, rb;
            if (((bits >> 52) & 0x7ff) == 0x7ff) continue;
            memcpy(&v, &bits, 8);
            if ((i > 22 && i < 24) && (v > 1e30 || v < -1e30)) continue;
            ra = loos_sprintf(a, fmts[i], v); rb = snprintf(b, sizeof b, fmts[i], v);
            if (ra != rb || strcmp(a, b)) { printf("FAIL float %s of %.17g -> \"%s\", want \"%s\"\n", fmts[i], v, a, b); fails++; break; }
        }
    }
    { char a[32]; double inf = __builtin_inf(), nan = __builtin_nan("");
      loos_sprintf(a, "%f|%E|%5f|%-6f|%05f", inf, -inf, inf, inf, inf); CHECK(!strcmp(a, "inf|-INF|  inf|inf   |  inf"));
      loos_sprintf(a, "%g", nan); CHECK(!strcmp(a, "nan")); }
}

static void test_legacy_contract(void)
{
    char b[16];
    int r;
    memset(b, 'Z', sizeof b);
    r = loos__snprintf(b, 5, "%s", "abcd");  CHECK(r == 4 && !strcmp(b, "abcd"));
    memset(b, 'Z', sizeof b);
    r = loos__snprintf(b, 5, "%s", "abcde"); CHECK(r == 5 && !memcmp(b, "abcde", 5) && b[5] == 'Z');   /* exact fit: no NUL */
    memset(b, 'Z', sizeof b);
    r = loos__snprintf(b, 5, "%s", "abcdef"); CHECK(r == -1 && !memcmp(b, "abcde", 5) && b[5] == 'Z');
    r = loos__snprintf(NULL, 0, "%d", 12345); CHECK(r == 5);
    r = loos_sprintf_s(b, 3, "%s", "abc"); CHECK(r == -1 && b[0] == 0);                            /* no room for the NUL */
    r = loos_sprintf_s(b, 5, "%s", "abc"); CHECK(r == 3 && !strcmp(b, "abc"));
    r = loos__snprintf_s(b, 4, (size_t)-1, "%s", "abcdef"); CHECK(r == -1 && !strcmp(b, "abc"));  /* _TRUNCATE */
    r = loos__snprintf_s(b, 8, 2, "%s", "abcdef"); CHECK(r == -1 && !strcmp(b, "ab"));
    r = loos_sprintf_s(NULL, 4, "x"); CHECK(r == -1);
    r = loos_sprintf(b, "%s", ""); CHECK(r == 0 && b[0] == 0);
}

static void test_wide(void)
{
    WCHAR out[64], f[64], s[16]; char n[64]; int r;
    wide(f, "%s|%S|%hs|%c|%C|%d|%5.1f"); wide(s, "wide");
    r = loos__snwprintf(out, 64, f, s, "narrow", "narrow2", (unsigned)'w', (unsigned)'n', -7, 3.14159);
    narrow(n, out);
    CHECK(r == (int)strlen("wide|narrow|narrow2|w|n|-7|  3.1") && !strcmp(n, "wide|narrow|narrow2|w|n|-7|  3.1"));
    wide(f, "%d"); r = loos__snwprintf(out, 3, f, 12345); CHECK(r == -1 && out[2] == '3');
    r = loos_swprintf(out, f, 99); CHECK(r == 2 && out[0] == '9' && out[2] == 0);
    wide(f, "%s"); r = loos_swprintf_s(out, 3, f, s); CHECK(r == -1 && out[0] == 0);
}

static int LOOS_CRT call_v(char *buf, size_t n, const char *fmt, ...)
{
    LOOS_VA_LIST ap; int r;
    LOOS_VA_START(ap, fmt); r = loos__vsnprintf(buf, n, fmt, ap); LOOS_VA_END(ap);
    return r;
}
static void test_va_list(void)
{
    char b[64]; int r = call_v(b, sizeof b, "%d-%s-%.2f", 5, "x", 2.5);
    CHECK(r == 8 && !strcmp(b, "5-x-2.50"));
}

int main(void)
{
    test_ints(); test_strings(); test_floats(); test_legacy_contract(); test_wide(); test_va_list();
    if (fails) printf("printf: %d FAILED\n", fails); else printf("printf: all passed\n");
    return fails != 0;
}
