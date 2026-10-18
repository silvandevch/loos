/* ntdll heap: Rtl*Heap.
 *
 * A boundary-tag allocator. Memory comes from NtAllocateVirtualMemory in segments;
 * every block has a 16-byte header, so user pointers are 16-byte aligned as the
 * Windows x64 ABI requires. Free blocks are coalesced with both neighbours and kept in
 * size-segregated bins. Requests of 256 KiB or more get their own virtual region.
 *
 *   header {head, info}:  head  = block size (multiple of 16) | INUSE | PREVINUSE | DIRECT
 *                         info  = requested size (low 56 bits) | 0xA5 magic (top byte)
 *   free block body:      fd, bk list links at +16; the block size is repeated in its last 8 bytes
 *                         so the next block can find the start of a free predecessor.
 *   segment:              [seg header 16][blocks ...][end sentinel: header-only in-use block]
 *
 * Locking: a per-heap spinlock unless HEAP_NO_SERIALIZE. Holders never block while holding
 * it, so a plain spin is correct, just not fair; revisit once the kernel has wait queues
 * worth using. */
#include "../include/loos_nt.h"
#include <string.h>

#define F_INUSE     1u
#define F_PREVINUSE 2u
#define F_DIRECT    4u
#define FLAG_MASK   15u
#define SIZE_OF(h)  ((h) & ~(uint64_t)FLAG_MASK)
#define HDR         16u
#define MIN_BLOCK   48u
#define MAGIC       0xA5u
#define DIRECT_MIN  (256u * 1024u)
#define SEG_GROW    (1024u * 1024u)
#define HEAP_SIG    0x50414548u       /* 'HEAP' */
#define NSMALL      64u               /* exact-size bins: 48 .. 1008 */
#define NBINS       (NSMALL + 40u)
#define REQ_MASK    (((uint64_t)1 << 56) - 1)

typedef struct block { uint64_t head, info; } block;
typedef struct link { block *fd, *bk; } link;
typedef struct segment { struct segment *next; size_t size; } segment;
typedef struct dregion { struct dregion *next, *prev; } dregion;      /* first 16 bytes of a direct region */

typedef struct heap {
    uint32_t sig, flags;
    volatile int lock;
    segment *segments;
    dregion *direct;                /* regions backing 256K+ allocations */
    size_t grow;
    block *bins[NBINS];
} heap;

static void lock_heap(heap *h)   { if (!(h->flags & HEAP_NO_SERIALIZE)) while (__atomic_exchange_n(&h->lock, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause(); }
static void unlock_heap(heap *h) { if (!(h->flags & HEAP_NO_SERIALIZE)) __atomic_store_n(&h->lock, 0, __ATOMIC_RELEASE); }

static size_t align16(size_t n) { return (n + 15u) & ~(size_t)15u; }
static link *links(block *b) { return (link *)((char *)b + HDR); }
static block *next_block(block *b) { return (block *)((char *)b + SIZE_OF(b->head)); }
static uint64_t *footer(block *b) { return (uint64_t *)((char *)b + SIZE_OF(b->head) - 8); }
static void *user_ptr(block *b) { return (char *)b + HDR; }
static block *from_user(void *p) { return (block *)((uintptr_t)p - HDR); }

static unsigned bin_of(size_t size)
{
    unsigned b;
    if (size < NSMALL * 16u) return (unsigned)(size / 16u);
    for (b = 0; (size >> (b + 10)) > 1 && b < 38; b++) { }       /* 1K.. in power-of-two steps */
    return NSMALL + b;
}

static void bin_insert(heap *h, block *b)
{
    unsigned i = bin_of(SIZE_OF(b->head));
    link *l = links(b);
    l->fd = h->bins[i]; l->bk = NULL;
    if (h->bins[i]) links(h->bins[i])->bk = b;
    h->bins[i] = b;
}

static void bin_remove(heap *h, block *b)
{
    link *l = links(b);
    if (l->bk) links(l->bk)->fd = l->fd; else h->bins[bin_of(SIZE_OF(b->head))] = l->fd;
    if (l->fd) links(l->fd)->bk = l->bk;
}

/* mark b free with size s (flags: prev-in-use state taken from `prev`), write the footer, bin it */
static void make_free(heap *h, block *b, size_t s, uint64_t prev_flag)
{
    block *n;
    b->head = s | prev_flag;
    b->info = 0;
    *footer(b) = s;
    n = next_block(b);
    n->head &= ~(uint64_t)F_PREVINUSE;
    bin_insert(h, b);
}

static int grow_heap(heap *h, size_t need)
{
    size_t seg = h->grow > need + 64u ? h->grow : align16(need + 64u);
    PVOID base = NULL;
    SIZE_T sz;
    segment *s;
    block *first, *end;

    seg = (seg + 0xFFFFu) & ~(size_t)0xFFFFu;
    sz = seg;
    if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &base, 0, &sz, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE))) return 0;
    s = base; s->size = sz; s->next = h->segments; h->segments = s;
    first = (block *)((char *)base + sizeof(segment));
    end = (block *)((char *)base + sz - HDR);
    end->head = F_INUSE | F_PREVINUSE; end->info = 0;                       /* sentinel: size 0 */
    make_free(h, first, (size_t)((char *)end - (char *)first), F_PREVINUSE);
    end->head = F_INUSE;                                                    /* make_free cleared nothing but be explicit: prev is free */
    return 1;
}

