/* Debian package metadata: version ordering (Debian Policy 5.6.12), control-file stanzas, dependency
 * expressions, and the installed/available package record used by the dpkg database and the resolver.
 * Plain C11 with malloc: pkgd is a user-space service. */
#ifndef LOOS_PKG_DEB_H
#define LOOS_PKG_DEB_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- versions ---- */
int deb_version_compare(const char *a, const char *b);            /* <0, 0, >0 exactly like dpkg --compare-versions */
bool deb_version_valid(const char *v);

/* ---- stanzas (RFC 822 style, as in Packages, status, Release) ---- */
typedef struct { char *key; char *value; } deb_field;               /* continuation lines joined by '\n', leading space stripped */
typedef struct { deb_field *f; int n, cap; } deb_stanza;
const char *deb_stanza_get(const deb_stanza *s, const char *key);   /* case-insensitive */
void deb_stanza_set(deb_stanza *s, const char *key, const char *value);
void deb_stanza_free(deb_stanza *s);
size_t deb_stanza_format(const deb_stanza *s, char *out, size_t cap);   /* round-trippable text incl. trailing blank line */
/* Parses the next stanza starting at *pos. Returns 1 on success, 0 at end of input, -1 on a malformed line. */
int deb_stanza_next(const char *text, size_t len, size_t *pos, deb_stanza *out);

/* ---- dependencies ---- */
typedef enum { DREL_NONE, DREL_LT, DREL_LE, DREL_EQ, DREL_GE, DREL_GT } deb_rel;
typedef struct { char *name; char *arch; deb_rel rel; char *version; } deb_atom;     /* arch: "" | "any" | "amd64" (from name:arch) */
typedef struct { deb_atom *alt; int nalt; } deb_or;
typedef struct { deb_or *grp; int n; } deb_deps;
bool deb_deps_parse(const char *text, const char *native_arch, deb_deps *out);       /* drops atoms whose [arch] list excludes native_arch */
void deb_deps_free(deb_deps *d);
bool deb_rel_satisfied(deb_rel rel, const char *want, const char *have);
size_t deb_deps_format(const deb_deps *d, char *out, size_t cap);

/* ---- package record ---- */
typedef struct {
    char *name, *version, *arch, *source, *section, *priority, *maintainer, *filename, *sha256, *description, *homepage;
    uint64_t size, installed_size;
    bool essential, multiarch_same;
    deb_deps depends, pre_depends, recommends, suggests, conflicts, breaks, replaces;
    struct { char *name; char *version; } *provides; int nprovides;
} deb_pkg;
bool deb_pkg_from_stanza(const deb_stanza *s, const char *native_arch, deb_pkg *out);
void deb_pkg_free(deb_pkg *p);
bool deb_arch_ok(const char *pkg_arch, const char *native_arch);                    /* native, "all" */

#ifdef __cplusplus
}
#endif
#endif
