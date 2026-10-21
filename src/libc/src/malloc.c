/* Boundary-tag allocator. Layout follows the classic scheme: a chunk is [prev_size|size|payload...]; the
 * low bits of `size` flag PREV_INUSE and MMAPPED; a free chunk's footer lives in the next chunk's prev_size.
 * Small requests are carved from 1 MiB+ regions with an address-ordered-by-insertion free list (first fit,
 * immediate coalescing); requests >= MMAP_THRESHOLD get their own mapping. Single-threaded, like the rest
 * of this first-generation libc. */
#include "libc.h"
#include <errno.h>
#include <stdlib.h>
#include <string.h>

#define PREV_INUSE 1UL
#define MMAPPED 2UL
#define SENTINEL 4UL
#define FLAGS 7UL
#define HDR 16UL
#define MIN_CHUNK 32UL
#define REGION_SIZE (1UL << 20)
#define MMAP_THRESHOLD (256UL << 10)

typedef struct chunk { size_t prev_size, size; struct chunk *fd, *bk; } chunk;   /* fd/bk only while free */

static chunk free_head = { 0, 0, &free_head, &free_head };

/* Fast bins: exact 16-byte size classes up to 512 bytes. Freed small chunks are parked here still marked
 * in-use (no coalescing) and reused LIFO; they are folded back into the main list when it cannot satisfy a
 * request, so fragmentation is bounded. */
#define FAST_MAX 512UL
#define NFAST (FAST_MAX / 16 + 1)
static chunk *fast_bin[NFAST];

#define CSIZE(c) ((c)->size & ~FLAGS)
#define NEXT(c) ((chunk *)((char *)(c) + CSIZE(c)))
#define TOCHUNK(p) ((chunk *)((char *)(p) - HDR))
#define TOMEM(c) ((void *)((char *)(c) + HDR))

static void unlink_chunk(chunk *c) { c->bk->fd = c->fd; c->fd->bk = c->bk; }
static void link_chunk(chunk *c) { c->fd = free_head.fd; c->bk = &free_head; free_head.fd->bk = c; free_head.fd = c; }

/* mark chunk c (size already stored) free: write footer, clear next's PREV_INUSE, link */
static void make_free(chunk *c, size_t size, size_t prev_flag)
{
    c->size = size | prev_flag;
    chunk *n = (chunk *)((char *)c + size);
    n->prev_size = size;
    n->size &= ~PREV_INUSE;
    link_chunk(c);
}

static int grow(size_t need)
{
    size_t bytes = need + 2 * HDR;
    if (bytes < REGION_SIZE) bytes = REGION_SIZE;
    bytes = (bytes + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);
    char *base = __os_map(bytes);
    if (!base) return 0;
    chunk *end = (chunk *)(base + bytes - HDR);
    end->prev_size = 0;
    end->size = HDR | SENTINEL;                      /* PREV_INUSE cleared by make_free */
    make_free((chunk *)base, bytes - HDR, PREV_INUSE);
    return 1;
}

static size_t request_size(size_t n)
{
    size_t s = n + 8;
    if (s < MIN_CHUNK) s = MIN_CHUNK;
    return (s + 15) & ~15UL;
}

static void *alloc_mapped(size_t n, size_t align)
{
    size_t total = n + HDR + (align > HDR ? align : 0) + HDR;
    total = (total + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);
    char *base = __os_map(total);
    if (!base) return 0;
    char *mem = base + HDR;
    if (align > HDR) mem = (char *)(((uintptr_t)mem + align - 1) & ~(uintptr_t)(align - 1));
    chunk *c = TOCHUNK(mem);
    c->prev_size = (size_t)((char *)c - base);       /* lead gap: base = c - prev_size */
    c->size = total | MMAPPED;
    return mem;
}

static void free_chunk(chunk *c);

static void consolidate_fast(void)
{
    for (size_t i = 0; i < NFAST; i++) {
        chunk *c = fast_bin[i];
        fast_bin[i] = 0;
        while (c) { chunk *nx = c->fd; free_chunk(c); c = nx; }
    }
}

