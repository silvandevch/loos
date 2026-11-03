/* lbox: freestanding Linux x86-64 test program (no libc). Multi-call: `lbox echo ARGS...` or `lbox selftest`.
 * Built with clang -static -nostdlib; runs unmodified under LoOS's Linux subsystem (and on a real Linux for comparison). */
typedef unsigned long u64; typedef long i64; typedef unsigned int u32;

static i64 sc(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f)
{
    i64 r; register i64 r10 __asm__("r10") = d; register i64 r8 __asm__("r8") = e; register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return r;
}
#define S0(n) sc(n,0,0,0,0,0,0)
#define S1(n,a) sc(n,(i64)(a),0,0,0,0,0)
#define S2(n,a,b) sc(n,(i64)(a),(i64)(b),0,0,0,0)
#define S3(n,a,b,c) sc(n,(i64)(a),(i64)(b),(i64)(c),0,0,0)
#define S4(n,a,b,c,d) sc(n,(i64)(a),(i64)(b),(i64)(c),(i64)(d),0,0)
#define S6(n,a,b,c,d,e,f) sc(n,(i64)(a),(i64)(b),(i64)(c),(i64)(d),(i64)(e),(i64)(f))

static u64 slen(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void out(const char *s) { S3(1, 1, s, slen(s)); }
static int seq(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }
static void dec(u64 v) { char b[24]; int i = 23; b[i] = 0; do { b[--i] = '0' + v % 10; v /= 10; } while (v); out(b + i); }
static int fails;
static void check(const char *name, int ok) { out(ok ? "ok   " : "FAIL "); out(name); out("\n"); if (!ok) fails++; }

static volatile int got_sig;
static void on_usr1(int s) { got_sig = s; }
struct ksa { void (*h)(int); u64 flags; void (*restorer)(void); u64 mask; };
__attribute__((naked)) static void restorer(void) { __asm__ volatile("mov $15,%eax; syscall"); }

static int selftest(char **envp)
{
    char buf[256];
    /* identity / time / uname */
    check("getpid", S0(39) > 0);
    char uts[390]; S1(63, uts); check("uname Linux", seq(uts, "Linux"));
    u64 ts[2]; check("clock_gettime", S2(228, 1, ts) == 0);
    /* memory */
    i64 b0 = S1(12, 0); i64 b1 = S1(12, b0 + 0x3000); check("brk grow", b1 == b0 + 0x3000);
    *(volatile u64 *)(b0 + 0x2ff0) = 7; check("brk memory writable", *(volatile u64 *)(b0 + 0x2ff0) == 7);
    char *m = (char *)S6(9, 0, 0x10000, 3, 0x22, -1, 0); check("mmap anon", (i64)m > 0);
    for (int i = 0; i < 0x10000; i += 4096) m[i] = (char)i; check("mmap touch", m[0] == 0);
    check("munmap", S2(11, m + 0x1000, 0x1000) == 0);
    char *sh = (char *)S6(9, 0, 4096, 3, 0x21, -1, 0);                         /* MAP_SHARED|ANON */
    /* stack growth (touch 200 KiB below the initial 256 KiB window is not needed; use a big frame) */
    volatile char big[300 * 1024]; big[0] = 1; big[sizeof big - 1] = 2; check("stack growth", big[0] + big[sizeof big - 1] == 3);
    /* files from the initrd */
    int fd = (int)S3(257, -100, "/etc/motd", 0); i64 n = S3(0, fd, buf, 255);
    check("open+read initrd file", fd >= 3 && n > 0 && buf[0] == 'L');
    S1(3, fd);
    char dents[512]; int dfd = (int)S3(257, -100, "/bin", 0x10000); i64 dn = S3(217, dfd, dents, 512);
    check("getdents64 /bin", dn > 0);
    /* pipe + fork + wait */
    int p[2]; S1(22, p);
    i64 pid = S0(57);
    if (pid == 0) { S3(1, p[1], "child-data", 10); S1(60, 42); }
    n = S3(0, p[0], buf, 255);
    int st = 0; i64 w = S4(61, pid, &st, 0, 0);
    check("fork+pipe+wait4", n == 10 && w == pid && ((st >> 8) & 0xff) == 42);
    /* MAP_SHARED across fork */
    pid = S0(57);
    if (pid == 0) { *(volatile u64 *)sh = 0x5a5a; S1(60, 0); }
    S4(61, pid, &st, 0, 0); check("MAP_SHARED visible after fork", *(volatile u64 *)sh == 0x5a5a);
    /* signals: handler + rt_sigreturn */
    struct ksa sa = { on_usr1, 0x04000000, restorer, 0 };
    S4(13, 10, &sa, 0, 8);
    S2(62, S0(39), 10);
    check("signal handler runs and returns", got_sig == 10);
    /* page fault -> SIGSEGV handler */
    got_sig = 0; S4(13, 11, &sa, 0, 8);
    sa.h = on_usr1;
    check("sigaction SEGV", S4(13, 11, &sa, 0, 8) == 0);
    /* sockets */
    int sp[2]; S4(53, 1, 1, 0, sp); S3(1, sp[0], "hi", 2);
    check("socketpair", S3(0, sp[1], buf, 16) == 2);
    /* nanosleep */
    u64 req[2] = { 0, 2000000 }; check("nanosleep", S2(35, req, 0) == 0);
    /* execve: re-run ourselves in echo mode, parent waits */
    pid = S0(57);
    if (pid == 0) { char *av[] = { "/bin/lbox", "echo", "exec-ok", 0 }; S3(59, "/bin/lbox", av, envp); S1(60, 99); }
    S4(61, pid, &st, 0, 0); check("fork+execve+wait4", ((st >> 8) & 0xff) == 0);
    /* liblooswin bridge: version, window create/draw/destroy */
    check("looswin version", S0(0x4C4F0000) == 0x0A000001);
    i64 hw = S6(0x4C4F0002, "lbox window", 20, 20, 200, 120, 0xcf0000);
    check("looswin create window", hw > 0);
    check("looswin draw", S6(0x4C4F0005, hw, 5, 5, 50, 30, 0x0000ff) == 0 && S6(0x4C4F0006, hw, 5, 40, "hi", 0, 0) == 0);
    check("looswin destroy", S1(0x4C4F0007, hw) == 0);
    /* SSE state survives syscalls */
    volatile double d = 1.5; d = d * 2.0 + 0.25; check("SSE double math", d == 3.25);
    out(fails ? "LBOX: FAILURES\n" : "LBOX: all ok\n");
    return fails ? 1 : 0;
}

int cmain(long *sp)
{
    long argc = sp[0]; char **argv = (char **)(sp + 1); char **envp = argv + argc + 1;
    if (argc >= 2 && seq(argv[1], "echo")) {
        for (int i = 2; i < argc; i++) { out(argv[i]); out(i + 1 < argc ? " " : "\n"); }
        return 0;
    }
    if (argc >= 2 && seq(argv[1], "selftest")) return selftest(envp);
    out("usage: lbox echo ARGS | selftest\n");
    return 2;
}
__attribute__((naked)) void _start(void) { __asm__ volatile("mov %rsp,%rdi; and $-16,%rsp; call cmain; mov %eax,%edi; mov $231,%eax; syscall"); }
