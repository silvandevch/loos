/* LoOS application-compatibility core: claimed-version resolution, per-app
 * profiles (the store behind Compat.getAppProfile / setClaimedVersion) and
 * import-redirect rules.
 *
 * Pure C, no allocation, no OS dependency. Persisting the profile text and
 * delivering the resolved version to a new process (PEB) is the caller's job.
 *
 * Version policy (docs/ARCHITECTURE.md, "no lying to the point of breakage"):
 *   1. a per-app profile set by the user wins;
 *   2. otherwise the highest <supportedOS> in the application manifest;
 *   3. otherwise the database default (Windows 11 out of the box).
 * A manifest naming only the Win10/11 GUID gets the native (newest) version.
 */
#ifndef LOOS_COMPAT_H
#define LOOS_COMPAT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    COMPAT_ERA_WIN7 = 0,
    COMPAT_ERA_WIN8,
    COMPAT_ERA_WIN81,
    COMPAT_ERA_WIN10,
    COMPAT_ERA_WIN11,
    COMPAT_ERA_COUNT
} compat_era;

#define COMPAT_NATIVE_ERA COMPAT_ERA_WIN11

typedef struct {
    compat_era era;
    const char *name;            /* "win7" ... "win11" */
    uint32_t major, minor, build;
    uint16_t sp_major, sp_minor, suite_mask;
    uint8_t  product_type;       /* VER_NT_WORKSTATION */
    uint32_t platform_id;        /* VER_PLATFORM_WIN32_NT */
    const char *csd;             /* "Service Pack 1" or "" */
} compat_version;

const compat_version *compat_version_of(compat_era e);
/* "win7", "7", "windows 7", "8.1", "win10", "11" ... (case-insensitive). 1 on success. */
int compat_era_parse(const char *s, compat_era *out);

typedef enum { COMPAT_SRC_DEFAULT = 0, COMPAT_SRC_MANIFEST, COMPAT_SRC_PROFILE } compat_source;

typedef struct {
    compat_era era;
    compat_source source;
    char reason[96];             /* human-readable "why", shown in the compat console */
} compat_claim;

typedef enum { COMPAT_OK = 0, COMPAT_ERR_SYNTAX, COMPAT_ERR_FULL, COMPAT_ERR_ERA, COMPAT_ERR_NAME, COMPAT_ERR_RANGE } compat_status;

#define COMPAT_MAX_APPS      128
#define COMPAT_MAX_REDIRECTS 64
#define COMPAT_NAME_MAX      96
#define COMPAT_SYM_MAX       64

typedef struct { char name[COMPAT_NAME_MAX]; compat_era era; } compat_app;

typedef struct {
    char app[COMPAT_NAME_MAX];   /* app key or "*" */
    char from_dll[COMPAT_SYM_MAX], from_sym[COMPAT_SYM_MAX];   /* from_sym "" = whole DLL */
    char to_dll[COMPAT_SYM_MAX],   to_sym[COMPAT_SYM_MAX];
} compat_redirect;

typedef struct {
    compat_era default_era;
    unsigned napps;       compat_app apps[COMPAT_MAX_APPS];
    unsigned nredirects;  compat_redirect redirects[COMPAT_MAX_REDIRECTS];
} compat_db;

void compat_db_init(compat_db *db);

/* Text store. Lines: `default <era>` | `app "<name-or-path>" <era>` |
 * `redirect <app|*> <dll>[!<sym>] <dll>[!<sym>]`, '#' comments. On error
 * *err_line is the 1-based line. The db is left unchanged on error. */
compat_status compat_db_parse(compat_db *db, const char *text, size_t len, unsigned *err_line);
size_t compat_db_dump(const compat_db *db, char *out, size_t cap);       /* returns bytes needed (excl. NUL) */

/* Compat.setClaimedVersion / getAppProfile / clear. `app` is an exe name or full path. */
compat_status compat_db_set_claim(compat_db *db, const char *app, compat_era era);
void compat_db_clear_claim(compat_db *db, const char *app);
const compat_app *compat_db_get_app(const compat_db *db, const char *app);
compat_status compat_db_add_redirect(compat_db *db, const char *app, const char *from_dll, const char *from_sym,
                                     const char *to_dll, const char *to_sym);

/* Highest <supportedOS> era in an application manifest. 1 if the manifest lists any known OS. */
int compat_manifest_era(const char *xml, size_t len, compat_era *out);

/* Resolve the claim for a process. `manifest` may be NULL. */
void compat_resolve(const compat_db *db, const char *exe_path, const char *manifest, size_t manifest_len, compat_claim *out);

/* Import redirection. `sym` is NULL for ordinal imports. Returns 1 and fills to_* when a rule applies. */
int compat_redirect_import(const compat_db *db, const char *app, const char *dll, const char *sym,
                           char *to_dll, size_t to_dll_sz, char *to_sym, size_t to_sym_sz);

#ifdef __cplusplus
}
#endif
#endif
