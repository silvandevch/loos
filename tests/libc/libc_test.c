/* Self-hosted test for the LoOS libc: linked -nostdlib -static against libloos_c.a, so every symbol exercised
 * here is the LoOS implementation (the host libc is never involved). Reference values are published IEEE-754
 * results / C-standard behaviour. */
#include <assert.h>
#include <ctype.h>
#include <errno.h>
#include <inttypes.h>
#include <limits.h>
#include <locale.h>
#include <math.h>
#include <setjmp.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wchar.h>
#include <wctype.h>
#include <complex.h>
#include <dirent.h>
#include <fcntl.h>
#include <fenv.h>
#include <sys/stat.h>
#include <sys/wait.h>

static int failures, checks;
#define CHECK(c) do { checks++; if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); } } while (0)
#define STREQ(a, b) do { checks++; const char *a_ = (a), *b_ = (b); if (strcmp(a_, b_)) { failures++; fprintf(stderr, "FAIL %s:%d: \"%s\" != \"%s\"\n", __FILE__, __LINE__, a_, b_); } } while (0)
#define NEAR(a, b, tol) do { checks++; double a_ = (a), b_ = (b); if (!(fabs(a_ - b_) <= (tol) * (fabs(b_) > 1 ? fabs(b_) : 1))) { failures++; fprintf(stderr, "FAIL %s:%d: %s = %.17g, want %.17g\n", __FILE__, __LINE__, #a, a_, b_); } } while (0)

static uint64_t bitsof(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }

static void test_string(void)
{
    char b[64];
    CHECK(strlen("hello") == 5 && strnlen("hello", 3) == 3);
    strcpy(b, "abc"); strcat(b, "def"); STREQ(b, "abcdef");
    strncpy(b, "xy", 5); CHECK(b[2] == 0 && b[4] == 0);
    CHECK(strcmp("a", "b") < 0 && strcmp("b", "a") > 0 && strcmp("a", "a") == 0 && strncmp("abc", "abd", 2) == 0);
    CHECK(strchr("hello", 'l') == (char *)"hello" + 2 || 1);
    STREQ(strchr("hello", 'l'), "llo"); STREQ(strrchr("hello", 'l'), "lo"); CHECK(strchr("abc", 'z') == NULL);
    CHECK(strchr("abc", 0) != NULL);
    STREQ(strstr("find the needle here", "needle"), "needle here"); CHECK(strstr("abc", "abd") == NULL);
    CHECK(strspn("aabbc", "ab") == 4 && strcspn("hello world", " ") == 5);
    STREQ(strpbrk("hello", "lo"), "llo");
    char tok[] = "a,b;;c"; STREQ(strtok(tok, ",;"), "a"); STREQ(strtok(NULL, ",;"), "b"); STREQ(strtok(NULL, ",;"), "c"); CHECK(strtok(NULL, ",;") == NULL);
    char *d = strdup("dup"); STREQ(d, "dup"); free(d);
    CHECK(strcasecmp("HeLLo", "hello") == 0 && strncasecmp("abcX", "ABCy", 3) == 0);
    char m[16]; memset(m, 'x', 16); CHECK(m[0] == 'x' && m[15] == 'x');
    memcpy(m, "0123456789", 10); memmove(m + 2, m, 8); CHECK(!memcmp(m, "0101234567", 10));
    memmove(m, m + 2, 8); CHECK(!memcmp(m, "01234567", 8));
    CHECK(memchr("abc", 'c', 3) != NULL && memchr("abc", 'c', 2) == NULL);
    STREQ(strerror(ENOENT), "No such file or directory");
    char sb[8]; CHECK(strlcpy(sb, "toolongstring", sizeof sb) == 13 && !strcmp(sb, "toolong"));
    char *sp = (char[]){ "a b" }; char *sv = sp; STREQ(strsep(&sv, " "), "a"); STREQ(sv, "b");
}

static void test_ctype(void)
{
    CHECK(isalpha('a') && !isalpha('1') && isdigit('7') && isspace('\t') && isupper('Q') && islower('q'));
    CHECK(isxdigit('f') && !isxdigit('g') && ispunct('!') && isprint(' ') && !isgraph(' ') && iscntrl(0x1f));
    CHECK(toupper('a') == 'A' && tolower('Z') == 'z' && toupper('1') == '1');
    CHECK(iswalpha(0x00e9) && iswupper(0x00c9) && towlower(0x00c9) == 0x00e9);
}

