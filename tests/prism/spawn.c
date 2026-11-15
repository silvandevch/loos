/* Process creation through Prism: fork + execl of a Mach-O, posix_spawn with a pipe, execvp errors. */
int printf(const char *, ...);
int fork(void);
int execl(const char *, const char *, ...);
int execvp(const char *, char *const[]);
void _exit(int) __attribute__((noreturn));
int waitpid(int, int *, int);
int pipe(int[2]);
long read(int, void *, unsigned long);
int close(int);
int *__error(void);
typedef void *posix_spawn_file_actions_t;
int posix_spawn_file_actions_init(posix_spawn_file_actions_t *);
int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *, int, int);
int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *, int);
int posix_spawnp(int *, const char *, const posix_spawn_file_actions_t *, const void *, char *const[], char *const[]);
int posix_spawnattr_init(void **); int posix_spawnattr_setflags(void **, short);
extern char **environ;

int main(int argc, char **argv) {
    const char *other = argc > 1 ? argv[1] : "/bin/hello_x86_64";
    int st = -1, pid = fork();
    if (pid == 0) {
        execl(other, "hello", (char *)0);
        _exit(99);
    }
    waitpid(pid, &st, 0);
    int fds[2]; pipe(fds);
    posix_spawn_file_actions_t fa; posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, fds[1], 1);
    posix_spawn_file_actions_addclose(&fa, fds[0]);
    char *args[] = {"hello", 0};
    int sp = -1, r = posix_spawnp(&sp, other, &fa, 0, args, environ);
    close(fds[1]);
    char buf[4096]; long n, tot = 0;
    while ((n = read(fds[0], buf + tot, sizeof buf - 1 - tot)) > 0) tot += n;
    buf[tot] = 0;
    int st2 = -1; waitpid(sp, &st2, 0);
    char *none[] = {"no-such-tool", 0};
    int e = execvp("no-such-tool", none);
    int ee = *__error(), sp3 = 0;
    void *attr; posix_spawnattr_init(&attr); posix_spawnattr_setflags(&attr, 2 /* SETPGROUP */);
    int r3 = posix_spawnp(&sp3, "no-such-tool", 0, &attr, none, environ);
    printf("spawn: fork=%d exit=%d spawn=%d piped=%d exit2=%d execvp=%d/%d missing=%d\n", pid > 0, st >> 8, r, tot > 100, st2 >> 8, e, ee, r3);
    return 0;
}
