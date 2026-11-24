#ifndef _UNISTD_H
#define _UNISTD_H
#include <_prism_cdefs.h>
#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4
#define _SC_PAGESIZE 29
#define _SC_PAGE_SIZE _SC_PAGESIZE
#define _SC_NPROCESSORS_CONF 57
#define _SC_NPROCESSORS_ONLN 58
#define _PC_NAME_MAX 4
#define _PC_PATH_MAX 5
__BEGIN_DECLS
int access(const char *, int); int chdir(const char *); int close(int); int dup(int); int dup2(int, int);
char *getcwd(char *, size_t); pid_t getpid(void); pid_t getppid(void); uid_t getuid(void); uid_t geteuid(void);
int isatty(int); int link(const char *, const char *); off_t lseek(int, off_t, int); long pathconf(const char *, int);
long fpathconf(int, int); int pipe(int[2]); ssize_t read(int, void *, size_t); ssize_t pread(int, void *, size_t, off_t);
ssize_t readlink(const char *, char *, size_t); int rmdir(const char *); int symlink(const char *, const char *);
long sysconf(int); int truncate(const char *, off_t); int ftruncate(int, off_t); int unlink(const char *);
ssize_t write(int, const void *, size_t); ssize_t pwrite(int, const void *, size_t, off_t);
unsigned sleep(unsigned); int usleep(useconds_t); int fsync(int); int getpagesize(void);
int gethostname(char *, size_t); void _exit(int) __attribute__((__noreturn__)); pid_t fork(void);
int execv(const char *, char *const[]); int execve(const char *, char *const[], char *const[]);
int execvp(const char *, char *const[]); int getentropy(void *, size_t);
int linkat(int, const char *, int, const char *, int); int unlinkat(int, const char *, int);
ssize_t readlinkat(int, const char *, char *, size_t); int symlinkat(const char *, int, const char *);
int faccessat(int, const char *, int, int);
__END_DECLS
#endif