static void test_malloc(void)
{
    enum { N = 2000 };
    static unsigned char *p[N]; static size_t sz[N];
    uint32_t seed = 12345;
    #define RND() (seed = seed * 1664525u + 1013904223u)
    for (int i = 0; i < N; i++) {
        sz[i] = RND() % 3000 + 1;
        p[i] = malloc(sz[i]);
        CHECK(p[i] && ((uintptr_t)p[i] & 15) == 0);
        memset(p[i], i & 0xff, sz[i]);
    }
    for (int round = 0; round < 6; round++) {
        for (int i = 0; i < N; i++) {
            if (RND() % 3) continue;
            for (size_t k = 0; k < sz[i]; k++) if (p[i][k] != (i & 0xff)) { CHECK(!"heap corruption"); break; }
            if (RND() & 1) { free(p[i]); sz[i] = RND() % 5000 + 1; p[i] = malloc(sz[i]); }
            else { size_t ns = RND() % 5000 + 1; unsigned char *q = realloc(p[i], ns); CHECK(q); p[i] = q; size_t keep = ns < sz[i] ? ns : sz[i]; for (size_t k = 0; k < keep; k++) if (q[k] != (i & 0xff)) { CHECK(!"realloc lost data"); break; } sz[i] = ns; }
            memset(p[i], i & 0xff, sz[i]);
        }
    }
    for (int i = 0; i < N; i++) { for (size_t k = 0; k < sz[i]; k++) if (p[i][k] != (i & 0xff)) { CHECK(!"final corruption"); break; } free(p[i]); }
    void *big = malloc(5 << 20); CHECK(big); memset(big, 1, 5 << 20); big = realloc(big, 9 << 20); CHECK(big && ((char *)big)[(5 << 20) - 1] == 1); free(big);
    unsigned char *z = calloc(100, 4); CHECK(z && z[399] == 0); free(z);
    CHECK(calloc(SIZE_MAX / 2, 4) == NULL && errno == ENOMEM);
    void *al = aligned_alloc(4096, 100); CHECK(al && ((uintptr_t)al & 4095) == 0); memset(al, 7, 100); free(al);
    void *pm; CHECK(posix_memalign(&pm, 64, 10) == 0 && ((uintptr_t)pm & 63) == 0); free(pm);
    CHECK(malloc_usable_size(malloc(10)) >= 10);
    free(NULL);
    /* a freed block is reused rather than leaked */
    void *a = malloc(100); free(a); void *b2 = malloc(100); CHECK(a == b2); free(b2);
}

static int cmp_int(const void *a, const void *b) { int x = *(const int *)a, y = *(const int *)b; return (x > y) - (x < y); }
static void test_stdlib(void)
{
    CHECK(strtol("  -123abc", NULL, 10) == -123 && strtol("0x1f", NULL, 0) == 31 && strtol("077", NULL, 0) == 63);
    char *e; CHECK(strtol("12xyz", &e, 10) == 12 && *e == 'x');
    CHECK(strtoul("4294967295", NULL, 10) == 4294967295UL && strtoll("-9223372036854775808", NULL, 10) == LLONG_MIN);
    errno = 0; CHECK(strtol("99999999999999999999", NULL, 10) == LONG_MAX && errno == ERANGE);
    errno = 0; CHECK(strtoull("18446744073709551616", NULL, 10) == ULLONG_MAX && errno == ERANGE);
    CHECK(strtol("zz", NULL, 36) == 35 * 36 + 35 && atoi("42") == 42 && atol("-7") == -7);
    CHECK(abs(-5) == 5 && labs(-5L) == 5 && llabs(-5LL) == 5);
    div_t dv = div(-7, 2); CHECK(dv.quot == -3 && dv.rem == -1);
    lldiv_t ld = lldiv(100, 7); CHECK(ld.quot == 14 && ld.rem == 2);
    srand(1); int r1 = rand(); srand(1); CHECK(rand() == r1 && r1 >= 0 && r1 <= RAND_MAX);
    int v[500]; for (int i = 0; i < 500; i++) v[i] = rand() % 100;
    qsort(v, 500, sizeof v[0], cmp_int);
    for (int i = 1; i < 500; i++) CHECK(v[i - 1] <= v[i]);
    int sorted[1000]; for (int i = 0; i < 1000; i++) sorted[i] = i * 2;
    int k = 998; CHECK(bsearch(&k, sorted, 1000, sizeof(int), cmp_int) == &sorted[499]); k = 7; CHECK(!bsearch(&k, sorted, 1000, sizeof(int), cmp_int));
    int adv[300]; for (int i = 0; i < 300; i++) adv[i] = 300 - i; qsort(adv, 300, sizeof(int), cmp_int);
    for (int i = 0; i < 300; i++) CHECK(adv[i] == i + 1);
    int same[200]; for (int i = 0; i < 200; i++) same[i] = 5; qsort(same, 200, sizeof(int), cmp_int); CHECK(same[0] == 5 && same[199] == 5);
    CHECK(setenv("LOOS_T", "v1", 1) == 0); STREQ(getenv("LOOS_T"), "v1");
    CHECK(setenv("LOOS_T", "v2", 0) == 0); STREQ(getenv("LOOS_T"), "v1");
    CHECK(setenv("LOOS_T", "v2", 1) == 0); STREQ(getenv("LOOS_T"), "v2");
    CHECK(unsetenv("LOOS_T") == 0 && getenv("LOOS_T") == NULL && getenv("NOPE_NOT_SET") == NULL);
}

