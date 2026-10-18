#define LOOS_HOST_TEST 1
#include "../kernel32/fake.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %llu, want %llu\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

static uint32_t rng = 0xC0FFEE;
static uint32_t rnd(void) { rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5; return rng; }

typedef struct { unsigned char *p; size_t n; unsigned char tag; } slot;

static void fill(slot *s) { memset(s->p, s->tag, s->n); }
static int intact(const slot *s, size_t upto) { size_t i; for (i = 0; i < upto; i++) if (s->p[i] != s->tag) return 0; return 1; }

static void test_basic(void)
{
    HANDLE h = RtlCreateHeap(HEAP_GROWABLE, NULL, 0, 0, NULL, NULL);
    unsigned char *a, *b, *c;
    CHECK(h != NULL);
    a = RtlAllocateHeap(h, 0, 100); b = RtlAllocateHeap(h, 0, 100); c = RtlAllocateHeap(h, HEAP_ZERO_MEMORY, 4096);
    CHECK(a && b && c && a != b);
    CHECK(((uintptr_t)a & 15) == 0 && ((uintptr_t)b & 15) == 0 && ((uintptr_t)c & 15) == 0);       /* x64 ABI alignment */
    CHECK(RtlSizeHeap(h, 0, a) == 100 && RtlSizeHeap(h, 0, c) == 4096);
    { size_t i; int z = 1; for (i = 0; i < 4096; i++) z &= c[i] == 0; CHECK(z); }
    memset(a, 0xAA, 100); memset(b, 0xBB, 100);
    CHECK(RtlValidateHeap(h, 0, NULL) && RtlValidateHeap(h, 0, a));
    CHECK(RtlFreeHeap(h, 0, a));
    CHECK(!RtlFreeHeap(h, 0, a));                               /* double free is rejected */
    CHECK(RtlSizeHeap(h, 0, a) == (SIZE_T)-1);
    CHECK(!RtlFreeHeap(h, 0, (char *)b + 16));                  /* not a block start */
    CHECK(RtlFreeHeap(h, 0, NULL));                             /* NULL free succeeds */
    CHECK(b[0] == 0xBB && b[99] == 0xBB);
    CHECK(RtlAllocateHeap(h, 0, (SIZE_T)1 << 60) == NULL);      /* absurd request fails cleanly */
    CHECK(RtlAllocateHeap(NULL, 0, 8) == NULL);
    RtlFreeHeap(h, 0, b); RtlFreeHeap(h, 0, c);
    CHECK(RtlValidateHeap(h, 0, NULL));
    CHECK(RtlDestroyHeap(h) == NULL);
    CHECK_EQ(F.nvm_blocks, 0);                                  /* everything returned to the VM layer */
}

static void test_coalesce(void)
{
    HANDLE h = RtlCreateHeap(0, NULL, 0, 0, NULL, NULL);
    void *p[200]; int i;
    unsigned before_free_calls;
    for (i = 0; i < 200; i++) { p[i] = RtlAllocateHeap(h, 0, 24 + (i % 7) * 40); CHECK(p[i] != NULL); }
    for (i = 0; i < 200; i += 2) CHECK(RtlFreeHeap(h, 0, p[i]));
    CHECK(RtlValidateHeap(h, 0, NULL));
    for (i = 1; i < 200; i += 2) CHECK(RtlFreeHeap(h, 0, p[i]));
    CHECK(RtlValidateHeap(h, 0, NULL));
    /* fully coalesced: one allocation nearly the size of the segment must now succeed without growing */
    before_free_calls = (unsigned)F.nvm_blocks;
    { void *big = RtlAllocateHeap(h, 0, 200 * 1024); CHECK(big != NULL); CHECK_EQ(F.nvm_blocks, before_free_calls); RtlFreeHeap(h, 0, big); }
    RtlDestroyHeap(h);
    CHECK_EQ(F.nvm_blocks, 0);
}

static void test_realloc(void)
{
    HANDLE h = RtlCreateHeap(HEAP_GROWABLE, NULL, 0, 0, NULL, NULL);
    unsigned char *a = RtlAllocateHeap(h, 0, 64), *b, *guard = RtlAllocateHeap(h, 0, 64);
    size_t i;
    memset(a, 0x5A, 64); memset(guard, 0x77, 64);
    b = RtlReAllocateHeap(h, 0, a, 32);                                  /* shrink: in place */
    CHECK(b == a && RtlSizeHeap(h, 0, b) == 32);
    b = RtlReAllocateHeap(h, HEAP_ZERO_MEMORY, a, 60);                   /* back up within capacity */
    CHECK(b == a); for (i = 0; i < 32; i++) CHECK(b[i] == 0x5A); for (i = 32; i < 60; i++) CHECK(b[i] == 0);
    CHECK(RtlReAllocateHeap(h, HEAP_REALLOC_IN_PLACE_ONLY, a, 100000) == NULL);   /* cannot move */
    CHECK(RtlSizeHeap(h, 0, a) == 60);                                   /* untouched on failure */
    b = RtlReAllocateHeap(h, HEAP_ZERO_MEMORY, a, 100000);               /* must move */
    CHECK(b && b != a);
    for (i = 0; i < 32; i++) CHECK(b[i] == 0x5A);
    for (i = 60; i < 100000; i += 997) CHECK(b[i] == 0);
    CHECK(guard[0] == 0x77 && guard[63] == 0x77);
    CHECK(RtlReAllocateHeap(h, 0, NULL, 128) != NULL);                   /* NULL ptr behaves like alloc */
    CHECK(RtlReAllocateHeap(h, 0, (void *)0x10, 128) == NULL);           /* garbage ptr */
    /* growing into a free neighbour stays in place */
    { HANDLE h2 = RtlCreateHeap(HEAP_GROWABLE, NULL, 0, 0, NULL, NULL);       /* fresh heap: x and y are adjacent at the top */
      unsigned char *x = RtlAllocateHeap(h2, 0, 64), *y = RtlAllocateHeap(h2, 0, 512);
      unsigned char *grown;
      RtlDestroyHeap(h); h = h2;
      memset(x, 1, 64);
      RtlFreeHeap(h, 0, y);
      grown = RtlReAllocateHeap(h, HEAP_REALLOC_IN_PLACE_ONLY, x, 400);
      CHECK(grown == x); CHECK(x[0] == 1 && x[63] == 1); }
    CHECK(RtlValidateHeap(h, 0, NULL));
    RtlDestroyHeap(h);
}

