#ifndef LOOS_K32_INTERNAL_H
#define LOOS_K32_INTERNAL_H
#include "../include/loos_win.h"

/* ---- thread/process block access ----
 * Real builds read the TEB through gs (x64: ClientId at 0x40/0x48, PEB at
 * 0x60, LastErrorValue at 0x68, PEB->ImageBaseAddress at +0x10). Host tests
 * (LOOS_HOST_TEST) use plain thread-local state so kernel32 logic can run
 * under a normal Linux process. */
#ifdef LOOS_HOST_TEST
extern _Thread_local DWORD k32_host_last_error;
extern DWORD k32_host_pid, k32_host_tid;
extern void *k32_host_image_base;
static inline DWORD *k32_last_error_ptr(void) { return &k32_host_last_error; }
static inline DWORD k32_pid(void) { return k32_host_pid; }
static inline DWORD k32_tid(void) { return k32_host_tid; }
static inline void *k32_image_base(void) { return k32_host_image_base; }
#else
static inline uint64_t k32_gs_q(unsigned off) { uint64_t v; __asm__ volatile("movq %%gs:%1, %0" : "=r"(v) : "m"(*(const uint64_t *)(uintptr_t)off)); return v; }
static inline DWORD *k32_last_error_ptr(void) { return (DWORD *)(k32_gs_q(0x30) + 0x68); }
static inline DWORD k32_pid(void) { return (DWORD)k32_gs_q(0x40); }
static inline DWORD k32_tid(void) { return (DWORD)k32_gs_q(0x48); }
static inline void *k32_image_base(void) { return *(void **)(k32_gs_q(0x60) + 0x10); }
#endif

/* Set last error from an NTSTATUS and return FALSE-ish convenience. */
static inline void k32_set_nt_error(NTSTATUS st) { *k32_last_error_ptr() = RtlNtStatusToDosError(st); }

/* ---- paths (path.c) ----
 * All buffers are caller-provided; K32_PATH_MAX is the NT limit in WCHARs. */
#define K32_PATH_MAX 32768
/* Stack buffer size for the short-path helpers (GetFullPathName, SetCurrentDirectory, cwd copy). */
#define K32_LOCAL_MAX 4096
/* Resolve `dos` against the current directory, normalise, write the DOS full
 * path. Returns the length in WCHARs (excluding NUL), or 0 with *err set. */
size_t k32_full_path(LPCWSTR dos, WCHAR *out, size_t out_cch, size_t *file_part, DWORD *err);
/* DOS path -> NT object path ("\??\C:\x") in `out`; fills `us`. Returns 0 on success else a Win32 error. */
DWORD k32_dos_to_nt(LPCWSTR dos, WCHAR *out, size_t out_cch, UNICODE_STRING *us);

/* Build OBJECT_ATTRIBUTES for a (possibly named) kernel object. name may be NULL. */
DWORD k32_object_attrs(SECURITY_ATTRIBUTES *sa, LPCWSTR name, WCHAR *buf, size_t buf_cch,
                       UNICODE_STRING *us, OBJECT_ATTRIBUTES *oa);
#endif
