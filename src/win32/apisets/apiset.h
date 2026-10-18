/* LoOS API-set resolver.
 *
 * Modern Windows binaries import contract names ("api-ms-win-core-file-l1-2-1.dll")
 * rather than real DLLs. This resolves a contract name to the LoOS host module
 * that implements it. Resolution rule (matches Windows behaviour as documented):
 * lowercase, drop ".dll", drop the trailing "-<minor>" revision, look the rest
 * up in the schema. The revision never affects the result.
 *
 * Pure C, no allocation, no OS dependency: usable from the loader, from tests
 * on the host, and from the fuzz harness.
 */
#ifndef LOOS_APISET_H
#define LOOS_APISET_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    APISET_OK = 0,
    APISET_NOT_AN_APISET,   /* name has no api-ms-/ext-ms- prefix: a real DLL name */
    APISET_UNKNOWN,         /* has the prefix but contract is not in the schema    */
    APISET_BAD_NAME,        /* malformed (empty, too long, no revision suffix)     */
    APISET_BUFFER_TOO_SMALL
} apiset_status;

/* Resolve `name` (with or without ".dll", any case) to a host module name
 * (e.g. "kernelbase.dll") written into out[outsz]. */
apiset_status apiset_resolve(const char *name, char *out, size_t outsz);

/* True when `name` starts with "api-ms-" or "ext-ms-" (case-insensitive). */
int apiset_is_apiset(const char *name);

/* Schema introspection (for the compat dashboard and tests). */
size_t apiset_count(void);
const char *apiset_contract_at(size_t i, const char **host);

#ifdef __cplusplus
}
#endif
#endif