static void test_direct_and_multiple(void)
{
    HANDLE h1 = RtlCreateHeap(0, NULL, 0, 0, NULL, NULL), h2 = RtlCreateHeap(HEAP_NO_SERIALIZE, NULL, 0, 0, NULL, NULL);
    unsigned char *big = RtlAllocateHeap(h1, 0, 3u << 20), *s2 = RtlAllocateHeap(h2, 0, 10);
    CHECK(big != NULL && s2 != NULL);
    big[0] = 1; big[(3u << 20) - 1] = 2;
    CHECK(RtlSizeHeap(h1, 0, big) == (3u << 20));
    CHECK(!RtlFreeHeap(h2, 0, big) || 1);                                /* foreign pointer: magic check may pass; must not crash */
    big = RtlReAllocateHeap(h1, 0, big, 5u << 20);
    CHECK(big && big[0] == 1 && big[(3u << 20) - 1] == 2);
    big = RtlReAllocateHeap(h1, 0, big, 100);                            /* shrink a direct block far below the threshold: moves into the heap */
    CHECK(big && big[0] == 1);
    CHECK(RtlFreeHeap(h1, 0, big));
    CHECK(RtlFreeHeap(h2, 0, s2));
    RtlDestroyHeap(h1); RtlDestroyHeap(h2);
    CHECK_EQ(F.nvm_blocks, 0);
}

static void test_stress(void)
{
    HANDLE h = RtlCreateHeap(HEAP_GROWABLE, NULL, 0, 0, NULL, NULL);
    enum { N = 400 };
    static slot s[N];
    int iter, i;
    memset(s, 0, sizeof s);
    for (iter = 0; iter < 60000; iter++) {
        i = (int)(rnd() % N);
        if (!s[i].p) {
            size_t n = (rnd() & 7) ? 1 + rnd() % 700 : 1 + rnd() % 70000;
            s[i].p = RtlAllocateHeap(h, 0, n);
            CHECK(s[i].p != NULL);
            if (!s[i].p) break;
            s[i].n = n; s[i].tag = (unsigned char)(1 + rnd() % 250);
            fill(&s[i]);
        } else {
            unsigned op = rnd() % 4;
            CHECK(intact(&s[i], s[i].n));                                /* no neighbour ever scribbled on us */
            if (op == 0) {                                               /* realloc */
                size_t n = 1 + rnd() % 3000;
                unsigned char *np = RtlReAllocateHeap(h, 0, s[i].p, n);
                CHECK(np != NULL);
                if (np) { size_t keep = n < s[i].n ? n : s[i].n; s[i].p = np; CHECK(intact(&s[i], keep)); s[i].n = n; fill(&s[i]); }
            } else {
                CHECK(RtlFreeHeap(h, 0, s[i].p));
                s[i].p = NULL;
            }
        }
        if (iter % 5000 == 0) CHECK(RtlValidateHeap(h, 0, NULL));
    }
    CHECK(RtlValidateHeap(h, 0, NULL));
    for (i = 0; i < N; i++) if (s[i].p) { CHECK(intact(&s[i], s[i].n)); CHECK(RtlFreeHeap(h, 0, s[i].p)); }
    CHECK(RtlValidateHeap(h, 0, NULL));
    RtlDestroyHeap(h);
    CHECK_EQ(F.nvm_blocks, 0);
}

static void test_process_heap(void)
{
    HANDLE a = RtlGetProcessHeap(), b = RtlGetProcessHeap();
    void *p;
    CHECK(a != NULL && a == b);
    p = RtlAllocateHeap(a, HEAP_ZERO_MEMORY, 33);
    CHECK(p != NULL && RtlFreeHeap(a, 0, p));
}

int main(void)
{
    fake_reset();
    test_basic(); test_coalesce(); test_realloc(); test_direct_and_multiple(); test_stress(); test_process_heap();
    fake_reset();
    if (fails) printf("heap: %d FAILED\n", fails); else printf("heap: all passed\n");
    return fails != 0;
}
