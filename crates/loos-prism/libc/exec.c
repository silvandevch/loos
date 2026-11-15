/* execl family (variadic, so C per docs/LANGUAGE.md §C.6): collect the arguments, then execv*. */
#include "prism.h"

int execve(const char *, char *const[], char *const[]);
int execv(const char *, char *const[]);
int execvp(const char *, char *const[]);

#define MAXARGS 1024

static int collect(const char *arg0, va_list ap, char **argv) {
    int n = 0;
    argv[n++] = (char *)arg0;
    while (arg0 && n < MAXARGS - 1 && (argv[n] = va_arg(ap, char *)) != NULL) n++;
    argv[n] = NULL;
    if (n >= MAXARGS - 1) { *__error() = 7; return -1; } /* E2BIG */
    return n;
}

int execl(const char *path, const char *arg0, ...) {
    char *argv[MAXARGS]; va_list ap; va_start(ap, arg0);
    int n = collect(arg0, ap, argv); va_end(ap);
    return n < 0 ? -1 : execv(path, argv);
}
int execlp(const char *file, const char *arg0, ...) {
    char *argv[MAXARGS]; va_list ap; va_start(ap, arg0);
    int n = collect(arg0, ap, argv); va_end(ap);
    return n < 0 ? -1 : execvp(file, argv);
}
int execle(const char *path, const char *arg0, ...) {
    char *argv[MAXARGS]; va_list ap; va_start(ap, arg0);
    int n = collect(arg0, ap, argv);
    char **envp = va_arg(ap, char **); va_end(ap);
    return n < 0 ? -1 : execve(path, argv, envp);
}
