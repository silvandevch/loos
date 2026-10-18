/* Freestanding <string.h> for LoOS user-mode system DLLs (ntdll, kernel32, ...).
 * These DLLs sit below the C runtime, so they carry their own tiny memory and string routines.
 * Only used when building for the Windows target; host tests use the system one. */
#ifndef LOOS_CRT_STRING_H
#define LOOS_CRT_STRING_H
#include <stddef.h>
void *memcpy(void *d, const void *s, size_t n);
void *memmove(void *d, const void *s, size_t n);
void *memset(void *d, int c, size_t n);
int   memcmp(const void *a, const void *b, size_t n);
size_t strlen(const char *s);
int   strcmp(const char *a, const char *b);
char *strcpy(char *d, const char *s);
char *strncpy(char *d, const char *s, size_t n);
char *strchr(const char *s, int c);
char *strrchr(const char *s, int c);
#endif
