/* Rust btrfs reader (loos-btrfs-ops) driven from C against an image made by mkfs.btrfs: tests/rust/run_btrfs_ffi.py */
#include "loos_rust.h"
#include <stdio.h>
#include <string.h>

static FILE *img;
static int rd(void *o, uint64_t off, void *b, size_t n) { (void)o; return fseek(img, (long)off, SEEK_SET) || fread(b, 1, n, img) != n; }
static int fails;
#define CHECK(c) do { if (!(c)) { fails++; printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); } } while (0)

int main(int argc, char **argv)
{
    if (argc != 2 || !(img = fopen(argv[1], "rb"))) return 2;
    fseek(img, 0, SEEK_END);
    loos_btrfs *b = loos_btrfs_open(rd, NULL, (uint64_t)ftell(img));
    CHECK(b != NULL);
    if (!b) return 1;
    char list[256]; uint64_t size; bool dir; char buf[64];
    CHECK(loos_btrfs_list(b, "/", list, sizeof list) > 0 && strstr(list, "hello.txt\n") && strstr(list, "sub/\n"));
    CHECK(loos_btrfs_stat(b, "/hello.txt", &size, &dir) == 0 && size == 12 && !dir);
    CHECK(loos_btrfs_stat(b, "/sub", &size, &dir) == 0 && dir);
    CHECK(loos_btrfs_stat(b, "/sub/big.bin", &size, &dir) == 0 && size == 300000);
    CHECK(loos_btrfs_read(b, "/hello.txt", 0, (uint8_t *)buf, sizeof buf) == 12 && !memcmp(buf, "hello btrfs\n", 12));
    CHECK(loos_btrfs_read(b, "/hello.txt", 6, (uint8_t *)buf, 5) == 5 && !memcmp(buf, "btrfs", 5));
    CHECK(loos_btrfs_stat(b, "/nope", &size, &dir) == -1 && loos_btrfs_read(b, "/nope", 0, (uint8_t *)buf, 1) == -1);
    loos_btrfs_close(b);
    printf("ffi_btrfs_test: %d failed\n", fails);
    return fails != 0;
}
