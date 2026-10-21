#ifndef _SYS_WAIT_H
#define _SYS_WAIT_H
#include "types.h"
#define WNOHANG 1
#define WEXITSTATUS(s) (((s) >> 8) & 0xff)
#define WTERMSIG(s) ((s) & 0x7f)
#define WIFEXITED(s) (WTERMSIG(s) == 0)
#define WIFSIGNALED(s) (((signed char)(((s) & 0x7f) + 1) >> 1) > 0)
pid_t waitpid(pid_t, int *, int);
pid_t wait(int *);
#endif
