/* err(3)/warn(3) and assertion failures. */
#include "prism.h"

static FILE *err_file;
void err_set_file(void *fp) { err_file = fp; }

static void vwarn_common(int code, int with_errno, const char *fmt, va_list ap) {
    FILE *f = err_file ? err_file : stderr;
    fprintf(f, "%s: ", getprogname());
    if (fmt) {
        vfprintf(f, fmt, ap);
        if (with_errno)
            fputs(": ", f);
    }
    if (with_errno)
        fputs(strerror(code), f);
    fputc('\n', f);
}

void vwarn(const char *fmt, va_list ap) { vwarn_common(errno, 1, fmt, ap); }
void vwarnc(int code, const char *fmt, va_list ap) { vwarn_common(code, 1, fmt, ap); }
void vwarnx(const char *fmt, va_list ap) { vwarn_common(0, 0, fmt, ap); }
void warn(const char *fmt, ...) { va_list ap; va_start(ap, fmt); vwarn(fmt, ap); va_end(ap); }
void warnc(int code, const char *fmt, ...) { va_list ap; va_start(ap, fmt); vwarnc(code, fmt, ap); va_end(ap); }
void warnx(const char *fmt, ...) { va_list ap; va_start(ap, fmt); vwarnx(fmt, ap); va_end(ap); }

__attribute__((noreturn)) void verr(int eval, const char *fmt, va_list ap) { vwarn(fmt, ap); exit(eval); }
__attribute__((noreturn)) void verrc(int eval, int code, const char *fmt, va_list ap) { vwarnc(code, fmt, ap); exit(eval); }
__attribute__((noreturn)) void verrx(int eval, const char *fmt, va_list ap) { vwarnx(fmt, ap); exit(eval); }
__attribute__((noreturn)) void err(int eval, const char *fmt, ...) { va_list ap; va_start(ap, fmt); verr(eval, fmt, ap); }
__attribute__((noreturn)) void errc(int eval, int code, const char *fmt, ...) { va_list ap; va_start(ap, fmt); verrc(eval, code, fmt, ap); }
__attribute__((noreturn)) void errx(int eval, const char *fmt, ...) { va_list ap; va_start(ap, fmt); verrx(eval, fmt, ap); }
