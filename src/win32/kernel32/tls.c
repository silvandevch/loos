/* Thread-local storage (TlsAlloc family, 64 inline + 1024 expansion slots) and fiber-local storage
 * (FlsAlloc family, 128 slots with destructor callbacks).
 *
 * Known gaps: TlsFree and FlsFree only clear the calling thread's value (the kernel normally walks all
 * threads); a reused index can therefore expose a stale value in another thread. FLS destructors run
 * when the owning thread exits through ExitThread or when the slot is freed on the calling thread. */
#include "k32_internal.h"

#define TLS_INLINE 64u
#define TLS_EXPANSION 1024u
#define TLS_TOTAL (TLS_INLINE + TLS_EXPANSION)
#define FLS_MAX 128u

static volatile int tls_lock;
static uint8_t tls_used[TLS_TOTAL / 8];
static void lock(volatile int *l) { while (__atomic_exchange_n(l, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause(); }
static void unlock(volatile int *l) { __atomic_store_n(l, 0, __ATOMIC_RELEASE); }

static PVOID *tls_ref(DWORD idx, int create)
{
    if (idx < TLS_INLINE) return &k32_tls_slots()[idx];
    if (idx >= TLS_TOTAL) return NULL;
    {
        PVOID **exp = k32_tls_expansion_slot();
        if (!*exp) {
            if (!create) return NULL;
            *exp = RtlAllocateHeap(RtlGetProcessHeap(), HEAP_ZERO_MEMORY, TLS_EXPANSION * sizeof(PVOID));
            if (!*exp) return NULL;
        }
        return &(*exp)[idx - TLS_INLINE];
    }
}

DWORD WINAPI TlsAlloc(void)
{
    DWORD i;
    lock(&tls_lock);
    for (i = 0; i < TLS_TOTAL; i++)
        if (!(tls_used[i / 8] & (1u << (i % 8)))) { tls_used[i / 8] |= (uint8_t)(1u << (i % 8)); break; }
    unlock(&tls_lock);
    if (i == TLS_TOTAL) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return TLS_OUT_OF_INDEXES; }
    { PVOID *s = tls_ref(i, 0); if (s) *s = NULL; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return i;
}

BOOL WINAPI TlsFree(DWORD idx)
{
    int was;
    if (idx >= TLS_TOTAL) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    lock(&tls_lock);
    was = (tls_used[idx / 8] & (1u << (idx % 8))) != 0;
    tls_used[idx / 8] &= (uint8_t)~(1u << (idx % 8));
    unlock(&tls_lock);
    if (!was) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    { PVOID *s = tls_ref(idx, 0); if (s) *s = NULL; }
    return TRUE;
}

LPVOID WINAPI TlsGetValue(DWORD idx)
{
    PVOID *s = tls_ref(idx, 0);
    if (idx >= TLS_TOTAL) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return s ? *s : NULL;
}

BOOL WINAPI TlsSetValue(DWORD idx, LPVOID v)
{
    PVOID *s;
    if (idx >= TLS_TOTAL) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    s = tls_ref(idx, 1);
    if (!s) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return FALSE; }
    *s = v;
    return TRUE;
}

/* ---------- fiber-local storage ---------- */
static volatile int fls_lock;
static struct { int used; void (WINAPI *cb)(PVOID); } fls_tab[FLS_MAX];

static PVOID *fls_data(int create)
{
    PVOID *slot = k32_fls_slot();
    if (!*slot && create) *slot = RtlAllocateHeap(RtlGetProcessHeap(), HEAP_ZERO_MEMORY, FLS_MAX * sizeof(PVOID));
    return (PVOID *)*slot;
}

DWORD WINAPI FlsAlloc(void (WINAPI *cb)(PVOID))
{
    DWORD i;
    lock(&fls_lock);
    for (i = 0; i < FLS_MAX; i++) if (!fls_tab[i].used) { fls_tab[i].used = 1; fls_tab[i].cb = cb; break; }
    unlock(&fls_lock);
    if (i == FLS_MAX) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return FLS_OUT_OF_INDEXES; }
    { PVOID *d = fls_data(0); if (d) d[i] = NULL; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    return i;
}

BOOL WINAPI FlsFree(DWORD idx)
{
    void (WINAPI *cb)(PVOID) = NULL;
    PVOID *d, v = NULL;
    if (idx >= FLS_MAX) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    lock(&fls_lock);
    if (!fls_tab[idx].used) { unlock(&fls_lock); *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    cb = fls_tab[idx].cb; fls_tab[idx].used = 0; fls_tab[idx].cb = NULL;
    unlock(&fls_lock);
    if ((d = fls_data(0)) != NULL) { v = d[idx]; d[idx] = NULL; }
    if (cb && v) cb(v);
    return TRUE;
}

PVOID WINAPI FlsGetValue(DWORD idx)
{
    PVOID *d;
    if (idx >= FLS_MAX || !fls_tab[idx].used) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    d = fls_data(0);
    return d ? d[idx] : NULL;
}

BOOL WINAPI FlsSetValue(DWORD idx, PVOID v)
{
    PVOID *d;
    if (idx >= FLS_MAX || !fls_tab[idx].used) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    d = fls_data(1);
    if (!d) { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; return FALSE; }
    d[idx] = v;
    return TRUE;
}

/* Run destructors for the exiting thread and release its FLS array. */
void k32_fls_thread_exit(void)
{
    PVOID *d = fls_data(0);
    DWORD i;
    int again;
    if (!d) return;
    do {                                                       /* a destructor may set another slot; iterate to a fixed point */
        again = 0;
        for (i = 0; i < FLS_MAX; i++) {
            void (WINAPI *cb)(PVOID) = NULL;
            PVOID v = d[i];
            lock(&fls_lock);
            if (fls_tab[i].used) cb = fls_tab[i].cb;
            unlock(&fls_lock);
            if (v && cb) { d[i] = NULL; cb(v); again = 1; }
        }
    } while (again);
    *k32_fls_slot() = NULL;
    RtlFreeHeap(RtlGetProcessHeap(), 0, d);
}
