/* Test-only synthetic PE builder. Produces small but structurally real PE32/PE32+
 * images (headers, .text/.rdata/.reloc, imports, exports, forwarders, relocs, TLS). */
#ifndef LOOS_TEST_PEBUILD_H
#define LOOS_TEST_PEBUILD_H
#include <stddef.h>
#include <stdint.h>

#define BMAX_IMP 6
#define BMAX_SYM 8
#define BMAX_EXP 12

typedef struct { const char *name; const char *forward; } bexport;   /* forward: "DLL.Func" / "DLL.#N" or NULL */
typedef struct { const char *dll; const char *syms[BMAX_SYM + 1]; } bimport; /* syms NULL-terminated; "#7" = ordinal */

typedef struct {
    int is64;
    int is_dll;             /* else EXE */
    int entry;              /* nonzero: entry point at rva 0x1000 */
    int relocs;             /* emit a reloc table; else RELOCS_STRIPPED (unless pic) */
    int pic;                /* no reloc table but not stripped: position independent */
    int tls;                /* add a TLS directory with one callback at rva 0x1010 */
    const char *manifest;   /* if set: embed as RT_MANIFEST id 1 */
    uint64_t base;          /* preferred ImageBase */
    int nexports;           bexport exports[BMAX_EXP];   /* ordinal = index + 1 */
    int nimports;           bimport imports[BMAX_IMP];
} bspec;

typedef struct {
    uint8_t *data; size_t size;
    uint32_t iat_rva[BMAX_IMP][BMAX_SYM];
} bimage;

#define B_FUNC_RVA(i)   (0x1000u + 0x10u * (unsigned)(i))     /* export i (non-forwarded) */
#define B_PTR_RVA       0x1100u                               /* a relocated pointer to B_FUNC_RVA(0) */
#define B_TLS_INDEX_RVA 0x1190u

void bimage_build(const bspec *s, bimage *out);               /* malloc'd; free with bimage_free */
void bimage_free(bimage *b);
#endif
