/* LoOS user-mode image loader (the Ldr* half of ntdll).
 *
 * Maps PE images, resolves API sets, relocates, binds imports (following
 * export forwarders), and runs DLL/TLS entry points in dependency order.
 * Everything that touches the OS goes through ldr_backend, so the same code
 * runs in the real ntdll (backend = Nt* syscalls) and in host tests (backend =
 * malloc + fake entry calls).
 *
 * Lifetime model: each module has an `explicit` count (LoadLibrary-style). A
 * module is live while it is reachable, through import edges, from any module
 * with explicit > 0. Unload and failed loads both run a mark-and-sweep, so
 * import cycles are collected correctly and DLL_PROCESS_DETACH runs in reverse
 * initialisation order.
 */
#ifndef LOOS_LDR_H
#define LOOS_LDR_H

#include "../pe/pe.h"

#ifdef __cplusplus
extern "C" {
#endif

#define LDR_MAX_MODULES 128
#define LDR_MAX_DEPS    48
#define LDR_NAME_MAX    64

#define LDR_REASON_PROCESS_DETACH 0
#define LDR_REASON_PROCESS_ATTACH 1
#define LDR_REASON_THREAD_ATTACH  2
#define LDR_REASON_THREAD_DETACH  3

typedef enum {
    LDR_OK = 0,
    LDR_ERR_DLL_NOT_FOUND,       /* STATUS_DLL_NOT_FOUND                    */
    LDR_ERR_BAD_IMAGE,           /* STATUS_INVALID_IMAGE_FORMAT             */
    LDR_ERR_MACHINE_MISMATCH,    /* STATUS_IMAGE_MACHINE_TYPE_MISMATCH      */
    LDR_ERR_PROC_NOT_FOUND,      /* STATUS_PROCEDURE_NOT_FOUND              */
    LDR_ERR_ORDINAL_NOT_FOUND,   /* STATUS_ORDINAL_NOT_FOUND                */
    LDR_ERR_INIT_FAILED,         /* STATUS_DLL_INIT_FAILED                  */
    LDR_ERR_CONFLICTING_ADDRESSES,
    LDR_ERR_NO_MEMORY,
    LDR_ERR_TOO_MANY_MODULES,
    LDR_ERR_INVALID_PARAMETER
} ldr_status;

typedef struct {
    ldr_status status;
    char module[LDR_NAME_MAX];   /* module that could not be loaded / did not export */
    char symbol[LDR_NAME_MAX];   /* missing symbol or "#ordinal" */
    pe_status pe;                /* detail when status == LDR_ERR_BAD_IMAGE */
} ldr_error;

struct ldr_module;
typedef struct {
    void *ctx;
    /* Locate and read a module. `name` is the normalised base name ("kernel32.dll");
     * `path` is the caller's original string when it contained a directory, else NULL.
     * On success (*data, *size) describe the file bytes, valid until release_file. */
    int  (*open_module)(void *ctx, const char *name, const char *path,
                        const void **data, size_t *size, void **cookie);
    void (*release_file)(void *ctx, void *cookie);
    /* Reserve+commit `size` writable bytes. Try `preferred`; if `fixed` is set the
     * image cannot move, so fail rather than return another address. *mem is the
     * writable view, *addr is the address the image will run at. */
    int  (*alloc_image)(void *ctx, uint64_t preferred, size_t size, int fixed, void **mem, uint64_t *addr);
    void (*free_image)(void *ctx, void *mem, uint64_t addr, size_t size);
    /* Call BOOL entry(base, reason, reserved). Returns the BOOL. */
    int  (*call_entry)(void *ctx, uint64_t fn, uint64_t base, uint32_t reason, void *reserved);
    /* Optional (may be NULL): compatibility import redirection, consulted for every
     * import before it is resolved. `app` is the process image name given to
     * ldr_load_exe, `name` is NULL for ordinal imports. Return nonzero and fill
     * to_dll/to_sym to resolve a different module/symbol (empty to_sym keeps the
     * original name/ordinal). */
    int  (*redirect_import)(void *ctx, const char *app, const char *dll, const char *name,
                            char *to_dll, size_t to_dll_sz, char *to_sym, size_t to_sym_sz);
    /* Optional: called once the process image is mapped and its imports are bound, but
     * before any DLL initialiser runs (so e.g. the compatibility layer can read the
     * manifest and fix the reported OS version first). */
    void (*exe_mapped)(void *ctx, struct ldr_module *exe);
} ldr_backend;

typedef enum { LDR_FREE = 0, LDR_MAPPED, LDR_BOUND, LDR_INITIALIZING, LDR_READY } ldr_state;

struct ldr_module {
    ldr_state state;
    char      name[LDR_NAME_MAX];     /* normalised base name */
    int       is_exe;
    void     *mem;                    /* writable view of the image */
    uint64_t  addr;                   /* run address (BaseAddress) */
    size_t    size;
    pe_image  img;                    /* mapped-layout view over mem */
    unsigned  explicit_refs;
    unsigned  ndeps;
    unsigned short deps[LDR_MAX_DEPS];
    uint32_t  tls_index;
    int       has_tls;
};
typedef struct ldr_module ldr_module;

typedef struct {
    ldr_backend be;
    int         is64;                 /* process bitness; other machines are rejected */
    ldr_module  mod[LDR_MAX_MODULES];
    unsigned short init_order[LDR_MAX_MODULES];
    unsigned    ninit;
    uint32_t    next_tls_index;
    char        app[256];             /* image name passed to ldr_load_exe */
} ldr_context;

void ldr_init(ldr_context *c, const ldr_backend *be, int is64);

/* Map the process image, load+bind+initialise its imports. The EXE's own entry
 * point is not called; use ldr_entry_point(). Takes the permanent reference. */
ldr_status ldr_load_exe(ldr_context *c, const char *name, ldr_module **out, ldr_error *err);

/* LdrLoadDll: load (or add a reference to) a DLL and everything it imports,
 * calling DllMain(PROCESS_ATTACH) dependencies-first. */
ldr_status ldr_load_dll(ldr_context *c, const char *name, ldr_module **out, ldr_error *err);

/* LdrUnloadDll: drop one explicit reference; unreachable modules are detached and freed. */
ldr_status ldr_unload_dll(ldr_context *c, ldr_module *m);

/* GetModuleHandle: no load, no reference. NULL if not loaded. Accepts API-set names. */
ldr_module *ldr_get_module(ldr_context *c, const char *name);

/* LdrGetProcedureAddress: by name (name != NULL) or ordinal. Follows forwarders. */
ldr_status ldr_get_proc(ldr_context *c, ldr_module *m, const char *name, uint16_t ordinal,
                        uint64_t *addr, ldr_error *err);

/* Run DLL_THREAD_ATTACH / DLL_THREAD_DETACH for every initialised DLL. */
void ldr_thread_notify(ldr_context *c, uint32_t reason);

/* Process exit: DLL_PROCESS_DETACH (reserved != NULL) in reverse order, nothing freed. */
void ldr_shutdown(ldr_context *c);

uint64_t ldr_entry_point(const ldr_module *m);

/* The module's embedded application manifest (RT_MANIFEST id 1), if any. The
 * pointer is into the mapped image and stays valid while the module is loaded. */
ldr_status ldr_get_manifest(const ldr_module *m, const char **xml, size_t *len);

/* NTSTATUS values from the public ntstatus.h contract. */
uint32_t ldr_status_to_ntstatus(ldr_status s);

#ifdef __cplusplus
}
#endif
#endif
