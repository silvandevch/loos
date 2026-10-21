#ifndef _DIRENT_H
#define _DIRENT_H
#include "features.h"
#include "sys/types.h"
__LOOS_BEGIN
struct dirent { ino_t d_ino; off_t d_off; unsigned short d_reclen; unsigned char d_type; char d_name[256]; };
typedef struct __loos_dir DIR;
#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 4
#define DT_BLK 6
#define DT_REG 8
#define DT_LNK 10
#define DT_SOCK 12
DIR *opendir(const char *);
struct dirent *readdir(DIR *);
int closedir(DIR *);
void rewinddir(DIR *);
int dirfd(DIR *);
__LOOS_END
#endif