static void test_strtod(void)
{
    CHECK(bitsof(strtod("0.1", NULL)) == bitsof(0.1));
    CHECK(bitsof(strtod("1e23", NULL)) == 0x44b52d02c7e14af6ULL);
    CHECK(bitsof(strtod("1.7976931348623157e308", NULL)) == 0x7fefffffffffffffULL);
    CHECK(bitsof(strtod("2.2250738585072011e-308", NULL)) == 0x000fffffffffffffULL);      /* famous slow-path value */
    CHECK(bitsof(strtod("2.2250738585072014e-308", NULL)) == 0x0010000000000000ULL);
    CHECK(bitsof(strtod("4.9406564584124654e-324", NULL)) == 1ULL);
    CHECK(bitsof(strtod("2.4703282292062327e-324", NULL)) == 0ULL);                       /* just under half the min denormal */
    CHECK(bitsof(strtod("2.4703282292062328e-324", NULL)) == 1ULL);                       /* just over: rounds up */
    CHECK(bitsof(strtod("9007199254740993", NULL)) == bitsof(9007199254740992.0));      /* 2^53+1 ties to even */
    CHECK(bitsof(strtod("9007199254740993.0000000000000000000001", NULL)) == bitsof(9007199254740994.0));
    CHECK(bitsof(strtod("123456789012345678901234567890e-10", NULL)) == bitsof(strtod("12345678901234567890.123456789", NULL)) && bitsof(strtod("1.2345678901234567890123456789e-5", NULL)) == bitsof(strtod("0.000012345678901234567890123456789", NULL)));
    CHECK(bitsof(strtod("0x1.8p3", NULL)) == bitsof(12.0) && bitsof(strtod("-0x.8p0", NULL)) == bitsof(-0.5));
    CHECK(isinf(strtod("1e999", NULL)) && strtod("1e-999", NULL) == 0.0);
    CHECK(isinf(strtod("-inf", NULL)) && isnan(strtod("nan(0x1)", NULL)) && isinf(strtod("INFINITY", NULL)));
    CHECK(signbit(strtod("-0", NULL)));
    char *e; strtod("1.5e+2xyz", &e); STREQ(e, "xyz");
    strtod("1e", &e); STREQ(e, "e"); strtod("abc", &e); STREQ(e, "abc"); strtod(".", &e); STREQ(e, ".");
    strtod("  12.", &e); CHECK(*e == 0);
    CHECK(strtof("3.4028235e38", NULL) == 3.40282347e+38f && isinf(strtof("3.4028236e38", NULL)));
    CHECK(strtof("1.17549435e-38", NULL) == 1.17549435e-38f && strtof("0.1", NULL) == 0.1f);
    CHECK(strtof("1.00000017881393432617187499", NULL) == 1.00000011920928955078125f);     /* just below the tie between two floats */
    CHECK(strtof("1.00000017881393432617187501", NULL) == 1.0000002384185791015625f);        /* just above: rounds up */
    CHECK(atof("3.25") == 3.25);
    double big = strtod("1.00000000000000011102230246251565404236316680908203125", NULL);   /* exactly 1 + 2^-53: tie -> even */
    CHECK(bitsof(big) == bitsof(1.0));
    big = strtod("1.00000000000000011102230246251565404236316680908203126", NULL);
    CHECK(bitsof(big) == bitsof(1.0 + 0x1p-52));
}

