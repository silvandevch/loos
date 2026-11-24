#ifndef _STDIO_H
#define _STDIO_H
#include <_prism_cdefs.h>
#include <stdarg.h>
typedef long long fpos_t;
struct __sbuf { unsigned char *_base; int _size; };
typedef struct __sFILE {
    unsigned char *_p; int _r; int _w; short _flags; short _file; struct __sbuf _bf; int _lbfsize;
    void *_cookie; int (*_close)(void *); int (*_read)(void *, char *, int);
    fpos_t (*_seek)(void *, fpos_t, int); int (*_write)(void *, const char *, int);
    struct __sbuf _ub; void *_extra; int _ur; unsigned char _ubuf[3]; unsigned char _nbuf[1];
    struct __sbuf _lb; int _blksize; fpos_t _offset;
} FILE;
__BEGIN_DECLS
extern FILE *__stdinp, *__stdoutp, *__stderrp;
__END_DECLS
#define stdin __stdinp
#define stdout __stdoutp
#define stderr __stderrp
#define EOF (-1)
#define BUFSIZ 1024
#define FILENAME_MAX 1024
#define FOPEN_MAX 20
#define L_tmpnam 1024
#define TMP_MAX 308915776
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
__BEGIN_DECLS
void clearerr(FILE *); int fclose(FILE *); int feof(FILE *); int ferror(FILE *); int fflush(FILE *);
int fgetc(FILE *); int fgetpos(FILE *, fpos_t *); char *fgets(char *, int, FILE *);
FILE *fopen(const char *, const char *); FILE *fdopen(int, const char *);
FILE *freopen(const char *, const char *, FILE *); int fileno(FILE *);
int fprintf(FILE *, const char *, ...); int fputc(int, FILE *); int fputs(const char *, FILE *);
size_t fread(void *, size_t, size_t, FILE *); int fscanf(FILE *, const char *, ...);
int fseek(FILE *, long, int); int fseeko(FILE *, off_t, int); int fsetpos(FILE *, const fpos_t *);
long ftell(FILE *); off_t ftello(FILE *); size_t fwrite(const void *, size_t, size_t, FILE *);
int getc(FILE *); int getchar(void); void perror(const char *); int printf(const char *, ...);
int putc(int, FILE *); int putchar(int); int puts(const char *); int remove(const char *);
int rename(const char *, const char *); void rewind(FILE *); int scanf(const char *, ...);
void setbuf(FILE *, char *); int setvbuf(FILE *, char *, int, size_t);
int sprintf(char *, const char *, ...); int snprintf(char *, size_t, const char *, ...);
int sscanf(const char *, const char *, ...); FILE *tmpfile(void); char *tmpnam(char *);
int ungetc(int, FILE *); int vfprintf(FILE *, const char *, va_list); int vprintf(const char *, va_list);
int vsprintf(char *, const char *, va_list); int vsnprintf(char *, size_t, const char *, va_list);
int vfscanf(FILE *, const char *, va_list); int vscanf(const char *, va_list);
int vsscanf(const char *, const char *, va_list); int asprintf(char **, const char *, ...);
int vasprintf(char **, const char *, va_list); int dprintf(int, const char *, ...);
ssize_t getline(char **, size_t *, FILE *); ssize_t getdelim(char **, size_t *, int, FILE *);
void flockfile(FILE *); void funlockfile(FILE *); int getc_unlocked(FILE *); int putc_unlocked(int, FILE *);
FILE *popen(const char *, const char *); int pclose(FILE *);
__END_DECLS
#include <xlocale.h>
#endif
