#include <_prism_cdefs.h>
#define LOG_ERR 3
#define LOG_WARNING 4
#define LOG_INFO 6
__BEGIN_DECLS
void syslog(int, const char *, ...); void openlog(const char *, int, int); void closelog(void);
__END_DECLS
