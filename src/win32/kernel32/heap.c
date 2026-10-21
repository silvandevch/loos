/* Heap and legacy Local/Global allocation, on the ntdll heap.
 *
 * LMEM_MOVEABLE / GMEM_MOVEABLE: a moveable handle is the same pointer as a fixed block here
 * (LocalLock returns it unchanged). That is enough for the callers left in practice that
 * treat the result as an opaque handle, but code that depends on a handle differing from the
 * pointer (clipboard / OLE data objects) will need a real handle table. */
#include "k32_internal.h"

HANDLE WINAPI GetProcessHeap(void) { return RtlGetProcessHeap(); }

HANDLE WINAPI HeapCreate(DWORD flags, SIZE_T initial, SIZE_T maximum)
{
    HANDLE h;
    if (flags & ~(DWORD)(HEAP_NO_SERIALIZE | HEAP_GENERATE_EXCEPTIONS | 0x00040000u /* executable */)) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
        return NULL;
    }
    if (maximum && initial > maximum) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    h = RtlCreateHeap(flags | (maximum == 0 ? HEAP_GROWABLE : 0), NULL, maximum, initial, NULL, NULL);
    if (!h) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return h;
}

BOOL WINAPI HeapDestroy(HANDLE heap)
{
    if (!heap || heap == RtlGetProcessHeap()) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (RtlDestroyHeap(heap) != NULL) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    return TRUE;
}

LPVOID WINAPI HeapAlloc(HANDLE heap, DWORD flags, SIZE_T size)
{
    LPVOID p = RtlAllocateHeap(heap, flags, size);
    if (!p) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return p;
}

LPVOID WINAPI HeapReAlloc(HANDLE heap, DWORD flags, LPVOID mem, SIZE_T size)
{
    LPVOID p;
    if (!mem) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }       /* unlike realloc(), NULL is an error */
    p = RtlReAllocateHeap(heap, flags, mem, size);
    if (!p) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return p;
}

BOOL WINAPI HeapFree(HANDLE heap, DWORD flags, LPVOID mem)
{
    if (!RtlFreeHeap(heap, flags, mem)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    return TRUE;
}

SIZE_T WINAPI HeapSize(HANDLE heap, DWORD flags, LPCVOID mem) { return RtlSizeHeap(heap, flags, (PVOID)mem); }
BOOL   WINAPI HeapValidate(HANDLE heap, DWORD flags, LPCVOID mem) { return RtlValidateHeap(heap, flags, (PVOID)mem); }
SIZE_T WINAPI HeapCompact(HANDLE heap, DWORD flags) { (void)heap; (void)flags; return 0; }

/* The termination-on-corruption / low-fragmentation knobs do not exist; accepting them keeps CRT startup happy. */
BOOL WINAPI HeapSetInformation(HANDLE heap, int info_class, PVOID info, SIZE_T len) { (void)heap; (void)info_class; (void)info; (void)len; return TRUE; }
BOOL WINAPI HeapQueryInformation(HANDLE heap, int info_class, PVOID info, SIZE_T len, SIZE_T *ret)
{
    (void)heap; (void)info_class;
    if (ret) *ret = sizeof(ULONG);
    if (info && len >= sizeof(ULONG)) *(ULONG *)info = 0;
    else { *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return FALSE; }
    return TRUE;
}

static HANDLE legacy_alloc(UINT flags, SIZE_T size)
{
    HANDLE p = RtlAllocateHeap(RtlGetProcessHeap(), (flags & LMEM_ZEROINIT) ? HEAP_ZERO_MEMORY : 0, size ? size : 1);
    if (!p) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return p;
}
static HANDLE legacy_realloc(HANDLE mem, SIZE_T size, UINT flags)
{
    HANDLE p;
    if (!mem) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return NULL; }
    p = RtlReAllocateHeap(RtlGetProcessHeap(), (flags & LMEM_ZEROINIT) ? HEAP_ZERO_MEMORY : 0, mem, size ? size : 1);
    if (!p) *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY;
    return p;
}
static HANDLE legacy_free(HANDLE mem)
{
    if (!mem) return NULL;
    if (!RtlFreeHeap(RtlGetProcessHeap(), 0, mem)) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return mem; }   /* failure returns the handle */
    return NULL;
}
static SIZE_T legacy_size(HANDLE mem)
{
    SIZE_T n = RtlSizeHeap(RtlGetProcessHeap(), 0, mem);
    if (n == (SIZE_T)-1) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return 0; }
    return n;
}

HLOCAL WINAPI LocalAlloc(UINT flags, SIZE_T size) { return legacy_alloc(flags, size); }
HLOCAL WINAPI LocalReAlloc(HLOCAL mem, SIZE_T size, UINT flags) { return legacy_realloc(mem, size, flags); }
HLOCAL WINAPI LocalFree(HLOCAL mem) { return legacy_free(mem); }
SIZE_T WINAPI LocalSize(HLOCAL mem) { return legacy_size(mem); }
LPVOID WINAPI LocalLock(HLOCAL mem) { return mem; }
BOOL   WINAPI LocalUnlock(HLOCAL mem) { (void)mem; *k32_last_error_ptr() = ERROR_SUCCESS; return FALSE; }   /* lock count reached zero */
HGLOBAL WINAPI GlobalAlloc(UINT flags, SIZE_T size) { return legacy_alloc(flags, size); }
HGLOBAL WINAPI GlobalReAlloc(HGLOBAL mem, SIZE_T size, UINT flags) { return legacy_realloc(mem, size, flags); }
HGLOBAL WINAPI GlobalFree(HGLOBAL mem) { return legacy_free(mem); }
SIZE_T WINAPI GlobalSize(HGLOBAL mem) { return legacy_size(mem); }
LPVOID WINAPI GlobalLock(HGLOBAL mem) { return mem; }
BOOL   WINAPI GlobalUnlock(HGLOBAL mem) { (void)mem; *k32_last_error_ptr() = ERROR_SUCCESS; return FALSE; }
