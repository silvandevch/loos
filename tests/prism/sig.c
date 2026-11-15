/* Signals through Prism: sigaction handler with Darwin numbering, signal(), sigprocmask, qsort. */
int printf(const char *, ...);
int raise(int);
typedef void (*handler_t)(int);
handler_t signal(int, handler_t);
struct sigaction { handler_t sa_handler; unsigned sa_mask; int sa_flags; };
int sigaction(int, const struct sigaction *, struct sigaction *);
int sigprocmask(int, const unsigned *, unsigned *);
char *realpath(const char *, char *) __asm__("_realpath$DARWIN_EXTSN");
int mkstemp(char *); char *mkdtemp(char *); int close(int); int unlink(const char *); int rmdir(const char *); int chdir(const char *);
int sysctlbyname(const char *, void *, unsigned long *, void *, unsigned long);
int sscanf(const char *, const char *, ...); void *setmode(const char *); unsigned short getmode(const void *, unsigned short); int wcwidth(int); int putwchar(int);
double strtod(const char *, char **); float strtof(const char *, char **); unsigned long mbrtowc(int *, const char *, unsigned long, void *);
int fnmatch(const char *, const char *, int); char *strptime(const char *, const char *, int *); long long strtonum(const char *, long long, long long, const char **);
double fmod(double, double); void openlog(const char *, int, int); void syslog(int, const char *, ...) __asm__("_syslog$DARWIN_EXTSN");
void qsort(void *, unsigned long, unsigned long, int (*)(const void *, const void *));

static volatile int got_usr1, got_term;
static void on_usr1(int s) { got_usr1 = s; }
static void on_term(int s) { got_term = s; }
static int cmp(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

int main(void) {
    struct sigaction sa = {on_usr1, 0, 0};
    sigaction(30, &sa, 0); /* SIGUSR1 is 30 on Darwin */
    raise(30);
    signal(15, on_term);
    raise(15);
    unsigned set = 1u << (30 - 1), old = 0;
    sigprocmask(1, &set, 0); /* block SIGUSR1 */
    sigprocmask(1, 0, &old);
    int v[] = {5, 3, 9, 1, 7};
    qsort(v, 5, sizeof v[0], cmp);
    printf("sig: usr1=%d term=%d blocked=%d sorted=%d%d%d%d%d\n", got_usr1, got_term, (old >> 29) & 1, v[0], v[1], v[2], v[3], v[4]);
    char t[] = "/tmp/fXXXXXX", d[] = "/tmp/dXXXXXX", rp[1024];
    int fd = mkstemp(t);
    int ok_tmp = fd >= 3 && t[6] != 'X' && mkdtemp(d) == d;
    chdir("/tmp");
    char *r = realpath("./../tmp//.", rp);
    int ncpu = 0; unsigned long n = sizeof ncpu;
    int sc = sysctlbyname("hw.ncpu", &ncpu, &n, 0, 0);
    printf("sys: tmp=%d real=%s ncpu=%d/%d missing=%d\n", ok_tmp, r ? r : "(null)", sc, ncpu, realpath("/nope", 0) == 0);
    int a = 0, b = 0; char w[8] = {0}; double x = 0;
    int k = sscanf("12 abc 0x1f 3.5", "%d %2s%*c %i %lf", &a, w, &b, &x);
    printf("scan: k=%d a=%d w=%s b=%d x10=%d mode=%o wcw=%d\n", k, a, w, b, (int)(x * 10), getmode(setmode("u+x,go-w"), 0666), wcwidth(0x4e2d));
    char *end; int wc = 0; char mbs[16] = {0};
    double dv = strtod(" -12.5e1x", &end); float fl = strtof("0.25", 0);
    unsigned long ml = mbrtowc(&wc, "\xc3\xa9", 2, mbs);
    printf("num: d=%d end=%c f=%d mb=%lu/%x\n", (int)dv, *end, (int)(fl * 100), ml, wc);
    int tmv[11] = {0}; const char *es = 0;
    char *pe = strptime("2026-11-15 13:04", "%Y-%m-%d %H:%M", tmv);
    long long tn = strtonum("42", 1, 100, &es), tb = strtonum("420", 1, 100, &es);
    printf("misc: fn=%d%d tm=%d/%d/%d:%d rest=%d tn=%lld/%lld/%s fmod=%d\n", fnmatch("*.c", "a.c", 0), fnmatch("*.c", "a.h", 0), tmv[5], tmv[4], tmv[3], tmv[2], pe && !*pe, tn, tb, es ? es : "-", (int)(fmod(7.5, 2.0) * 10));
    openlog("sigtest", 0x20, 0); syslog(3, "logged %d", 5);
    putwchar(0xe9); putwchar('\n');
    close(fd); unlink(t); rmdir(d);
    return 0;
}
