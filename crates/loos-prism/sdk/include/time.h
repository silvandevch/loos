#ifndef _TIME_H
#define _TIME_H
#include <_prism_cdefs.h>
struct tm { int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst; long tm_gmtoff; char *tm_zone; };
#define CLOCKS_PER_SEC 1000000
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC_RAW 4
#define CLOCK_MONOTONIC_RAW_APPROX 5
#define CLOCK_MONOTONIC 6
#define CLOCK_UPTIME_RAW 8
#define CLOCK_PROCESS_CPUTIME_ID 12
#define CLOCK_THREAD_CPUTIME_ID 16
#define TIME_UTC 1
__BEGIN_DECLS
char *asctime(const struct tm *); clock_t clock(void); char *ctime(const time_t *); double difftime(time_t, time_t);
struct tm *gmtime(const time_t *); struct tm *localtime(const time_t *); time_t mktime(struct tm *);
size_t strftime(char *, size_t, const char *, const struct tm *); time_t time(time_t *);
struct tm *gmtime_r(const time_t *, struct tm *); struct tm *localtime_r(const time_t *, struct tm *);
int nanosleep(const struct timespec *, struct timespec *); int clock_gettime(clockid_t, struct timespec *);
int clock_getres(clockid_t, struct timespec *); int timespec_get(struct timespec *, int);
uint64_t clock_gettime_nsec_np(clockid_t); char *strptime(const char *, const char *, struct tm *);
void tzset(void); time_t timegm(struct tm *);
__END_DECLS
#include <xlocale.h>
#endif