PVOID NTAPI RtlCreateHeap(ULONG flags, PVOID base, SIZE_T reserve, SIZE_T commit, PVOID lock, PVOID params)
{
    PVOID mem = NULL;
    SIZE_T sz;
    segment *s;
    heap *h;
    block *first, *end;
    size_t seg;
    (void)base; (void)lock; (void)params; (void)commit;

    seg = reserve > SEG_GROW ? (size_t)reserve : SEG_GROW;
    seg = (seg + 0xFFFFu) & ~(size_t)0xFFFFu;
    sz = seg;
    if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &mem, 0, &sz, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE))) return NULL;
    s = mem; s->size = sz; s->next = NULL;
    h = (heap *)((char *)mem + sizeof(segment));
    memset(h, 0, sizeof *h);
    h->sig = HEAP_SIG; h->flags = flags; h->segments = s; h->grow = SEG_GROW;
    first = (block *)(((uintptr_t)h + sizeof *h + 15u) & ~(uintptr_t)15u);
    end = (block *)((char *)mem + sz - HDR);
    end->head = F_INUSE; end->info = 0;
    make_free(h, first, (size_t)((char *)end - (char *)first), F_PREVINUSE);
    return h;
}

HANDLE NTAPI RtlDestroyHeap(HANDLE handle)
{
    heap *h = handle;
    segment *s, *next;
    if (!h || h->sig != HEAP_SIG) return handle;
    lock_heap(h);
    s = h->segments;
    h->sig = 0;
    /* the heap object lives in the oldest segment, the tail of the list, so it is freed last */
    for (; s; s = next) {
        PVOID base = s; SIZE_T zero = 0;
        next = s->next;
        NtFreeVirtualMemory(NtCurrentProcess(), &base, &zero, MEM_RELEASE);
    }
    return NULL;
}

static void *alloc_direct(heap *h, size_t req)
{
    PVOID base = NULL;
    SIZE_T sz;
    dregion *r;
    block *b;
    if (req > ((size_t)1 << 47)) return NULL;
    sz = (req + 2 * HDR + 0xFFFu) & ~(size_t)0xFFFu;
    if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &base, 0, &sz, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE))) return NULL;
    r = base;                                                      /* [region links 16][block header 16][user data] */
    r->prev = NULL; r->next = h->direct;
    if (h->direct) h->direct->prev = r;
    h->direct = r;
    b = (block *)((char *)base + HDR);
    b->head = (uint64_t)sz | F_INUSE | F_DIRECT | F_PREVINUSE;
    b->info = (uint64_t)req | ((uint64_t)MAGIC << 56);
    return user_ptr(b);
}

static void *heap_alloc_locked(heap *h, size_t req)
{
    size_t need = req + HDR < MIN_BLOCK ? MIN_BLOCK : align16(req + HDR);
    unsigned i;
    block *b = NULL;

    if (req > ((size_t)1 << 47)) return NULL;
    if (need >= DIRECT_MIN) return alloc_direct(h, req);
    for (;;) {
        for (i = bin_of(need); i < NBINS && !b; i++) {
            block *c;
            for (c = h->bins[i]; c; c = links(c)->fd) if (SIZE_OF(c->head) >= need) { b = c; break; }
        }
        if (b) break;
        if (!grow_heap(h, need)) return NULL;
    }
    bin_remove(h, b);
    {
        size_t have = SIZE_OF(b->head), rem = have - need;
        uint64_t prev = b->head & F_PREVINUSE;
        if (rem >= MIN_BLOCK) {
            block *r = (block *)((char *)b + need);
            b->head = need | F_INUSE | prev;
            make_free(h, r, rem, F_PREVINUSE);              /* its predecessor (b) is now in use */
        } else {
            b->head = have | F_INUSE | prev;
            next_block(b)->head |= F_PREVINUSE;
        }
    }
    b->info = (uint64_t)req | ((uint64_t)MAGIC << 56);
    return user_ptr(b);
}

