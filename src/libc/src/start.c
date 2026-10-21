/* Process start-up: _start (arch/x86_64.S) hands us the initial stack. */
#include "libc.h"
#include <stdlib.h>

extern int main(int, char **, char **);
char **environ;
static char *progname_default = "";
char *program_invocation_name;

void __libc_init_env(char **envp) { environ = envp; }

__attribute__((used, noreturn)) void __libc_start_main(long *sp)
{
    int argc = (int)sp[0];
    char **argv = (char **)(sp + 1);
    char **envp = argv + argc + 1;
    program_invocation_name = argc ? argv[0] : progname_default;
    __libc_init_env(envp);
    __libc_init_stdio();
    exit(main(argc, argv, envp));
}
