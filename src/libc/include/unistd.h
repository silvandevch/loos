#ifndef _UNISTD_H
#define _UNISTD_H
#include "features.h"
#include <stddef.h>
#include "sys/types.h"
__LOOS_BEGIN
#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
ssize_t read(int, void *, size_t);
ssize_t write(int, const void *, size_t);
int close(int);
long lseek(int, long, int);
int unlink(const char *);
int isatty(int);
int getpid(void);
int getppid(void);
unsigned getuid(void); unsigned geteuid(void); unsigned getgid(void); unsigned getegid(void);
char *getcwd(char *, size_t);
int chdir(const char *);
int rmdir(const char *);
int access(const char *, int);
int dup(int); int dup2(int, int);
int pipe(int[2]);
int fsync(int);
int ftruncate(int, long);
int truncate(const char *, long);
int symlink(const char *, const char *);
long readlink(const char *restrict, char *restrict, size_t);
int link(const char *, const char *);
pid_t fork(void);
int execve(const char *, char *const[], char *const[]);
int execv(const char *, char *const[]);
int execvp(const char *, char *const[]);
int usleep(unsigned);
long sysconf(int);
int gethostname(char *, size_t);
#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4
#define _SC_PAGESIZE 30
#define _SC_NPROCESSORS_ONLN 84
#define _SC_CLK_TCK 2
extern char **environ;
__LOOS_NORETURN void _exit(int);
__LOOS_END
#endif
