/* Time base: TSC calibrated once against PIT channel 2, plus the CMOS real-time clock. */
#include "kernel.h"

static uint64_t tsc_per_us;

static inline uint64_t rdtsc(void) { uint32_t lo, hi; __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi)); return (uint64_t)hi << 32 | lo; }

void time_init(void)
{
    if (tsc_per_us) return;
    outb(0x61, (uint8_t)((inb(0x61) & ~2) | 1));              /* gate on, speaker off */
    outb(0x43, 0xb0);                                          /* channel 2, lobyte/hibyte, mode 0 */
    uint16_t count = 11932;                                    /* 10 ms at 1.193182 MHz */
    outb(0x42, (uint8_t)count); outb(0x42, (uint8_t)(count >> 8));
    outb(0x61, (uint8_t)(inb(0x61) & ~1)); outb(0x61, (uint8_t)(inb(0x61) | 1));
    uint64_t t0 = rdtsc();
    while (!(inb(0x61) & 0x20)) {}
    uint64_t dt = rdtsc() - t0;
    tsc_per_us = dt / 10000;
    if (!tsc_per_us) tsc_per_us = 1000;
}

uint64_t time_us(void) { return rdtsc() / tsc_per_us; }
uint32_t time_ms(void) { return (uint32_t)(time_us() / 1000); }
void udelay(uint64_t us) { uint64_t t = time_us() + us; while (time_us() < t) __asm__ volatile("pause"); }

static uint8_t cmos(uint8_t reg) { outb(0x70, reg); return inb(0x71); }

void rtc_time(int *hour, int *minute, int *second)
{
    while (cmos(0x0a) & 0x80) {}                               /* update in progress */
    uint8_t h = cmos(4), m = cmos(2), s = cmos(0), b = cmos(0x0b);
    if (!(b & 4)) {                                            /* BCD */
        h = (uint8_t)((h & 0x0f) + ((h & 0x70) >> 4) * 10 + (h & 0x80));
        m = (uint8_t)((m & 0x0f) + (m >> 4) * 10);
        s = (uint8_t)((s & 0x0f) + (s >> 4) * 10);
    }
    if (!(b & 2) && (h & 0x80)) h = (uint8_t)(((h & 0x7f) + 12) % 24);   /* 12-hour clock, PM bit */
    *hour = h; *minute = m; *second = s;
}

/* Randomness for salts: RDRAND when the CPU has it, mixed with TSC and RTC jitter either way (never relied on alone). */
void krandom(uint8_t *out, size_t n)
{
    uint32_t a = 1, b, c, d;
    __asm__ volatile("cpuid" : "+a"(a), "=b"(b), "=c"(c), "=d"(d));
    int have_rdrand = (c >> 30) & 1;
    static uint64_t state;
    int hh, mm, ss;
    rtc_time(&hh, &mm, &ss);
    state ^= rdtsc() * 0x9e3779b97f4a7c15ull ^ ((uint64_t)ss << 40) ^ ((uint64_t)mm << 32);
    for (size_t i = 0; i < n; i += 8) {
        uint64_t r = 0; unsigned char ok = 0;
        if (have_rdrand) __asm__ volatile("rdrand %0; setc %1" : "=r"(r), "=qm"(ok));
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;      /* xorshift64 mixed with the hardware value */
        state += rdtsc();
        uint64_t v = state ^ (ok ? r : 0);
        for (size_t k = 0; k < 8 && i + k < n; k++) out[i + k] = (uint8_t)(v >> (8 * k));
    }
}

uint64_t rtc_unix(void)
{
    while (cmos(0x0a) & 0x80) {}
    uint8_t s = cmos(0), mi = cmos(2), h = cmos(4), d = cmos(7), mo = cmos(8), y = cmos(9), c = cmos(0x32), b = cmos(0x0b);
    if (!(b & 4)) {
        s = (uint8_t)((s & 15) + (s >> 4) * 10); mi = (uint8_t)((mi & 15) + (mi >> 4) * 10); h = (uint8_t)((h & 15) + ((h & 0x70) >> 4) * 10 + (h & 0x80));
        d = (uint8_t)((d & 15) + (d >> 4) * 10); mo = (uint8_t)((mo & 15) + (mo >> 4) * 10); y = (uint8_t)((y & 15) + (y >> 4) * 10); c = (uint8_t)((c & 15) + (c >> 4) * 10);
    }
    int year = (c >= 19 && c <= 30 ? c : 20) * 100 + y;
    int64_t yy = year - (mo <= 2), era = (yy >= 0 ? yy : yy - 399) / 400;
    unsigned yoe = (unsigned)(yy - era * 400), doy = (153u * (mo + (mo > 2 ? -3 : 9)) + 2) / 5 + d - 1, doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    int64_t days = era * 146097 + (int64_t)doe - 719468;
    return (uint64_t)days * 86400 + (uint64_t)h * 3600 + (uint64_t)mi * 60 + s;
}
