/* The dpkg database: /var/lib/dpkg/status records (with dpkg's want/flag/status triple), auto-installed marks
 * (extended_states), per-package file lists and file ownership with Replaces / diversion semantics. */
#ifndef LOOS_PKG_DPKGDB_H
#define LOOS_PKG_DPKGDB_H
#include "deb.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum { WANT_UNKNOWN, WANT_INSTALL, WANT_HOLD, WANT_DEINSTALL, WANT_PURGE } dpkg_want;
typedef enum { FLAG_OK, FLAG_REINSTREQ } dpkg_flag;
typedef enum { ST_NOT_INSTALLED, ST_CONFIG_FILES, ST_HALF_INSTALLED, ST_UNPACKED, ST_HALF_CONFIGURED, ST_TRIGGERS_AWAITED, ST_TRIGGERS_PENDING, ST_INSTALLED } dpkg_status;

typedef struct {
    deb_pkg pkg;
    dpkg_want want; dpkg_flag flag; dpkg_status status;
    bool auto_installed;
    char **files; int nfiles;                 /* paths owned (as in info/<pkg>.list) */
} dpkg_entry;

typedef struct {
    dpkg_entry *e; int n, cap;
    char native[16];
    struct { char *path; char *from, *to_pkg; } *diversion; int ndiv;    /* "path diverted to ... by pkg" */
} dpkg_db;

void dpkg_db_init(dpkg_db *db, const char *native_arch);
void dpkg_db_free(dpkg_db *db);
int  dpkg_db_load_status(dpkg_db *db, const char *text, size_t len);         /* entries read, -1 on a malformed stanza */
size_t dpkg_db_save_status(const dpkg_db *db, char *out, size_t cap);
void dpkg_db_load_extended_states(dpkg_db *db, const char *text, size_t len);
size_t dpkg_db_save_extended_states(const dpkg_db *db, char *out, size_t cap);
dpkg_entry *dpkg_db_find(dpkg_db *db, const char *name, const char *arch);   /* arch NULL = any */
const dpkg_entry *dpkg_db_find_const(const dpkg_db *db, const char *name, const char *arch);
bool dpkg_entry_installed(const dpkg_entry *e);                              /* unpacked or beyond: its files are on disk */
dpkg_entry *dpkg_db_put(dpkg_db *db, const deb_pkg *pkg, dpkg_status st);    /* copies; replaces an existing entry of the same name+arch */
bool dpkg_db_remove(dpkg_db *db, const char *name, const char *arch, bool purge);   /* purge drops the record, remove keeps config-files */
bool dpkg_db_set_files(dpkg_db *db, const char *name, const char *arch, const char *const *paths, int n);
int  dpkg_db_load_list(dpkg_db *db, const char *name, const char *arch, const char *text);   /* info/<pkg>.list */
size_t dpkg_db_save_list(const dpkg_db *db, const char *name, const char *arch, char *out, size_t cap);
const dpkg_entry *dpkg_db_owner(const dpkg_db *db, const char *path);       /* the package that owns this file, or NULL */

typedef struct { const char *path; const char *owner; } dpkg_file_conflict;
/* Files `pkg` may not install: owned by another package that `pkg` does not replace. Returns the count (<= max). */
int  dpkg_db_check_files(const dpkg_db *db, const deb_pkg *pkg, const char *const *paths, int n, dpkg_file_conflict *out, int max);
void dpkg_db_add_diversion(dpkg_db *db, const char *path, const char *diverted_to, const char *by_pkg);
/* Installed packages with unmet dependencies/conflicts; fills `out` with "pkg: reason" strings (malloc'd by caller-sized buffers). */
int  dpkg_db_audit(const dpkg_db *db, char out[][160], int max);

#ifdef __cplusplus
}
#endif
#endif
