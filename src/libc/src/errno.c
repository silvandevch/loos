#include <errno.h>
static int libc_errno;
int *__errno_location(void) { return &libc_errno; }
