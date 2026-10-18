/* LoOS PE/COFF image parser, relocator and import binder.
 *
 * Pure C99, no allocation, no OS calls: runs in the loader, in the kernel's
 * SEC_IMAGE path, and on the host for tests. Every offset read from the image
 * is bounds-checked against the buffer; the image is untrusted input.
 *
 * Two buffer layouts are supported:
 *   PE_LAYOUT_FILE   raw file bytes (sections at PointerToRawData)
 *   PE_LAYOUT_MAPPED sections already placed at their RVAs (SizeOfImage bytes)
 * pe_map() converts the first into the second. Relocation and binding only
 * operate on mapped images.
 */
#ifndef LOOS_PE_H
#define LOOS_PE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    PE_OK = 0,
    PE_ERR_TRUNCATED,      /* header/table runs past the buffer          */
    PE_ERR_BAD_MAGIC,      /* not MZ / PE / known optional-header magic  */
    PE_ERR_UNSUPPORTED,    /* machine or format we don't handle          */
    PE_ERR_BAD_FORMAT,     /* internally inconsistent header              */
    PE_ERR_BAD_RVA,        /* an RVA doesn't land inside the image        */
    PE_ERR_WRONG_LAYOUT,   /* operation needs the other layout            */
    PE_ERR_BUFFER_TOO_SMALL,
    PE_ERR_NO_RELOCS,      /* needs rebasing but image is stripped of relocs */
    PE_ERR_BAD_RELOC,
    PE_ERR_APISET,         /* api-ms-/ext-ms- import not in schema        */
    PE_ERR_UNRESOLVED,     /* resolver callback failed                    */
    PE_ERR_NOT_FOUND       /* export lookup miss                          */
} pe_status;

typedef enum { PE_LAYOUT_FILE = 0, PE_LAYOUT_MAPPED = 1 } pe_layout;

#define PE_MACHINE_I386  0x014c
#define PE_MACHINE_AMD64 0x8664

#define PE_DIR_EXPORT    0
#define PE_DIR_IMPORT    1
#define PE_DIR_RESOURCE  2
#define PE_DIR_BASERELOC 5
#define PE_DIR_TLS       9
#define PE_DIR_BOUND_IMPORT 11
#define PE_DIR_IAT       12
#define PE_DIR_DELAY_IMPORT 13
#define PE_DIR_COM_DESCRIPTOR 14

#define PE_FILE_DLL              0x2000
#define PE_FILE_RELOCS_STRIPPED  0x0001

#define PE_SCN_MEM_EXECUTE 0x20000000u
#define PE_SCN_MEM_READ    0x40000000u
#define PE_SCN_MEM_WRITE   0x80000000u

typedef struct { uint32_t rva, size; } pe_dir;

typedef struct {
    char     name[9];            /* NUL-terminated copy */
    uint32_t virtual_address;
    uint32_t virtual_size;
    uint32_t raw_offset;
    uint32_t raw_size;
    uint32_t characteristics;
} pe_section;

typedef struct {
    uint8_t *base;               /* buffer (writable for mapped images) */
    size_t   size;
    pe_layout layout;

    uint16_t machine;
    uint16_t file_characteristics;
    uint16_t dll_characteristics;
    int      is64;
    uint64_t image_base;
    uint32_t entry_rva;
    uint32_t section_align, file_align;
    uint32_t size_of_image, size_of_headers;
    uint16_t nsections;
    pe_dir   dir[16];

    size_t   sect_off;           /* offset of first section header in buffer */
    size_t   image_base_off;     /* offset of ImageBase field (for rebase)   */
} pe_image;

/* Parse and validate headers. Does not modify the buffer. */
pe_status pe_open(pe_image *img, void *buf, size_t size, pe_layout layout);

pe_status pe_section_at(const pe_image *img, unsigned idx, pe_section *out);

/* Pointer to `len` readable bytes at `rva`, or NULL if any byte falls outside
 * the image (or, in file layout, outside the section's raw data). */
uint8_t *pe_rva_ptr(const pe_image *img, uint32_t rva, size_t len);

/* Copy a NUL-terminated string at rva into out (max outsz incl. NUL). */
pe_status pe_rva_string(const pe_image *img, uint32_t rva, char *out, size_t outsz);

/* File layout -> mapped layout into dst[dst_size >= size_of_image]; dst is zeroed first.
 * On success `mapped` is an opened mapped-layout image over dst. */
pe_status pe_map(const pe_image *file, void *dst, size_t dst_size, pe_image *mapped);