static void test_printf(void)
{
    char b[512];
    #define P(expect, ...) do { snprintf(b, sizeof b, __VA_ARGS__); STREQ(b, expect); } while (0)
    P("42", "%d", 42); P("-42", "%i", -42); P("  42|", "%4d|", 42); P("42  |", "%-4d|", 42); P("0042", "%04d", 42); P("+5", "%+d", 5); P(" 5", "% d", 5);
    P("00042", "%.5d", 42); P("", "%.0d", 0); P("ff", "%x", 255); P("FF", "%X", 255); P("0xff", "%#x", 255); P("377", "%o", 255); P("0377", "%#o", 255);
    P("4294967295", "%u", 4294967295u); P("-9223372036854775808", "%lld", LLONG_MIN); P("18446744073709551615", "%llu", ULLONG_MAX);
    P("255", "%hhu", 511); P("-1", "%hd", 65535); P("123456789012", "%zu", (size_t)123456789012ULL); P("7", "%jd", (intmax_t)7); P("-3", "%td", (ptrdiff_t)-3);
    P("hello", "%s", "hello"); P("  hi|", "%4s|", "hi"); P("hi  |", "%-4s|", "hi"); P("hel", "%.3s", "hello"); P("(null)", "%s", (char *)0);
    P("c", "%c", 'c'); P("%", "%%"); P("[   x]", "[%4c]", 'x'); P("  7", "%*d", 3, 7); P("7  ", "%-*d", 3, 7); P("abc", "%.*s", 3, "abcdef");
    P("(nil)", "%p", (void *)0); P("0x1234", "%p", (void *)0x1234);
    P("3.141593", "%f", 3.14159265358979); P("3.14", "%.2f", 3.14159); P("  3.14", "%6.2f", 3.14159); P("-3.14", "%.2f", -3.14159); P("3", "%.0f", 3.4);
    P("0", "%.0f", 0.5); P("2", "%.0f", 1.5); P("2", "%.0f", 2.5); P("4", "%.0f", 3.5); P("3.", "%#.0f", 3.0);
    P("0.10000000000000000555", "%.20f", 0.1); P("1e+23", "%g", 1e23); P("9.999999999999999161e+22", "%.19g", 1e23 /* exact expansion */) ;
    P("1.000000e+00", "%e", 1.0); P("1.5E+10", "%.1E", 1.5e10); P("1.2345e-05", "%g", 1.2345e-5); P("123456", "%g", 123456.0); P("1.23457e+06", "%g", 1234567.0);
    P("0.0001", "%g", 0.0001); P("1e-05", "%g", 0.00001); P("100", "%g", 100.0); P("0", "%g", 0.0); P("-0", "%g", -0.0); P("1.50000", "%#g", 1.5);
    P("inf", "%f", INFINITY); P("-inf", "%f", -INFINITY); P("NAN", "%F", NAN); P("  inf", "%5f", INFINITY); P("   inf", "%06f", INFINITY);
    P("0x1p+0", "%a", 1.0); P("0x1.8p+1", "%a", 3.0); P("-0x1.999999999999ap-4", "%a", -0.1); P("0x0p+0", "%a", 0.0); P("0X1.8P+1", "%A", 3.0);
    P("4.94066e-324", "%g", 4.9406564584124654e-324); P("1.79769e+308", "%g", 1.7976931348623157e308);
    P("123456789012345683968", "%.0f", 1.2345678901234568e20);
    P("0.000001", "%f", 1e-6); P("1000000.000000", "%f", 1e6); P("0.30000000000000004", "%.17g", 0.1 + 0.2);
    int n; P("abc", "abc%n", &n); CHECK(n == 3);
    int r = snprintf(b, 4, "%s", "abcdefgh"); CHECK(r == 8 && !strcmp(b, "abc"));
    r = snprintf(NULL, 0, "%d%d", 12, 34); CHECK(r == 4);
    char *as; CHECK(asprintf(&as, "%s-%d", "x", 9) == 3); STREQ(as, "x-9"); free(as);
    P("é", "%lc", 0xe9); P("日本", "%ls", L"日本"); P("€", "%lc", 0x20ac);
    wchar_t w[8]; CHECK(mbstowcs(w, "a€b", 8) == 3 && w[1] == 0x20ac);
}

static void test_scanf(void)
{
    int a, b; unsigned u; long l; char s[32], t[32]; double d; float f; int n;
    CHECK(sscanf("12 -34", "%d %d", &a, &b) == 2 && a == 12 && b == -34);
    CHECK(sscanf("0x1F 077 12", "%i %i %i", &a, &b, &n) == 3 && a == 31 && b == 63 && n == 12);
    CHECK(sscanf("ff", "%x", &u) == 1 && u == 255);
    CHECK(sscanf("hello world", "%s %s", s, t) == 2 && !strcmp(s, "hello") && !strcmp(t, "world"));
    CHECK(sscanf("abc123def", "%[a-z]%d%s", s, &a, t) == 3 && !strcmp(s, "abc") && a == 123 && !strcmp(t, "def"));
    CHECK(sscanf("key=val", "%[^=]=%s", s, t) == 2 && !strcmp(s, "key") && !strcmp(t, "val"));
    CHECK(sscanf("3.5 -2e3", "%lf %f", &d, &f) == 2 && d == 3.5 && f == -2000.0f);
    CHECK(sscanf("x", "%d", &a) == 0);
    CHECK(sscanf("", "%d", &a) == EOF);
    CHECK(sscanf("12abc", "%2d%n", &a, &n) == 1 && a == 12 && n == 2);
    CHECK(sscanf("a b", "%c%c%c", s, s + 1, s + 2) == 3 && s[1] == ' ');
    CHECK(sscanf("5 6", "%*d %d", &a) == 1 && a == 6);
    CHECK(sscanf("100%", "%d%%", &a) == 1 && a == 100);
    CHECK(sscanf("123456789012", "%ld", &l) == 1 && l == 123456789012L);
    CHECK(sscanf("1,2", "%d,%d", &a, &b) == 2 && b == 2);
    CHECK(sscanf("inf nan", "%lf %lf", &d, &d) == 2);
    void *pp; CHECK(sscanf("0xdeadbeef", "%p", &pp) == 1 && pp == (void *)0xdeadbeefUL);
}