/* A pointer is only trusted if it lies inside this heap's own segments or is one of its direct
 * regions, so a stray pointer is rejected without being dereferenced. */
static int valid_block(heap *h, void *p, block **out)
{
    block *b;
    segment *s;
    dregion *r;
    int owned = 0;
    if (!p || ((uintptr_t)p & 15u)) return 0;
    b = from_user(p);
    for (s = h->segments; s && !owned; s = s->next)
        owned = (char *)b >= (char *)s + sizeof(segment) && (char *)p < (char *)s + s->size;
    for (r = h->direct; r && !owned; r = r->next) owned = (char *)r + HDR == (char *)b;
    if (!owned) return 0;
    if ((b->info >> 56) != MAGIC || !(b->head & F_INUSE)) return 0;
    *out = b;
    return 1;
}

static void heap_free_locked(heap *h, block *b)
{
    size_t s;
    uint64_t prev_flag;
    block *n;

    if (b->head & F_DIRECT) {
        dregion *r = (dregion *)((char *)b - HDR);
        PVOID base = r; SIZE_T zero = 0;
        if (r->prev) r->prev->next = r->next; else h->direct = r->next;
        if (r->next) r->next->prev = r->prev;
        b->info = 0;
        NtFreeVirtualMemory(NtCurrentProcess(), &base, &zero, MEM_RELEASE);
        return;
    }
    s = SIZE_OF(b->head);
    prev_flag = b->head & F_PREVINUSE;
    n = next_block(b);
    if (!(n->head & F_INUSE)) {                              /* merge forward */
        bin_remove(h, n);
        s += SIZE_OF(n->head);
    }
    if (!prev_flag) {                                        /* merge backward through the footer */
        size_t ps = (size_t)*(uint64_t *)((char *)b - 8);
        block *p = (block *)((char *)b - ps);
        bin_remove(h, p);
        s += ps;
        b = p;
        prev_flag = b->head & F_PREVINUSE;
    }
    make_free(h, b, s, prev_flag);
}

static heap *as_heap(HANDLE handle) { heap *h = handle; return h && h->sig == HEAP_SIG ? h : NULL; }

PVOID NTAPI RtlAllocateHeap(HANDLE handle, ULONG flags, SIZE_T size)
{
    heap *h = as_heap(handle);
    void *p;
    if (!h) return NULL;
    lock_heap(h);
    p = heap_alloc_locked(h, size);
    unlock_heap(h);
    if (p && ((flags | h->flags) & HEAP_ZERO_MEMORY)) memset(p, 0, size);
    return p;
}

BOOLEAN NTAPI RtlFreeHeap(HANDLE handle, ULONG flags, PVOID ptr)
{
    heap *h = as_heap(handle);
    block *b;
    (void)flags;
    if (!h) return FALSE;
    if (!ptr) return TRUE;                                   /* freeing NULL is a successful no-op */
    lock_heap(h);
    if (!valid_block(h, ptr, &b)) { unlock_heap(h); return FALSE; }
    heap_free_locked(h, b);
    unlock_heap(h);
    return TRUE;
}

SIZE_T NTAPI RtlSizeHeap(HANDLE handle, ULONG flags, PVOID ptr)
{
    heap *h = as_heap(handle);
    block *b;
    (void)flags;
    if (!h || !valid_block(h, ptr, &b)) return (SIZE_T)-1;
    return (SIZE_T)(b->info & REQ_MASK);
}

