#ifndef LOOS_K32_INTERNAL_H
#define LOOS_K32_INTERNAL_H
#include "../include/loos_win.h"

#include "../include/loos_teb.h"

/* Unicode simple case mapping lives in ntdll (Latin-1, Latin Extended-A, Greek, Cyrillic; not the full tables). */
static inline WCHAR k32_towupper(WCHAR c) { return RtlUpcaseUnicodeChar(c); }
static inline WCHAR k32_towlower(WCHAR c) { return RtlDowncaseUnicodeChar(c); }

void k32_fls_thread_exit(void);                   /* tls.c: run FLS destructors for the exiting thread */

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
