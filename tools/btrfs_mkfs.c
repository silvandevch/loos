#define _POSIX_C_SOURCE 200809L
/* Host wrapper around loos_btrfs_mkfs: btrfs_mkfs IMAGE [SIZE_MIB] [LABEL] -> formats (or creates) IMAGE. Used by the tests. */
#include "btrfs.h"
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static int wr(void *fd, uint64_t off, const void *d, size_t n)
{
    return pwrite(*(int *)fd, d, n, (off_t)off) == (ssize_t)n ? 0 : -1;
}

int main(int argc, char **argv)
{
    if (argc < 2) { fprintf(stderr, "usage: btrfs_mkfs IMAGE [SIZE_MIB] [LABEL]\n"); return 2; }
    int fd = open(argv[1], O_RDWR | O_CREAT, 0644);
    if (fd < 0) { perror("open"); return 1; }
    uint64_t size = argc > 2 ? strtoull(argv[2], 0, 10) << 20 : 0;
    if (size && ftruncate(fd, (off_t)size) != 0) { perror("ftruncate"); return 1; }
    if (!size) { struct stat st; fstat(fd, &st); size = (uint64_t)st.st_size; }
    struct loos_btrfs_mkfs_params p;
    memset(&p, 0, sizeof p);
    p.device_bytes = size; p.label = argc > 3 ? argv[3] : "LoOS"; p.unix_time = (uint64_t)time(0);
    FILE *r = fopen("/dev/urandom", "rb");
    if (!r || fread(p.fsid, 1, 16, r) != 16 || fread(p.dev_uuid, 1, 16, r) != 16 || fread(p.chunk_tree_uuid, 1, 16, r) != 16 || fread(p.subvol_uuid, 1, 16, r) != 16) return 1;
    fclose(r);
    int rc = loos_btrfs_mkfs(&p, wr, &fd);
    close(fd);
    if (rc) fprintf(stderr, "mkfs failed: %d\n", rc);
    return rc != 0;
}