BOOLEAN NTAPI RtlValidateHeap(HANDLE handle, ULONG flags, PVOID ptr)
{
    heap *h = as_heap(handle);
    segment *s;
    block *b;
    (void)flags;
    if (!h) return FALSE;
    if (ptr) return valid_block(h, ptr, &b);
    lock_heap(h);
    for (s = h->segments; s; s = s->next) {                  /* walk every segment, checking the tags */
        char *seg_end = (char *)s + s->size;
        block *c = (block *)((char *)s + sizeof(segment));
        if ((char *)h > (char *)s && (char *)h < seg_end)               /* the segment that holds the heap object */
            c = (block *)(((uintptr_t)h + sizeof *h + 15u) & ~(uintptr_t)15u);
        for (;;) {
            size_t sz = (size_t)SIZE_OF(c->head);
            if (sz == 0) { if ((char *)c + HDR != seg_end) { unlock_heap(h); return FALSE; } break; }
            if ((char *)c + sz > seg_end || sz < MIN_BLOCK) { unlock_heap(h); return FALSE; }
            if (!(c->head & F_INUSE)) {
                block *n = next_block(c);
                if (*footer(c) != sz || (n->head & F_PREVINUSE) || !(n->head & F_INUSE ? 1 : 0)) {
                    /* a free block must be followed by an in-use block (no uncoalesced neighbours) */
                    unlock_heap(h); return FALSE;
                }
            } else if ((c->info >> 56) != MAGIC) { unlock_heap(h); return FALSE; }
            c = next_block(c);
        }
    }
    unlock_heap(h);
    return TRUE;
}

PVOID NTAPI RtlReAllocateHeap(HANDLE handle, ULONG flags, PVOID ptr, SIZE_T size)
{
    heap *h = as_heap(handle);
    block *b;
    void *np;
    size_t old, cap;

    if (!h) return NULL;
    if (!ptr) return RtlAllocateHeap(handle, flags, size);
    lock_heap(h);
    if (!valid_block(h, ptr, &b)) { unlock_heap(h); return NULL; }
    old = (size_t)(b->info & REQ_MASK);
    cap = (size_t)SIZE_OF(b->head) - ((b->head & F_DIRECT) ? 2 * HDR : HDR);

    if (size <= cap && (!(b->head & F_DIRECT) || size >= DIRECT_MIN / 2)) {
        b->info = (uint64_t)size | ((uint64_t)MAGIC << 56);          /* fits: stay put (shrink keeps the slack) */
        unlock_heap(h);
        if (size > old && ((flags | h->flags) & HEAP_ZERO_MEMORY)) memset((char *)ptr + old, 0, size - old);
        return ptr;
    }
    if (!(b->head & F_DIRECT) && size + HDR < DIRECT_MIN) {          /* try to extend into a free successor */
        block *n = next_block(b);
        size_t cur = SIZE_OF(b->head), want = size + HDR < MIN_BLOCK ? MIN_BLOCK : align16(size + HDR);
        if (!(n->head & F_INUSE) && cur + SIZE_OF(n->head) >= want) {
            size_t total = cur + SIZE_OF(n->head), rem = total - want;
            uint64_t prev = b->head & F_PREVINUSE;
            bin_remove(h, n);
            if (rem >= MIN_BLOCK) {
                block *r = (block *)((char *)b + want);
                b->head = want | F_INUSE | prev;
                make_free(h, r, rem, F_PREVINUSE);
            } else {
                b->head = total | F_INUSE | prev;
                next_block(b)->head |= F_PREVINUSE;
            }
            b->info = (uint64_t)size | ((uint64_t)MAGIC << 56);
            unlock_heap(h);
            if (size > old && ((flags | h->flags) & HEAP_ZERO_MEMORY)) memset((char *)ptr + old, 0, size - old);
            return ptr;
        }
    }
    if (flags & HEAP_REALLOC_IN_PLACE_ONLY) { unlock_heap(h); return NULL; }
    np = heap_alloc_locked(h, size);
    if (np) {
        memcpy(np, ptr, old < size ? old : size);
        if (size > old && ((flags | h->flags) & HEAP_ZERO_MEMORY)) memset((char *)np + old, 0, size - old);
        heap_free_locked(h, b);
    }
    unlock_heap(h);
    return np;
}

static HANDLE process_heap;

HANDLE NTAPI RtlGetProcessHeap(void)
{
    HANDLE h = __atomic_load_n(&process_heap, __ATOMIC_ACQUIRE);
    if (!h) {
        HANDLE fresh = RtlCreateHeap(HEAP_GROWABLE, NULL, 0, 0, NULL, NULL), expected = NULL;
        if (!fresh) return NULL;
        if (__atomic_compare_exchange_n(&process_heap, &expected, fresh, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) h = fresh;
        else { RtlDestroyHeap(fresh); h = expected; }
    }
    return h;
}

#ifdef LOOS_HOST_TEST
/* Host tests tear the fake address space down between cases; forget the cached process heap with it. */
void loos_heap_test_reset(void) { process_heap = NULL; }
#endif