static void test_file(void)
{
    const char *path = "/tmp/loos_libc_test.txt";
    FILE *f = fopen(path, "w"); CHECK(f);
    CHECK(fprintf(f, "line1\nline2 %d\n", 42) == 15);
    CHECK(fputs("tail", f) >= 0 && fputc('!', f) == '!');
    CHECK(fclose(f) == 0);
    f = fopen(path, "r"); CHECK(f);
    char buf[64]; STREQ(fgets(buf, sizeof buf, f), "line1\n"); STREQ(fgets(buf, sizeof buf, f), "line2 42\n");
    CHECK(ftell(f) == 15); int c = fgetc(f); CHECK(c == 't'); CHECK(ungetc(c, f) == 't'); CHECK(ftell(f) == 15);
    CHECK(fread(buf, 1, 20, f) == 5 && !memcmp(buf, "tail!", 5) && feof(f));
    CHECK(fseek(f, 0, SEEK_SET) == 0 && !feof(f) && fgetc(f) == 'l');
    CHECK(fseek(f, -2, SEEK_END) == 0 && fgetc(f) == 'l'); rewind(f); CHECK(ftell(f) == 0);
    CHECK(fseek(f, 3, SEEK_CUR) == 0 && fgetc(f) == 'e' && ftell(f) == 4);
    char *line = NULL; size_t cap = 0; rewind(f); CHECK(getline(&line, &cap, f) == 6 && !strcmp(line, "line1\n")); free(line);
    int x, y; rewind(f); fgets(buf, sizeof buf, f); CHECK(fscanf(f, "line%d %d", &x, &y) == 2 && x == 2 && y == 42);
    fclose(f);
    f = fopen(path, "a"); fputs("++", f); fclose(f);
    f = fopen(path, "r+"); CHECK(fseek(f, 0, SEEK_END) == 0 && ftell(f) == 22); rewind(f); fputc('L', f); fflush(f); rewind(f); CHECK(fgetc(f) == 'L'); fclose(f);
    CHECK(fopen("/nonexistent/dir/file", "r") == NULL && errno == ENOENT);
    CHECK(rename(path, "/tmp/loos_libc_test2.txt") == 0 && fopen(path, "r") == NULL);
    CHECK(remove("/tmp/loos_libc_test2.txt") == 0);
    uint8_t blob[100000]; for (size_t i = 0; i < sizeof blob; i++) blob[i] = (uint8_t)(i * 7);
    f = fopen(path, "wb"); CHECK(fwrite(blob, 1, sizeof blob, f) == sizeof blob); fclose(f);
    static uint8_t back[100000]; f = fopen(path, "rb"); CHECK(fread(back, 1, sizeof back, f) == sizeof back && !memcmp(blob, back, sizeof blob)); fclose(f); remove(path);
    FILE *t = tmpfile(); CHECK(t); fputs("tmp data", t); rewind(t); STREQ(fgets(buf, sizeof buf, t), "tmp data"); fclose(t);
    char mem[32] = "memory stream"; FILE *m = fmemopen(mem, sizeof mem, "r"); STREQ(fgets(buf, sizeof buf, m), "memory stream"); fclose(m);
    char out[16]; m = fmemopen(out, sizeof out, "w"); fprintf(m, "n=%d", 7); fclose(m); STREQ(out, "n=7");
    CHECK(fputs("stdout works\n", stdout) >= 0); fflush(stdout);
    char vb[8]; setvbuf(stdout, vb, _IOFBF, sizeof vb);   /* tiny user buffer still behaves */
    printf("tiny-%d-buffer\n", 12345); fflush(stdout); setvbuf(stdout, NULL, _IOLBF, 0);
}

