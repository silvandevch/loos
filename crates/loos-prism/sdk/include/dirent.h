#ifndef _DIRENT_H
#define _DIRENT_H
#include <_prism_cdefs.h>
struct dirent { uint64_t d_ino, d_seekoff; uint16_t d_reclen, d_namlen; uint8_t d_type; char d_name[1024]; };
typedef struct _prism_DIR DIR;
#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 4
#define DT_BLK 6
#define DT_REG 8
#define DT_LNK 10
#define DT_SOCK 12
#define DT_WHT 14
__BEGIN_DECLS
DIR *opendir(const char *) __DARWIN_INODE64(opendir);
DIR *fdopendir(int) __DARWIN_INODE64(fdopendir);
struct dirent *readdir(DIR *) __DARWIN_INODE64(readdir);
int readdir_r(DIR *, struct dirent *, struct dirent **) __DARWIN_INODE64(readdir_r);
void rewinddir(DIR *) __DARWIN_INODE64(rewinddir);
int closedir(DIR *); int dirfd(DIR *);
__END_DECLS
#endif
