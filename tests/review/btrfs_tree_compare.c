/* Independent reviewer check for the btrfs read path (docs/reviews/): build a real filesystem with
 * mkfs.btrfs --rootdir from an arbitrary directory tree, then walk that tree with the host OS and verify
 * the LoOS reader returns the same names and the same bytes (random offsets and lengths, so extent and
 * node boundaries get crossed).
 *   btrfs_tree_compare <image> <source-dir> */
#include "btrfs.h"
#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

static struct { uint8_t *data; size_t size; } img;
static int rd(void *o, uint64_t off, void *b, size_t n)
{
    (void)o;
    if (off > img.size || n > img.size - (size_t)off) return -1;
    memcpy(b, img.data + off, n);
    return 0;
}

static struct loos_btrfs_mount M;
static int fails, files, dirs;
static uint32_t rng = 12345;
static uint32_t rnd(void) { rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5; return rng; }

struct names { char n[600][256]; int count; };
static int collect(void *o, const struct loos_btrfs_dirent *e)
{
    struct names *s = o;
    if (!strcmp(e->name, ".") || !strcmp(e->name, "..")) return 0;
    if (s->count < 600) strcpy(s->n[s->count++], e->name);
    return 0;
}

static void compare_file(const char *host, const char *vpath, off_t size)
{
    FILE *f = fopen(host, "rb");
    uint8_t *want = malloc(size ? (size_t)size : 1), *got;
    int iter;
    if (!f || (size && fread(want, 1, (size_t)size, f) != (size_t)size)) { printf("cannot read %s\n", host); fails++; if (f) fclose(f); free(want); return; }
    fclose(f);
    got = malloc((size_t)size + 8192);
    for (iter = 0; iter < 12; iter++) {
        size_t off = size ? rnd() % (size_t)(size) : 0, len = iter == 0 ? (size_t)size : 1 + rnd() % 70000, n = 0;
        int st;
        if (iter == 0) off = 0;
        if (iter == 1) { off = 0; len = 1; }
        st = loos_btrfs_read_file(&M, vpath, off, got, len, &n);
        if (st != LOOS_BTRFS_OK) { printf("FAIL read %s off=%zu len=%zu status=%d\n", vpath, off, len, st); fails++; break; }
        {
            size_t expect = off >= (size_t)size ? 0 : ((size_t)size - off < len ? (size_t)size - off : len);
            if (n != expect || (expect && memcmp(got, want + off, expect))) { printf("FAIL content %s off=%zu len=%zu got=%zu expect=%zu\n", vpath, off, len, n, expect); fails++; break; }
        }
    }
    files++;
    free(want); free(got);
}

static void walk(const char *host, const char *vpath)
{
    DIR *d = opendir(host);
    struct dirent *de;
    struct names listed = {0}, actual = {0};
    int i, j, st;
    if (!d) { printf("cannot open %s\n", host); fails++; return; }
    st = loos_btrfs_list_dir(&M, vpath, collect, &listed);
    if (st != LOOS_BTRFS_OK) { printf("FAIL list %s status=%d\n", vpath, st); fails++; closedir(d); return; }
    dirs++;
    while ((de = readdir(d))) if (strcmp(de->d_name, ".") && strcmp(de->d_name, "..") && actual.count < 600) strcpy(actual.n[actual.count++], de->d_name);
    closedir(d);
    if (listed.count != actual.count) { printf("FAIL %s: %d entries listed, %d on disk\n", vpath, listed.count, actual.count); fails++; }
    for (i = 0; i < actual.count; i++) {
        char h[2048], v[2048];
        struct stat sb;
        int found = 0;
        for (j = 0; j < listed.count; j++) if (!strcmp(listed.n[j], actual.n[i])) found = 1;
        if (!found) { printf("FAIL %s: '%s' missing from listing\n", vpath, actual.n[i]); fails++; }
        snprintf(h, sizeof h, "%s/%s", host, actual.n[i]);
        snprintf(v, sizeof v, "%s%s%s", vpath, strcmp(vpath, "/") ? "/" : "", actual.n[i]);
        if (lstat(h, &sb)) continue;
        if (S_ISDIR(sb.st_mode)) walk(h, v);
        else if (S_ISREG(sb.st_mode)) compare_file(h, v, sb.st_size);
    }
}

int main(int argc, char **argv)
{
    FILE *f;
    long l;
    void *ns;
    struct loos_btrfs_io io = { rd, NULL };
    int st;
    if (argc != 3) { fprintf(stderr, "usage: %s image dir\n", argv[0]); return 2; }
    f = fopen(argv[1], "rb");
    if (!f) { perror(argv[1]); return 2; }
    fseek(f, 0, SEEK_END); l = ftell(f); fseek(f, 0, SEEK_SET);
    img.data = malloc((size_t)l); img.size = (size_t)l;
    if (fread(img.data, 1, (size_t)l, f) != (size_t)l) return 2;
    fclose(f);
    ns = malloc(LOOS_BTRFS_NODE_STORAGE_SIZE);
    st = loos_btrfs_mount(&M, &io, ns, LOOS_BTRFS_NODE_STORAGE_SIZE, true);
    if (st != LOOS_BTRFS_OK) { printf("mount failed: %d\n", st); return 1; }
    walk(argv[2], "/");
    printf("btrfs_tree_compare: %d directories, %d files, %d failures\n", dirs, files, fails);
    return fails != 0;
}
