/* The blocks runtime (block and __block variable layouts are the compiler's ABI) and
 * dispatch_once. */
#include "objc.h"

struct block_descriptor { unsigned long reserved, size; void (*copy)(void *, const void *); void (*dispose)(const void *); };
struct block { void *isa; volatile int32_t flags; int32_t reserved; void (*invoke)(void *, ...); struct block_descriptor *descriptor; };
struct byref { void *isa; struct byref *forwarding; volatile int32_t flags; uint32_t size; void (*keep)(struct byref *, struct byref *); void (*destroy)(struct byref *); };

enum {
    DEALLOCATING = 1, REFCOUNT_MASK = 0xfffe, NEEDS_FREE = 1 << 24, HAS_COPY_DISPOSE = 1 << 25,
    IS_GLOBAL = 1 << 28, BYREF_HAS_COPY_DISPOSE = 1 << 25,
    FIELD_IS_OBJECT = 3, FIELD_IS_BLOCK = 7, FIELD_IS_BYREF = 8, FIELD_IS_WEAK = 16, BYREF_CALLER = 128,
};

uintptr_t _NSConcreteStackBlock[32], _NSConcreteMallocBlock[32], _NSConcreteGlobalBlock[32],
    _NSConcreteAutoBlock[32], _NSConcreteFinalizingBlock[32], _NSConcreteWeakBlockVariable[32];

void *_Block_copy(const void *arg) {
    struct block *b = (struct block *)arg;
    if (!b) return 0;
    if (b->flags & NEEDS_FREE) { b->flags += 2; return b; }
    if (b->flags & IS_GLOBAL) return b;
    struct block *c = malloc(b->descriptor->size);
    if (!c) return 0;
    memmove(c, b, b->descriptor->size);
    c->flags = (c->flags & ~(REFCOUNT_MASK | DEALLOCATING)) | NEEDS_FREE | 2;
    c->isa = _NSConcreteMallocBlock;
    if (b->flags & HAS_COPY_DISPOSE) c->descriptor->copy(c, b);
    return c;
}

void _Block_release(const void *arg) {
    struct block *b = (struct block *)arg;
    if (!b || !(b->flags & NEEDS_FREE)) return;
    if ((b->flags & REFCOUNT_MASK) > 2) { b->flags -= 2; return; }
    if (b->flags & HAS_COPY_DISPOSE) b->descriptor->dispose(b);
    free(b);
}

static struct byref *byref_copy(struct byref *src) {
    if ((src->forwarding->flags & REFCOUNT_MASK) == 0) {
        struct byref *c = malloc(src->size);
        c->isa = 0;
        c->flags = src->flags | NEEDS_FREE | 4; /* one reference for the heap block, one for the stack frame */
        c->forwarding = c;
        src->forwarding = c;
        c->size = src->size;
        if (src->flags & BYREF_HAS_COPY_DISPOSE) {
            c->keep = src->keep;
            c->destroy = src->destroy;
            /* extended layout word, then the variable */
            size_t head = sizeof *c;
            memmove((char *)c + head, (char *)src + head, src->size - head);
            src->keep(c, src);
        } else {
            size_t head = 24;
            memmove((char *)c + head, (char *)src + head, src->size - head);
        }
        return c;
    }
    if (src->forwarding->flags & NEEDS_FREE) src->forwarding->flags += 2;
    return src->forwarding;
}

static void byref_release(struct byref *b) {
    b = b->forwarding;
    if (!(b->flags & NEEDS_FREE)) return;
    if ((b->flags & REFCOUNT_MASK) > 2) { b->flags -= 2; return; }
    if (b->flags & BYREF_HAS_COPY_DISPOSE) b->destroy(b);
    free(b);
}

void _Block_object_assign(void *dst, const void *obj, const int flags) {
    void **d = dst;
    if (flags & BYREF_CALLER) { *d = (void *)obj; return; }
    switch (flags & (FIELD_IS_OBJECT | FIELD_IS_BLOCK | FIELD_IS_BYREF)) {
    case FIELD_IS_OBJECT: *d = (flags & FIELD_IS_WEAK) ? (void *)obj : objc_retain((id)obj); break;
    case FIELD_IS_BLOCK: *d = _Block_copy(obj); break;
    case FIELD_IS_BYREF: *d = byref_copy((struct byref *)obj); break;
    default: *d = (void *)obj;
    }
}

void _Block_object_dispose(const void *obj, const int flags) {
    if (flags & BYREF_CALLER) return;
    switch (flags & (FIELD_IS_OBJECT | FIELD_IS_BLOCK | FIELD_IS_BYREF)) {
    case FIELD_IS_OBJECT: if (!(flags & FIELD_IS_WEAK)) objc_release((id)obj); break;
    case FIELD_IS_BLOCK: _Block_release(obj); break;
    case FIELD_IS_BYREF: byref_release((struct byref *)obj); break;
    }
}

size_t Block_size(void *b) { return ((struct block *)b)->descriptor->size; }

/* 0: not run; 1: running on some thread (others wait); ~0: done. */
int sched_yield(void);
void dispatch_once_f(long *pred, void *ctx, void (*fn)(void *)) {
    if (__atomic_load_n(pred, __ATOMIC_ACQUIRE) == ~0l) return;
    long z = 0;
    if (__atomic_compare_exchange_n(pred, &z, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
        fn(ctx);
        __atomic_store_n(pred, ~0l, __ATOMIC_RELEASE);
        return;
    }
    while (__atomic_load_n(pred, __ATOMIC_ACQUIRE) != ~0l) sched_yield();
}
static void once_block(void *b) { ((void (*)(void *))((struct block *)b)->invoke)(b); }
void dispatch_once(long *pred, void *block) { dispatch_once_f(pred, block, once_block); }
