#ifndef _SYS_TIME_H
#define _SYS_TIME_H
#include <_prism_cdefs.h>
#include <time.h>
struct timezone { int tz_minuteswest, tz_dsttime; };
__BEGIN_DECLS
int gettimeofday(struct timeval *, void *); int utimes(const char *, const struct timeval *);
int futimes(int, const struct timeval *);
__END_DECLS
#endif