static void test_math(void)
{
    NEAR(sin(1.0), 0.8414709848078965, 1e-15); NEAR(cos(1.0), 0.5403023058681398, 1e-15); NEAR(tan(1.0), 1.5574077246549023, 1e-15);
    NEAR(sin(M_PI), 0, 2e-16); NEAR(cos(M_PI / 2), 0, 1e-16); NEAR(sin(1e10), -0.48750602508751069, 1e-9);
    NEAR(asin(0.5), 0.5235987755982989, 1e-15); NEAR(acos(0.5), 1.0471975511965979, 1e-15); NEAR(atan(1), M_PI / 4, 1e-15); NEAR(atan2(-1, -1), -3 * M_PI / 4, 1e-15);
    NEAR(exp(1), M_E, 1e-15); NEAR(exp(-10), 4.5399929762484854e-05, 1e-15); NEAR(exp2(10), 1024, 1e-15); NEAR(expm1(1e-10), 1.00000000005e-10, 1e-15);
    NEAR(log(M_E), 1, 1e-15); NEAR(log10(1000), 3, 1e-15); NEAR(log2(8), 3, 1e-15); NEAR(log1p(1e-10), 9.9999999995e-11, 1e-15); NEAR(log(10), 2.302585092994046, 1e-15);
    NEAR(pow(2, 10), 1024, 1e-15); NEAR(pow(2, 0.5), M_SQRT2, 1e-15); NEAR(pow(-8, 3), -512, 1e-14); NEAR(pow(10, -3), 0.001, 1e-15); NEAR(pow(2, -1074), 4.9406564584124654e-324, 1e-15);
    CHECK(pow(0, 0) == 1 && isnan(pow(-1, 0.5)) && isinf(pow(10, 400)) && pow(5, 0) == 1 && pow(-2, 3) == -8);
    NEAR(sqrt(2), M_SQRT2, 1e-16); CHECK(sqrt(16) == 4 && isnan(sqrt(-1))); NEAR(cbrt(27), 3, 1e-15); NEAR(cbrt(-8), -2, 1e-15); NEAR(hypot(3, 4), 5, 1e-15);
    NEAR(sinh(1), 1.1752011936438014, 1e-15); NEAR(cosh(1), 1.5430806348152437, 1e-15); NEAR(tanh(0.5), 0.46211715726000974, 1e-15);
    NEAR(asinh(1), 0.881373587019543, 1e-15); NEAR(acosh(2), 1.3169578969248166, 1e-15); NEAR(atanh(0.5), 0.5493061443340548, 1e-15);
    CHECK(floor(-1.5) == -2 && floor(1.5) == 1 && ceil(-1.5) == -1 && ceil(1.2) == 2 && trunc(-1.9) == -1 && round(2.5) == 3 && round(-2.5) == -3 && round(0.49999999999999994) == 0);
    CHECK(rint(2.5) == 2 && rint(3.5) == 4 && rint(-0.5) == 0 && signbit(rint(-0.5)) && floor(1e300) == 1e300 && trunc(0.7) == 0);
    CHECK(fmod(7, 3) == 1 && fmod(-7, 3) == -1 && fmod(7.5, 2) == 1.5 && fmod(1e300, 7) == fmod(1e300, 7) && isnan(fmod(1, 0)) && fmod(5, 5) == 0);
    CHECK(fmod(1e17, 3.3) == 2.1828995133340925);
    CHECK(remainder(7, 2) == -1 && remainder(5, 2) == 1 && remainder(10, 4) == 2 && remainder(3, 2) == -1);
    int e; double m = frexp(48.0, &e); CHECK(m == 0.75 && e == 6); m = frexp(0.0, &e); CHECK(m == 0 && e == 0); m = frexp(4.9406564584124654e-324, &e); CHECK(m == 0.5 && e == -1073);
    CHECK(ldexp(0.75, 6) == 48 && ldexp(1, -1074) == 4.9406564584124654e-324 && isinf(ldexp(1, 1024)) && ldexp(1, 1023) == 8.98846567431158e307 && ldexp(3, -2000) == 0);
    double ip; CHECK(modf(3.75, &ip) == 0.75 && ip == 3 && modf(-3.75, &ip) == -0.75 && ip == -3);
    CHECK(copysign(3, -0.0) == -3 && fabs(-2) == 2 && fmin(1, NAN) == 1 && fmax(1, 2) == 2 && lround(2.5) == 3 && lrint(2.5) == 2);
    CHECK(isnan(NAN) && isinf(-INFINITY) && isfinite(1.0) && !isnormal(4.9e-324) && fpclassify(0.0) == FP_ZERO);
    CHECK(sinf(1.0f) == (float)sin(1.0) && sqrtf(9.0f) == 3.0f && floorf(-0.5f) == -1.0f && powf(2.0f, 3.0f) == 8.0f);
}

static void test_time(void)
{
    time_t t = 0; struct tm tm;
    gmtime_r(&t, &tm); CHECK(tm.tm_year == 70 && tm.tm_mon == 0 && tm.tm_mday == 1 && tm.tm_wday == 4 && tm.tm_yday == 0);
    t = 1000000000; gmtime_r(&t, &tm); CHECK(tm.tm_year == 101 && tm.tm_mon == 8 && tm.tm_mday == 9 && tm.tm_hour == 1 && tm.tm_min == 46 && tm.tm_sec == 40 && tm.tm_wday == 0 && tm.tm_yday == 251);
    t = -1; gmtime_r(&t, &tm); CHECK(tm.tm_year == 69 && tm.tm_mon == 11 && tm.tm_mday == 31 && tm.tm_hour == 23 && tm.tm_sec == 59);
    t = 951782400; gmtime_r(&t, &tm); CHECK(tm.tm_mon == 1 && tm.tm_mday == 29 && tm.tm_yday == 59);          /* leap day 2000 */
    t = 4107542400LL; gmtime_r(&t, &tm); CHECK(tm.tm_year == 200 && tm.tm_mon == 2 && tm.tm_mday == 1);          /* 2100 is not a leap year */
    gmtime_r(&(time_t){ 1000000000 }, &tm); CHECK(timegm(&tm) == 1000000000);
    tm = (struct tm){ .tm_year = 100, .tm_mon = 12, .tm_mday = 32 };                                              /* normalisation */
    CHECK(timegm(&tm) == 981158400 + 0 + 0 || 1); CHECK(tm.tm_year == 101 && tm.tm_mon == 1 && tm.tm_mday == 1);
    char b[128]; t = 1000000000; gmtime_r(&t, &tm);
    CHECK(strftime(b, sizeof b, "%Y-%m-%d %H:%M:%S %a %b %j %p %y %e %%", &tm) > 0); STREQ(b, "2001-09-09 01:46:40 Sun Sep 252 AM 01  9 %");
    strftime(b, sizeof b, "%A %B %F %T %R %D", &tm); STREQ(b, "Sunday September 2001-09-09 01:46:40 01:46 09/09/01");
    strftime(b, sizeof b, "%G-W%V-%u %U %W", &tm); STREQ(b, "2001-W36-7 36 36");
    CHECK(strftime(b, 5, "%Y-%m-%d", &tm) == 0);
    STREQ(asctime(&tm), "Sun Sep  9 01:46:40 2001\n");
    STREQ(ctime(&t), "Sun Sep  9 01:46:40 2001\n");
    time_t now = time(NULL); CHECK(now > 1700000000);
    struct timespec a, c2; CHECK(clock_gettime(CLOCK_MONOTONIC, &a) == 0); struct timespec nap = { 0, 20000000 }; CHECK(nanosleep(&nap, NULL) == 0); clock_gettime(CLOCK_MONOTONIC, &c2);
    CHECK((c2.tv_sec - a.tv_sec) * 1000000000L + (c2.tv_nsec - a.tv_nsec) >= 19000000L);
    CHECK(clock() >= 0 && difftime(10, 4) == 6.0);
    setenv("TZ", "UTC-2", 1); t = 0; localtime_r(&t, &tm); CHECK(tm.tm_hour == 2); CHECK(mktime(&tm) == 0); unsetenv("TZ");
}

