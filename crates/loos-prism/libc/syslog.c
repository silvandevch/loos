/* syslog (variadic, so C per docs/LANGUAGE.md §C.6). LoOS has no log daemon: a message reaches
 * stderr when openlog asked for LOG_PERROR or LOG_CONS, and is dropped otherwise. %m is the
 * text of errno, as syslog(3) specifies. */
#include "prism.h"

int getpid(void);
static const char *log_ident;
static int log_opt, log_mask = 0xff;

void openlog(const char *ident, int opt, int facility) {
    (void)facility;
    log_ident = ident;
    log_opt = opt;
}
void closelog(void) { log_ident = NULL; log_opt = 0; }
int setlogmask(int mask) {
    int old = log_mask;
    if (mask) log_mask = mask;
    return old;
}

void vsyslog(int pri, const char *fmt, va_list ap) {
    int saved = *__error();
    if (!(log_mask & (1 << (pri & 7))) || !(log_opt & (0x20 | 0x02))) return; /* LOG_PERROR | LOG_CONS */
    /* Expand %m first, then format. */
    char f[1024]; size_t o = 0;
    for (const char *p = fmt; *p && o < sizeof f - 1; p++) {
        if (p[0] == '%' && p[1] == 'm') {
            const char *e = strerror(saved);
            while (*e && o < sizeof f - 1) f[o++] = *e++;
            p++;
        } else if (p[0] == '%' && p[1] == '%') {
            if (o < sizeof f - 2) { f[o++] = '%'; f[o++] = '%'; }
            p++;
        } else {
            f[o++] = *p;
        }
    }
    f[o] = 0;
    char msg[2048];
    const char *id = log_ident ? log_ident : getprogname();
    int n = (log_opt & 1) /* LOG_PID */ ? snprintf(msg, sizeof msg, "%s[%d]: ", id, getpid()) : snprintf(msg, sizeof msg, "%s: ", id);
    if (n < 0 || n >= (int)sizeof msg) n = 0;
    int m = vsnprintf(msg + n, sizeof msg - n, f, ap);
    if (m < 0) return;
    n += m;
    if (n > (int)sizeof msg - 2) n = sizeof msg - 2;
    if (n == 0 || msg[n - 1] != '\n') msg[n++] = '\n';
    write(2, msg, (size_t)n);
}

void syslog(int pri, const char *fmt, ...) {
    va_list ap; va_start(ap, fmt);
    vsyslog(pri, fmt, ap);
    va_end(ap);
}
/* The SDK names syslog's symbol syslog$DARWIN_EXTSN. */
__asm__(".globl _syslog$DARWIN_EXTSN\n.set _syslog$DARWIN_EXTSN, _syslog\n");
