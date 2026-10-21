/* <time.h>. LoOS has no zone database in the libc: local time is UTC unless TZ is a fixed offset
 * ("UTC", "UTC+hh[:mm]", "UTC-hh[:mm]" where, per POSIX, the sign counts hours *west* of Greenwich). */
#include "libc.h"
#include <time.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <errno.h>
#include <ctype.h>

time_t time(time_t *t)
{
    struct timespec ts;
    time_t r = __os_clock_gettime(CLOCK_REALTIME, &ts) < 0 ? (time_t)-1 : ts.tv_sec;
    if (t) *t = r;
    return r;
}
int clock_gettime(clockid_t id, struct timespec *ts) { return (int)__syscall_ret(__os_clock_gettime(id, ts)); }
int timespec_get(struct timespec *ts, int base) { if (base != TIME_UTC) return 0; return clock_gettime(CLOCK_REALTIME, ts) == 0 ? base : 0; }
clock_t clock(void)
{
    struct timespec ts;
    if (__os_clock_gettime(2 /*PROCESS_CPUTIME_ID*/, &ts) < 0) return (clock_t)-1;
    return (clock_t)(ts.tv_sec * 1000000L + ts.tv_nsec / 1000);
}
double difftime(time_t a, time_t b) { return (double)a - (double)b; }
int nanosleep(const struct timespec *r, struct timespec *m) { return (int)__syscall_ret(__os_nanosleep(r, m)); }
unsigned sleep(unsigned s)
{
    struct timespec req = { (time_t)s, 0 }, rem = { 0, 0 };
    if (__os_nanosleep(&req, &rem) < 0) return (unsigned)rem.tv_sec;
    return 0;
}

static long long days_from_civil(long long y, unsigned m, unsigned d)      /* Howard Hinnant's algorithm */
{
    y -= m <= 2;
    long long era = (y >= 0 ? y : y - 399) / 400;
    unsigned yoe = (unsigned)(y - era * 400);
    unsigned doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    unsigned doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + (long long)doe - 719468;
}

struct tm *gmtime_r(const time_t *restrict tp, struct tm *restrict tm)
{
    long long t = *tp, days = t / 86400, rem = t % 86400;
    if (rem < 0) { rem += 86400; days--; }
    tm->tm_hour = (int)(rem / 3600); tm->tm_min = (int)(rem % 3600 / 60); tm->tm_sec = (int)(rem % 60);
    tm->tm_wday = (int)((days + 4) % 7); if (tm->tm_wday < 0) tm->tm_wday += 7;
    long long z = days + 719468, era = (z >= 0 ? z : z - 146096) / 146097;
    unsigned doe = (unsigned)(z - era * 146097), yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long long y = (long long)yoe + era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    unsigned d = doy - (153 * mp + 2) / 5 + 1, m = mp < 10 ? mp + 3 : mp - 9;
    y += m <= 2;
    if (y - 1900 > 2147483647LL || y - 1900 < -2147483648LL) { errno = EOVERFLOW; return 0; }
    tm->tm_year = (int)(y - 1900); tm->tm_mon = (int)m - 1; tm->tm_mday = (int)d;
    int leap = (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0));
    tm->tm_yday = (int)(days_from_civil(y, m, d) - days_from_civil(y, 1, 1));
    (void)leap;
    tm->tm_isdst = 0; tm->tm_gmtoff = 0; tm->tm_zone = "UTC";
    return tm;
}
struct tm *gmtime(const time_t *t) { static struct tm tm; return gmtime_r(t, &tm); }

static long tz_offset(void)                       /* seconds east of UTC */
{
    const char *tz = getenv("TZ");
    if (!tz || strncmp(tz, "UTC", 3)) return 0;
    tz += 3;
    if (!*tz) return 0;
    int sign = *tz == '-' ? 1 : -1;                 /* POSIX: UTC+5 means 5 hours *behind* */
    if (*tz == '+' || *tz == '-') tz++;
    long h = 0, m = 0;
    while (isdigit((unsigned char)*tz)) h = h * 10 + (*tz++ - '0');
    if (*tz == ':') { tz++; while (isdigit((unsigned char)*tz)) m = m * 10 + (*tz++ - '0'); }
    return sign * (h * 3600 + m * 60);
}
struct tm *localtime_r(const time_t *restrict tp, struct tm *restrict tm)
{
    long off = tz_offset();
    time_t t = *tp + off;
    if (!gmtime_r(&t, tm)) return 0;
    tm->tm_gmtoff = off;
    tm->tm_zone = off ? "" : "UTC";
    return tm;
}
struct tm *localtime(const time_t *t) { static struct tm tm; return localtime_r(t, &tm); }

time_t timegm(struct tm *tm)
{
    long long y = tm->tm_year + 1900LL, mon = tm->tm_mon;
    y += mon / 12; mon %= 12;
    if (mon < 0) { mon += 12; y--; }
    long long days = days_from_civil(y, (unsigned)mon + 1, 1) + (tm->tm_mday - 1);
    time_t t = (time_t)(days * 86400 + tm->tm_hour * 3600LL + tm->tm_min * 60LL + tm->tm_sec);
    struct tm norm;
    gmtime_r(&t, &norm);
    norm.tm_gmtoff = tm->tm_gmtoff; norm.tm_zone = tm->tm_zone;
    *tm = norm;
    return t;
}
time_t mktime(struct tm *tm)
{
    long off = tz_offset();
    time_t t = timegm(tm) - off;
    localtime_r(&t, tm);
    return t;
}

