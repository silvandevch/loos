#include "apiset.h"
#include "apiset_table.h"
#include <string.h>

#define NAME_MAX_LEN 128

static char lc(char c) { return (c >= 'A' && c <= 'Z') ? (char)(c + 32) : c; }

int apiset_is_apiset(const char *n)
{
    static const char a[] = "api-ms-", e[] = "ext-ms-";
    size_t i;
    if (!n) return 0;
    for (i = 0; a[i]; i++) if (lc(n[i]) != a[i]) break;
    if (!a[i]) return 1;
    for (i = 0; e[i]; i++) if (lc(n[i]) != e[i]) break;
    return !e[i];
}

/* Normalise into key[]: lowercase, strip ".dll", strip trailing "-<digits>".
 * Returns key length or 0 if malformed. */
static size_t make_key(const char *name, char *key)
{
    size_t n = 0, i;
    for (; name[n]; n++) {
        if (n + 1 >= NAME_MAX_LEN) return 0;
        key[n] = lc(name[n]);
    }
    key[n] = 0;
    if (n > 4 && !memcmp(key + n - 4, ".dll", 4)) key[n -= 4] = 0;
    /* trailing revision: "-<digits>" */
    i = n;
    while (i > 0 && key[i - 1] >= '0' && key[i - 1] <= '9') i--;
    if (i == n || i == 0 || key[i - 1] != '-') return 0;
    key[i - 1] = 0;
    return i - 1;
}

apiset_status apiset_resolve(const char *name, char *out, size_t outsz)
{
    char key[NAME_MAX_LEN];
    size_t lo = 0, hi = apiset_table_len, klen;

    if (!name || !*name) return APISET_BAD_NAME;
    if (!apiset_is_apiset(name)) return APISET_NOT_AN_APISET;
    klen = make_key(name, key);
    if (!klen) return APISET_BAD_NAME;

    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        int c = strcmp(key, apiset_table[mid].contract);
        if (!c) {
            size_t hl = strlen(apiset_table[mid].host);
            if (hl + 1 > outsz) return APISET_BUFFER_TOO_SMALL;
            memcpy(out, apiset_table[mid].host, hl + 1);
            return APISET_OK;
        }
        if (c < 0) hi = mid; else lo = mid + 1;
    }
    return APISET_UNKNOWN;
}

size_t apiset_count(void) { return apiset_table_len; }

const char *apiset_contract_at(size_t i, const char **host)
{
    if (i >= apiset_table_len) return NULL;
    if (host) *host = apiset_table[i].host;
    return apiset_table[i].contract;
}