/* Rebase a mapped image so it believes it lives at new_base. Updates the
 * header's ImageBase. A no-op when new_base == image_base. */
pe_status pe_relocate(pe_image *img, uint64_t new_base);

/* ---- exports ---- */
typedef struct {
    uint32_t rva;                /* function RVA (0 for pure forwarder) */
    char     forwarder[128];     /* "DLL.Name" or "DLL.#12" if forwarded, else "" */
} pe_export;

/* Look up by name (name != NULL) or by ordinal (name == NULL). */
pe_status pe_find_export(const pe_image *img, const char *name, uint16_t ordinal, pe_export *out);

/* Visit every exported function. name is NULL for ordinal-only exports; forwarder is NULL
 * unless the export is a forwarder string (then rva is 0). visit returns nonzero to stop. */
pe_status pe_enum_exports(const pe_image *img,
                          int (*visit)(void *ctx, const char *name, uint16_t ordinal, uint32_t rva, const char *forwarder),
                          void *ctx);

/* ---- imports ---- */
/* Pointers are valid only for the duration of the callback. */
typedef struct {
    const char *dll;             /* name exactly as imported */
    const char *host;            /* LoOS module after API-set resolution (lowercased) */
    const char *name;            /* NULL when imported by ordinal */
    uint16_t    ordinal;         /* ordinal, or hint when `name` != NULL */
} pe_import_ref;

/* Resolver supplies the address of one imported symbol. Return 0 on success. */
typedef int (*pe_resolver)(void *ctx, const pe_import_ref *ref, uint64_t *addr);

typedef struct {
    pe_status status;
    char dll[64];
    char symbol[64];             /* name or "#ordinal" */
} pe_bind_error;

/* Walk the import table of a mapped image, resolve every symbol through
 * `resolve`, and write the addresses into the IAT. Import-descriptor DLL names
 * are run through the API-set schema first. On failure err (optional) says
 * which import broke. IAT slots already written are not rolled back. */
pe_status pe_bind_imports(pe_image *img, pe_resolver resolve, void *ctx, pe_bind_error *err);

/* Visit imports without binding (read-only; works on either layout). */
pe_status pe_enum_imports(const pe_image *img, int (*visit)(void *ctx, const pe_import_ref *ref), void *ctx);

/* Same as above for the delay-load table (IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT).
 * Binding eagerly fills the delay IAT; a loader that wants lazy resolution just
 * enumerates instead. Both RVA-based and legacy VA-based descriptors work. */
pe_status pe_bind_delay_imports(pe_image *img, pe_resolver resolve, void *ctx, pe_bind_error *err);
pe_status pe_enum_delay_imports(const pe_image *img, int (*visit)(void *ctx, const pe_import_ref *ref), void *ctx);

/* Bound-import directory: informational only. LoOS always rewrites the IAT, so
 * stale bindings can never be trusted by accident. visit returns nonzero to stop. */
pe_status pe_enum_bound_imports(const pe_image *img,
                                int (*visit)(void *ctx, const char *dll, uint32_t timestamp, unsigned nforwarders),
                                void *ctx);

/* ---- TLS ---- */
#define PE_MAX_TLS_CALLBACKS 32
typedef struct {
    uint32_t raw_rva, raw_size;   /* initialisation template (may be empty)   */
    uint32_t zero_fill;           /* extra zeroed bytes after the template    */
    uint32_t index_rva;           /* where the loader stores the TLS index    */
    uint32_t alignment;           /* bytes, power of two                      */
    unsigned ncallbacks;
    uint32_t callback_rva[PE_MAX_TLS_CALLBACKS];
} pe_tls;

/* VAs in the directory are converted to RVAs against the image's *current*
 * image_base, so call after pe_relocate. PE_ERR_NOT_FOUND = no TLS directory. */
pe_status pe_get_tls(const pe_image *img, pe_tls *t);

/* Write one thread's initial TLS block (template + zero fill) into dst. Total
 * size is raw_size + zero_fill. */
pe_status pe_tls_init_block(const pe_image *img, const pe_tls *t, void *dst, size_t dst_size);

/* ---- resources ---- */
#define PE_RT_MANIFEST 24
/* Locate the data of resource (type, id) - first language. On success (*rva, *size) is the raw data. */
pe_status pe_find_resource(const pe_image *img, uint32_t type, uint32_t id, uint32_t *rva, uint32_t *size);

const char *pe_status_str(pe_status s);

#ifdef __cplusplus
}
#endif
#endif
