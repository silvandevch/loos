#ifndef _STDIO_H
#define _STDIO_H
#include "features.h"
#include <stddef.h>
#include <stdarg.h>
__LOOS_BEGIN
typedef struct __loos_file FILE;
typedef long long fpos_t;
typedef long off_t;
typedef long ssize_t;
#define EOF (-1)
#define BUFSIZ 8192
#define FILENAME_MAX 4096
#define FOPEN_MAX 64
#define TMP_MAX 238328
#define L_tmpnam 20
#define P_tmpdir "/tmp"
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
extern FILE *stdin, *stdout, *stderr;
#define stdin stdin
#define stdout stdout
#define stderr stderr
FILE *fopen(const char *restrict, const char *restrict);
FILE *freopen(const char *restrict, const char *restrict, FILE *restrict);
FILE *fdopen(int, const char *);
FILE *tmpfile(void);
char *tmpnam(char *);
int fclose(FILE *);
int fflush(FILE *);
int setvbuf(FILE *restrict, char *restrict, int, size_t);
void setbuf(FILE *restrict, char *restrict);
int fprintf(FILE *restrict, const char *restrict, ...) __LOOS_PRINTF(2, 3);
int printf(const char *restrict, ...) __LOOS_PRINTF(1, 2);
int sprintf(char *restrict, const char *restrict, ...) __LOOS_PRINTF(2, 3);
int snprintf(char *restrict, size_t, const char *restrict, ...) __LOOS_PRINTF(3, 4);
int dprintf(int, const char *restrict, ...) __LOOS_PRINTF(2, 3);
int asprintf(char **, const char *restrict, ...) __LOOS_PRINTF(2, 3);
int vfprintf(FILE *restrict, const char *restrict, va_list);
int vprintf(const char *restrict, va_list);
int vsprintf(char *restrict, const char *restrict, va_list);
int vsnprintf(char *restrict, size_t, const char *restrict, va_list);
int vdprintf(int, const char *restrict, va_list);
int vasprintf(char **, const char *restrict, va_list);
int fscanf(FILE *restrict, const char *restrict, ...);
int scanf(const char *restrict, ...);
int sscanf(const char *restrict, const char *restrict, ...);
int vfscanf(FILE *restrict, const char *restrict, va_list);
int vscanf(const char *restrict, va_list);
int vsscanf(const char *restrict, const char *restrict, va_list);
int fgetc(FILE *); int getc(FILE *); int getchar(void);
int fputc(int, FILE *); int putc(int, FILE *); int putchar(int);
char *fgets(char *restrict, int, FILE *restrict);
int fputs(const char *restrict, FILE *restrict);
int puts(const char *);
int ungetc(int, FILE *);
size_t fread(void *restrict, size_t, size_t, FILE *restrict);
size_t fwrite(const void *restrict, size_t, size_t, FILE *restrict);
int fseek(FILE *, long, int);
long ftell(FILE *);
int fseeko(FILE *, off_t, int);
off_t ftello(FILE *);
void rewind(FILE *);
int fgetpos(FILE *restrict, fpos_t *restrict);
int fsetpos(FILE *, const fpos_t *);
void clearerr(FILE *);
int feof(FILE *);
int ferror(FILE *);
int fileno(FILE *);
void perror(const char *);
int remove(const char *);
int rename(const char *, const char *);
ssize_t getline(char **restrict, size_t *restrict, FILE *restrict);
ssize_t getdelim(char **restrict, size_t *restrict, int, FILE *restrict);
FILE *fmemopen(void *, size_t, const char *);
__LOOS_END
#endif