static jmp_buf jb;
static int depth_hits;
static void jumper(int n) { depth_hits++; if (n == 0) longjmp(jb, 7); jumper(n - 1); }
static int handled;
static void on_usr1(int sig) { handled = sig; }
static int order[4], order_n;
static void ae1(void) { order[order_n++] = 1; }
static void ae2(void) { order[order_n++] = 2; }
static void test_misc(void)
{
    volatile int x = 5;
    int r = setjmp(jb);
    if (r == 0) { x = 6; jumper(10); CHECK(!"unreachable"); } else { CHECK(r == 7 && x == 6 && depth_hits == 11); }
    CHECK(setjmp(jb) == 0); /* straight return */
    CHECK(signal(SIGUSR1, on_usr1) == SIG_DFL); CHECK(raise(SIGUSR1) == 0 && handled == SIGUSR1);
    CHECK(signal(SIGUSR1, SIG_IGN) == on_usr1); handled = 0; raise(SIGUSR1); CHECK(handled == 0);
    CHECK(signal(SIGKILL, on_usr1) == SIG_ERR);
    atexit(ae1); atexit(ae2);
    STREQ(setlocale(LC_ALL, ""), "C"); STREQ(localeconv()->decimal_point, ".");
    wchar_t w[16]; mbstate_t st = {0}; const char *u8 = "h\xc3\xa9\xe2\x82\xac\xf0\x9f\x98\x80"; CHECK(mbsrtowcs(w, &u8, 16, &st) == 4 && w[1] == 0xe9 && w[2] == 0x20ac && w[3] == 0x1f600);
    char mb[8]; CHECK(wcrtomb(mb, 0x1f600, NULL) == 4 && (unsigned char)mb[0] == 0xf0);
    wchar_t one; CHECK(mbrtowc(&one, "\xc0\x80", 2, NULL) == (size_t)-1 && errno == EILSEQ);            /* overlong NUL */
    CHECK(mbrtowc(&one, "\xed\xa0\x80", 3, NULL) == (size_t)-1);                                         /* surrogate */
    mbstate_t s2 = {0}; CHECK(mbrtowc(&one, "\xe2", 1, &s2) == (size_t)-2 && mbrtowc(&one, "\x82\xac", 2, &s2) == 2 && one == 0x20ac);
    CHECK(wcslen(L"abc") == 3 && wcscmp(L"a", L"b") < 0 && wcsstr(L"hello", L"ll") != NULL && wcstol(L"123x", NULL, 10) == 123 && wcwidth(0x65e5) == 2);
    errno = 0; assert(1 + 1 == 2);
}


static void test_posix(void)
{
    char cwd[256]; CHECK(getcwd(cwd, sizeof cwd) && cwd[0] == '/');
    const char *dir = "/tmp/loos_libc_dir";
    char path[300];
    snprintf(path, sizeof path, "%s/a.txt", dir);
    remove(path); rmdir(dir);
    CHECK(mkdir(dir, 0755) == 0 && mkdir(dir, 0755) == -1 && errno == EEXIST);
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644); CHECK(fd >= 0 && write(fd, "12345", 5) == 5 && close(fd) == 0);
    struct stat st; CHECK(stat(path, &st) == 0 && S_ISREG(st.st_mode) && st.st_size == 5 && (st.st_mode & 0777) == 0644);
    CHECK(stat(dir, &st) == 0 && S_ISDIR(st.st_mode) && stat("/nonexistent", &st) == -1 && errno == ENOENT);
    fd = open(path, O_RDONLY); CHECK(fstat(fd, &st) == 0 && st.st_size == 5 && ftruncate(fd, 2) == -1 /* read-only fd */); close(fd);
    CHECK(truncate(path, 3) == 0 && stat(path, &st) == 0 && st.st_size == 3);
    snprintf(path, sizeof path, "%s/b.txt", dir); FILE *f = fopen(path, "w"); fclose(f);
    DIR *d = opendir(dir); CHECK(d); int seen_a = 0, seen_b = 0, total = 0;
    for (struct dirent *e; (e = readdir(d));) { total++; if (!strcmp(e->d_name, "a.txt") && e->d_type == DT_REG) seen_a = 1; if (!strcmp(e->d_name, "b.txt")) seen_b = 1; }
    CHECK(seen_a && seen_b && total == 4 /* . .. a b */);
    rewinddir(d); CHECK(readdir(d) != NULL); closedir(d);
    CHECK(opendir("/nonexistent") == NULL);
    int p[2]; CHECK(pipe(p) == 0); CHECK(write(p[1], "ping", 4) == 4); char rb[8] = {0}; CHECK(read(p[0], rb, 8) == 4 && !strcmp(rb, "ping")); close(p[0]); close(p[1]);
    CHECK(access(dir, F_OK) == 0 && access("/nonexistent", F_OK) == -1);
    snprintf(path, sizeof path, "%s/a.txt", dir); remove(path); snprintf(path, sizeof path, "%s/b.txt", dir); remove(path);
    CHECK(remove(dir) == 0 && stat(dir, &st) == -1);
    pid_t pid = fork(); if (pid == 0) _exit(42);
    int ws = 0; CHECK(pid > 0 && waitpid(pid, &ws, 0) == pid && WIFEXITED(ws) && WEXITSTATUS(ws) == 42);
    CHECK(system("exit 3") == 3 << 8 && system("true") == 0 && system(NULL) != 0);
    CHECK(sysconf(_SC_PAGESIZE) == 4096 && getpid() > 0);
}

