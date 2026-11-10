/* LoOS vDSO: lets Linux programs read the clock without trapping into the kernel.
 *
 * The kernel maps a read-only time page (struct vvar) right before this image and refreshes it every
 * time a thread enters user mode: `ns_base` is the monotonic time at that moment and `tsc_base` the CPU
 * timestamp counter then. Time keeps advancing while user code runs: ns = ns_base + (tsc - tsc_base) * mult >> 32.
 * Clocks this page cannot answer fall back to the real system call. */
#include <stdint.h>

struct vvar {
    uint64_t ns_base;   /* CLOCK_MONOTONIC at tsc_base */
    uint64_t tsc_base;
    uint64_t mult;      /* nanoseconds per TSC tick, 32.32 fixed point */
    uint64_t epoch_s;   /* CLOCK_REALTIME - CLOCK_MONOTONIC, in seconds */
    uint32_t cpu;
};

struct timespec_k { int64_t tv_sec, tv_nsec; };
struct timeval_k { int64_t tv_sec, tv_usec; };

#define VVAR ((const volatile struct vvar *)0x7ffff8000000ull)

static inline uint64_t rdtsc(void)
{
    uint32_t lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}

static inline long sys(long n, long a, long b)
{
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b) : "rcx", "r11", "memory");
    return r;
}

static uint64_t mono_ns(void)
{
    uint64_t dt = rdtsc() - VVAR->tsc_base;
    return VVAR->ns_base + (uint64_t)(((unsigned __int128)dt * VVAR->mult) >> 32);
}

int __vdso_clock_gettime(int clk, struct timespec_k *ts)
{
    uint64_t ns;
    switch (clk) {
    case 1: case 4: case 6: case 7: ns = mono_ns(); break;                           /* MONOTONIC, _RAW, _COARSE, BOOTTIME */
    case 0: case 5: ns = mono_ns() + VVAR->epoch_s * 1000000000ull; break;            /* REALTIME, _COARSE */
    default: return (int)sys(228, clk, (long)ts);                                    /* CPU-time clocks etc. */
    }
    ts->tv_sec = (int64_t)(ns / 1000000000ull);
    ts->tv_nsec = (int64_t)(ns % 1000000000ull);
    return 0;
}

int __vdso_gettimeofday(struct timeval_k *tv, void *tz)
{
    if (tv) {
        uint64_t ns = mono_ns() + VVAR->epoch_s * 1000000000ull;
        tv->tv_sec = (int64_t)(ns / 1000000000ull);
        tv->tv_usec = (int64_t)(ns % 1000000000ull / 1000);
    }
    if (tz) {
        ((int *)tz)[0] = 0;
        ((int *)tz)[1] = 0;
    }
    return 0;
}

long __vdso_time(long *t)
{
    long s = (long)((mono_ns() / 1000000000ull) + VVAR->epoch_s);
    if (t) *t = s;
    return s;
}

int __vdso_getcpu(unsigned *cpu, unsigned *node, void *unused)
{
    (void)unused;
    if (cpu) *cpu = VVAR->cpu;
    if (node) *node = 0;
    return 0;
}

int __vdso_clock_getres(int clk, struct timespec_k *res)
{
    if (clk < 0 || clk > 11) return (int)sys(229, clk, (long)res);
    if (res) { res->tv_sec = 0; res->tv_nsec = 1; }
    return 0;
}

/* Unversioned aliases, as the Linux vDSO exports them. */
int clock_gettime(int, struct timespec_k *) __attribute__((weak, alias("__vdso_clock_gettime")));
int gettimeofday(struct timeval_k *, void *) __attribute__((weak, alias("__vdso_gettimeofday")));
long time(long *) __attribute__((weak, alias("__vdso_time")));
int getcpu(unsigned *, unsigned *, void *) __attribute__((weak, alias("__vdso_getcpu")));
int clock_getres(int, struct timespec_k *) __attribute__((weak, alias("__vdso_clock_getres")));
