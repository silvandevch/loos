/* LoOS Prism test program: built for macOS (x86_64 and arm64) with clang + ld64.lld against libSystem.tbd. */
int printf(const char *, ...);
int snprintf(char *, unsigned long, const char *, ...);
int puts(const char *);
void *malloc(unsigned long);
void *realloc(void *, unsigned long);
void free(void *);
long write(int, const void *, unsigned long);
unsigned long strlen(const char *);
int strcmp(const char *, const char *);
long strtol(const char *, char **, int);
char *getenv(const char *);
int atexit(void (*)(void));
void *memset(void *, int, unsigned long);

static int initialized;
__attribute__((constructor)) static void init(void) { initialized = getenv("PRISM_NO_SUCH_VAR") ? 1 : 42; }
const char *farewell = "atexit handler ran";
static void bye(void) { puts(farewell); }
static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
struct pt { double x, y; long tag; };
static double dot(struct pt a, struct pt b) { return a.x * b.x + a.y * b.y; }

int main(int argc, char **argv) {
    atexit(bye);
    printf("prism: hello from %s, argc=%d, ctor=%d\n", argv[0], argc, initialized);
    char *p = malloc(8);
    for (int i = 0; i < 1000; i++) { p = realloc(p, 8 + i * 3); p[i] = (char)('a' + i % 26); }
    p[26] = 0;
    printf("heap: %s len=%lu\n", p, strlen(p));
    free(p);
    char buf[64];
    int n = snprintf(buf, sizeof buf, "%5.2f|%-4d|%x|%s|%c", 3.14159, 7, 0xbeef, "str", 'Z');
    printf("fmt: [%s] n=%d\n", buf, n);
    struct pt a = {1.5, 2.0, 1}, b = {4.0, -0.5, 2};
    printf("math: fib(20)=%d dot=%.3f div=%ld\n", fib(20), dot(a, b), strtol("-1234", 0, 10) / 7);
    const char *h = getenv("HOME");
    printf("env: HOME=%s cmp=%d\n", h ? h : "(none)", strcmp("abc", "abd") < 0);
    const char *m = "write: ok\n";
    write(1, m, strlen(m));
    return argc == 1 ? 7 : 8;
}
