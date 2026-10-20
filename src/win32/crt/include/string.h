/* Freestanding <string.h> for the LoOS user-mode system DLLs (ntdll, kernel32, ...): they sit below the C
 * runtime, so they carry the libc's pure string routines (crt/string.c builds libc/src/string.c). */
#include "../../../libc/include/string.h"