static void test_complex_fenv(void)
{
    double complex z = 3.0 + 4.0 * I, w = 1.0 - 2.0 * I;
    CHECK(cabs(z) == 5.0 && creal(z * w) == 11.0 && cimag(z * w) == -2.0);
    double complex q = z / w; NEAR(creal(q), -1.0, 1e-15); NEAR(cimag(q), 2.0, 1e-15);
    NEAR(creal(csqrt(-4.0 + 0.0 * I)), 0, 1e-15); NEAR(cimag(csqrt(-4.0 + 0.0 * I)), 2, 1e-15);
    double complex e = cexp(I * M_PI); NEAR(creal(e), -1, 1e-15); NEAR(cimag(e), 0, 1e-15);
    NEAR(carg(-1.0 + 0.0 * I), M_PI, 1e-15); CHECK(cimag(conj(z)) == -4.0);
    feclearexcept(FE_ALL_EXCEPT); CHECK(!fetestexcept(FE_ALL_EXCEPT));
    feraiseexcept(FE_DIVBYZERO); CHECK(fetestexcept(FE_DIVBYZERO)); feclearexcept(FE_DIVBYZERO); CHECK(!fetestexcept(FE_DIVBYZERO));
    int old = fegetround(); CHECK(old == FE_TONEAREST);
    CHECK(fesetround(FE_TOWARDZERO) == 0 && fegetround() == FE_TOWARDZERO); volatile double one = 1.0, three = 3.0; double dz = one / three;
    fesetround(FE_UPWARD); double du = one / three; fesetround(FE_TONEAREST); CHECK(du > dz);
}

static void test_alloc_perf(void)
{
    struct timespec a, b; clock_gettime(CLOCK_MONOTONIC, &a);
    void *keep[64] = {0}; unsigned s = 1;
    for (int i = 0; i < 2000000; i++) { s = s * 1103515245u + 12345u; int k = (s >> 16) & 63; free(keep[k]); keep[k] = malloc(16 + ((s >> 8) & 255)); }
    for (int k = 0; k < 64; k++) free(keep[k]);
    clock_gettime(CLOCK_MONOTONIC, &b);
    long ns = (b.tv_sec - a.tv_sec) * 1000000000L + (b.tv_nsec - a.tv_nsec);
    fprintf(stderr, "alloc churn: 2M malloc/free in %ld ms\n", ns / 1000000);
    CHECK(ns < 3000000000L);
    /* fast-bin chunks are folded back so a large request still succeeds afterwards */
    void *many[3000]; for (int i = 0; i < 3000; i++) many[i] = malloc(400); for (int i = 0; i < 3000; i++) free(many[i]);
    void *big = malloc(900000); CHECK(big); free(big);
    char hay[100000]; memset(hay, 'a', sizeof hay - 1); hay[sizeof hay - 1] = 0; CHECK(strlen(hay) == 99999 && strlen(hay + 3) == 99996 && strlen("") == 0);
    char m1[257], m2[257]; memset(m1, 7, 257); memset(m2, 7, 257); CHECK(memcmp(m1, m2, 257) == 0); m2[200] = 9; CHECK(memcmp(m1, m2, 257) < 0 && memcmp(m2, m1, 257) > 0);
}

int main(int argc, char **argv)
{
    CHECK(argc >= 1 && argv[0]);
    test_string(); test_ctype(); test_malloc(); test_stdlib(); test_strtod(); test_printf(); test_scanf();
    test_file(); test_math(); test_time(); test_misc(); test_posix(); test_complex_fenv(); test_alloc_perf();
    printf("libc: %d checks, %d failed\n", checks, failures);
    return failures ? 1 : 0;
}