static const char *const wdays[] = { "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday" };
static const char *const months[] = { "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December" };

char *asctime(const struct tm *tm)
{
    static char buf[32];
    snprintf(buf, sizeof buf, "%.3s %.3s%3d %02d:%02d:%02d %d\n", tm->tm_wday >= 0 && tm->tm_wday < 7 ? wdays[tm->tm_wday] : "???",
             tm->tm_mon >= 0 && tm->tm_mon < 12 ? months[tm->tm_mon] : "???", tm->tm_mday, tm->tm_hour, tm->tm_min, tm->tm_sec, 1900 + tm->tm_year);
    return buf;
}
char *ctime(const time_t *t) { struct tm *tm = localtime(t); return tm ? asctime(tm) : 0; }

static int iso_week(const struct tm *tm, int *year)
{
    int y = tm->tm_year + 1900, wd = (tm->tm_wday + 6) % 7;            /* Monday = 0 */
    int w = (tm->tm_yday - wd + 10) / 7;
    #define LEAP(Y) (((Y) % 4 == 0 && (Y) % 100 != 0) || (Y) % 400 == 0)
    if (w < 1) { y--; w = (tm->tm_yday + (LEAP(y) ? 366 : 365) - wd + 10) / 7; }
    else if (w == 53) {
        int days_in_year = LEAP(y) ? 366 : 365;
        if (tm->tm_yday - wd + 3 >= days_in_year) { w = 1; y++; }
    }
    *year = y;
    return w;
}

size_t strftime(char *restrict s, size_t max, const char *restrict fmt, const struct tm *restrict tm)
{
    size_t n = 0;
    #define EMIT(...) do { char t_[96]; int k_ = snprintf(t_, sizeof t_, __VA_ARGS__); if (k_ < 0 || n + (size_t)k_ >= max) return 0; memcpy(s + n, t_, (size_t)k_); n += (size_t)k_; } while (0)
    #define SUB(f) do { size_t k_ = strftime(s + n, max - n, f, tm); if (!k_) return 0; n += k_; } while (0)
    if (!max) return 0;
    for (; *fmt; fmt++) {
        if (*fmt != '%') { if (n + 1 >= max) return 0; s[n++] = *fmt; continue; }
        fmt++;
        if (*fmt == 'E' || *fmt == 'O') fmt++;
        switch (*fmt) {
        case '%': EMIT("%%"); break;
        case 'a': EMIT("%.3s", wdays[(unsigned)tm->tm_wday % 7]); break;
        case 'A': EMIT("%s", wdays[(unsigned)tm->tm_wday % 7]); break;
        case 'b': case 'h': EMIT("%.3s", months[(unsigned)tm->tm_mon % 12]); break;
        case 'B': EMIT("%s", months[(unsigned)tm->tm_mon % 12]); break;
        case 'c': SUB("%a %b %e %H:%M:%S %Y"); break;
        case 'C': EMIT("%02d", (tm->tm_year + 1900) / 100); break;
        case 'd': EMIT("%02d", tm->tm_mday); break;
        case 'D': SUB("%m/%d/%y"); break;
        case 'e': EMIT("%2d", tm->tm_mday); break;
        case 'F': SUB("%Y-%m-%d"); break;
        case 'g': { int y; iso_week(tm, &y); EMIT("%02d", y % 100); break; }
        case 'G': { int y; iso_week(tm, &y); EMIT("%d", y); break; }
        case 'H': EMIT("%02d", tm->tm_hour); break;
        case 'I': EMIT("%02d", tm->tm_hour % 12 ? tm->tm_hour % 12 : 12); break;
        case 'j': EMIT("%03d", tm->tm_yday + 1); break;
        case 'm': EMIT("%02d", tm->tm_mon + 1); break;
        case 'M': EMIT("%02d", tm->tm_min); break;
        case 'n': EMIT("\n"); break;
        case 'p': EMIT("%s", tm->tm_hour >= 12 ? "PM" : "AM"); break;
        case 'r': SUB("%I:%M:%S %p"); break;
        case 'R': SUB("%H:%M"); break;
        case 's': { struct tm c = *tm; EMIT("%ld", (long)mktime(&c)); break; }
        case 'S': EMIT("%02d", tm->tm_sec); break;
        case 't': EMIT("\t"); break;
        case 'T': case 'X': SUB("%H:%M:%S"); break;
        case 'u': EMIT("%d", tm->tm_wday ? tm->tm_wday : 7); break;
        case 'U': EMIT("%02d", (tm->tm_yday + 7 - tm->tm_wday) / 7); break;
        case 'V': { int y; EMIT("%02d", iso_week(tm, &y)); break; }
        case 'w': EMIT("%d", tm->tm_wday); break;
        case 'W': EMIT("%02d", (tm->tm_yday + 7 - (tm->tm_wday + 6) % 7) / 7); break;
        case 'x': SUB("%m/%d/%y"); break;
        case 'y': EMIT("%02d", (tm->tm_year + 1900) % 100); break;
        case 'Y': EMIT("%d", tm->tm_year + 1900); break;
        case 'z': { long o = tm->tm_gmtoff; char sg = o < 0 ? '-' : '+'; if (o < 0) o = -o; EMIT("%c%02ld%02ld", sg, o / 3600, o % 3600 / 60); break; }
        case 'Z': EMIT("%s", tm->tm_zone ? tm->tm_zone : ""); break;
        default: return 0;
        }
    }
    s[n] = 0;
    return n;
}
