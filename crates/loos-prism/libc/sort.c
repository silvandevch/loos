/* qsort/bsearch (they call the program's comparison function). qsort is a merge sort through a
 * temporary buffer when one can be allocated (stable, O(n log n)), else an insertion sort. */
#include "prism.h"

typedef int (*cmp_fn)(const void *, const void *);

static void swap_bytes(char *a, char *b, size_t n) {
    while (n--) {
        char t = *a;
        *a++ = *b;
        *b++ = t;
    }
}

static void insertion(char *base, size_t n, size_t w, cmp_fn cmp) {
    for (size_t i = 1; i < n; i++)
        for (size_t j = i; j > 0 && cmp(base + (j - 1) * w, base + j * w) > 0; j--)
            swap_bytes(base + (j - 1) * w, base + j * w, w);
}

static void merge_sort(char *a, char *tmp, size_t n, size_t w, cmp_fn cmp) {
    if (n < 16) {
        insertion(a, n, w, cmp);
        return;
    }
    size_t h = n / 2;
    merge_sort(a, tmp, h, w, cmp);
    merge_sort(a + h * w, tmp, n - h, w, cmp);
    if (cmp(a + (h - 1) * w, a + h * w) <= 0)
        return; /* already in order */
    size_t i = 0, j = h, k = 0;
    while (i < h && j < n) {
        char *src = cmp(a + i * w, a + j * w) <= 0 ? a + i++ * w : a + j++ * w;
        memcpy(tmp + k++ * w, src, w);
    }
    if (i < h)
        memcpy(tmp + k * w, a + i * w, (h - i) * w), k += h - i;
    if (j < n)
        memcpy(tmp + k * w, a + j * w, (n - j) * w), k += n - j;
    memcpy(a, tmp, n * w);
}

void qsort(void *base, size_t n, size_t w, cmp_fn cmp) {
    if (n < 2 || w == 0)
        return;
    char *tmp = n * w <= ((size_t)1 << 32) ? malloc(n * w) : NULL;
    if (tmp) {
        merge_sort(base, tmp, n, w, cmp);
        free(tmp);
    } else {
        insertion(base, n, w, cmp);
    }
}

int mergesort(void *base, size_t n, size_t w, cmp_fn cmp) {
    qsort(base, n, w, cmp);
    return 0;
}

static void *qsort_r_thunk_ctx;
static int (*qsort_r_cmp)(void *, const void *, const void *);
static int qsort_r_thunk(const void *a, const void *b) { return qsort_r_cmp(qsort_r_thunk_ctx, a, b); }
void qsort_r(void *base, size_t n, size_t w, void *ctx, int (*cmp)(void *, const void *, const void *)) {
    qsort_r_thunk_ctx = ctx; /* one thread per Prism process */
    qsort_r_cmp = cmp;
    qsort(base, n, w, qsort_r_thunk);
}

void *bsearch(const void *key, const void *base, size_t n, size_t w, cmp_fn cmp) {
    const char *lo = base;
    while (n) {
        const char *mid = lo + (n / 2) * w;
        int r = cmp(key, mid);
        if (r == 0)
            return (void *)mid;
        if (r > 0) {
            lo = mid + w;
            n -= n / 2 + 1;
        } else {
            n /= 2;
        }
    }
    return NULL;
}
