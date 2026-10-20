/* Memory and string routines for the system DLLs: the LoOS libc's pure subset, compiled for the Windows target.
 * -fno-builtin keeps the compiler from turning the loops back into calls to themselves. */
#define LOOS_LIBC_PURE 1
#include "../../libc/src/string.c"