void *malloc(size_t n)
{
    if (n > PTRDIFF_MAX - 4 * PAGE_SIZE) { errno = ENOMEM; return 0; }
    size_t need = request_size(n);
    if (need >= MMAP_THRESHOLD) {
        void *p = alloc_mapped(n, 0);
        if (!p) errno = ENOMEM;
        return p;
    }
    if (need <= FAST_MAX && fast_bin[need / 16]) {
        chunk *c = fast_bin[need / 16];
        fast_bin[need / 16] = c->fd;
        return TOMEM(c);
    }
    for (int pass = 0; pass < 3; pass++) {
        if (pass == 1) consolidate_fast();
        if (pass == 2 && !grow(need)) break;
        for (chunk *c = free_head.fd; c != &free_head; c = c->fd) {
            size_t sz = CSIZE(c);
            if (sz < need) continue;
            unlink_chunk(c);
            if (sz - need >= MIN_CHUNK) {
                chunk *rest = (chunk *)((char *)c + need);
                c->size = need | (c->size & PREV_INUSE);
                make_free(rest, sz - need, PREV_INUSE);
            } else {
                NEXT(c)->size |= PREV_INUSE;
            }
            return TOMEM(c);
        }
    }
    errno = ENOMEM;
    return 0;
}

void free(void *p)
{
    if (!p) return;
    chunk *c = TOCHUNK(p);
    if (c->size & MMAPPED) { __os_unmap((char *)c - c->prev_size, CSIZE(c)); return; }
    if (CSIZE(c) <= FAST_MAX) { size_t i = CSIZE(c) / 16; c->fd = fast_bin[i]; fast_bin[i] = c; return; }
    free_chunk(c);
}

static void free_chunk(chunk *c)
{
    size_t size = CSIZE(c);
    size_t prev_flag = c->size & PREV_INUSE;
    if (!prev_flag) {
        chunk *pv = (chunk *)((char *)c - c->prev_size);
        unlink_chunk(pv);
        size += CSIZE(pv);
        prev_flag = pv->size & PREV_INUSE;
        c = pv;
    }
    chunk *n = (chunk *)((char *)c + size);
    if (!(n->size & SENTINEL) && !(NEXT(n)->size & PREV_INUSE) ) {
        unlink_chunk(n);
        size += CSIZE(n);
    }
    make_free(c, size, prev_flag);
}

size_t malloc_usable_size(void *p)
{
    if (!p) return 0;
    chunk *c = TOCHUNK(p);
    if (c->size & MMAPPED) return (size_t)((char *)c - c->prev_size + CSIZE(c) - (char *)p);
    return CSIZE(c) - 8;
}

void *calloc(size_t a, size_t b)
{
    size_t n;
    if (__builtin_mul_overflow(a, b, &n)) { errno = ENOMEM; return 0; }
    void *p = malloc(n);
    if (p) memset(p, 0, n);
    return p;
}

void *realloc(void *p, size_t n)
{
    if (!p) return malloc(n);
    if (!n) { free(p); return 0; }
    size_t have = malloc_usable_size(p);
    if (n <= have && (n >= have / 2 || have < 512)) return p;
    void *q = malloc(n);
    if (!q) return 0;
    memcpy(q, p, have < n ? have : n);
    free(p);
    return q;
}

void *aligned_alloc(size_t align, size_t n)
{
    if (align == 0 || (align & (align - 1))) { errno = EINVAL; return 0; }
    if (align <= 16) return malloc(n);
    void *p = alloc_mapped(n, align);
    if (!p) errno = ENOMEM;
    return p;
}

int posix_memalign(void **out, size_t align, size_t n)
{
    if (align < sizeof(void *) || (align & (align - 1))) return EINVAL;
    void *p = aligned_alloc(align, n);
    if (!p) return ENOMEM;
    *out = p;
    return 0;
}
