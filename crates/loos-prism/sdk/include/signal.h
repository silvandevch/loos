#ifndef _SIGNAL_H
#define _SIGNAL_H
#include <_prism_cdefs.h>
typedef int sig_atomic_t;
#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)
#define SIG_ERR ((void (*)(int))-1)
#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGILL 4
#define SIGTRAP 5
#define SIGABRT 6
#define SIGFPE 8
#define SIGKILL 9
#define SIGBUS 10
#define SIGSEGV 11
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 20
#define SIGUSR1 30
#define SIGUSR2 31
__BEGIN_DECLS
void (*signal(int, void (*)(int)))(int); int raise(int); int kill(pid_t, int);
int sigemptyset(sigset_t *); int sigfillset(sigset_t *); int sigaddset(sigset_t *, int);
int pthread_sigmask(int, const sigset_t *, sigset_t *);
__END_DECLS
#endif
