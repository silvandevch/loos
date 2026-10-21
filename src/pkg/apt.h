/* apt repository client logic (no I/O): sources.list / deb822 .sources parsing with apt's option syntax, Release file
 * parsing and validation (Date / Valid-Until / hash lists), index and pool URL construction, hash verification and
 * apt_preferences pinning. Fetching and signature checks are layered on top (see pgp.h). */
#ifndef LOOS_PKG_APT_H
#define LOOS_PKG_APT_H
#include "deb.h"
#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    bool is_src;                      /* deb-src */
    char uri[256], suite[96];
    char comp[8][32]; int ncomp;
    char arch[4][16]; int narch;      /* empty = the native architecture only */
    char signed_by[160];              /* keyring path or fingerprint list; empty = the default trusted keyring */
    bool by_hash;                     /* Acquire-By-Hash */
    bool trusted_override;            /* [trusted=yes]: refused by policy, kept so the UI can explain */
    bool check_date;                  /* default true */
    bool enabled;
} apt_source;

int  apt_parse_sources_list(const char *text, apt_source *out, int max);   /* "deb [arch=amd64 signed-by=/k.gpg] https://h/ubuntu jammy main universe" */
int  apt_parse_deb822(const char *text, apt_source *out, int max);         /* Types/URIs/Suites/Components/Architectures/Signed-By/Enabled */
typedef struct { bool allow_http_local; bool allow_http; bool allow_unsigned; } apt_policy;
bool apt_source_acceptable(const apt_source *s, const apt_policy *pol, char *why, size_t cap);   /* https only (plain http for a private network if allowed); never [trusted=yes] */
size_t apt_index_url(const apt_source *s, const char *comp, const char *arch, const char *name, char *out, size_t cap);   /* <uri>/dists/<suite>/<comp>/binary-<arch>/<name> */
size_t apt_release_url(const apt_source *s, const char *file, char *out, size_t cap);                                      /* <uri>/dists/<suite>/<file> */
size_t apt_pool_url(const apt_source *s, const char *filename, char *out, size_t cap);

typedef struct { char path[192]; char sha256[65]; uint64_t size; } apt_release_file;
typedef struct {
    char origin[64], label[64], suite[64], codename[64], version[32];
    char date[40], valid_until[40]; time_t date_t, valid_until_t;
    char archs[16][16]; int narch;
    char comps[16][32]; int ncomp;
    bool by_hash;
    apt_release_file *f; int nf;
} apt_release;
bool apt_release_parse(const char *text, size_t len, apt_release *out);
void apt_release_free(apt_release *r);
const apt_release_file *apt_release_find(const apt_release *r, const char *path);
typedef enum { APT_REL_OK, APT_REL_FROM_FUTURE, APT_REL_EXPIRED, APT_REL_WRONG_SUITE, APT_REL_NO_HASHES } apt_release_status;
apt_release_status apt_release_check(const apt_release *r, const apt_source *s, time_t now, long max_skew_s);
time_t apt_parse_date(const char *s);                                       /* RFC 2822 "Thu, 18 Oct 2026 12:00:00 UTC" */
bool apt_verify_file(const apt_release_file *e, const void *data, size_t len);            /* size and SHA-256 */
size_t apt_best_index(const apt_release *r, const char *comp, const char *arch, char *name, size_t cap, char *path_out, size_t pcap);   /* picks Packages.xz > .gz > plain that the Release lists */
bool apt_verify_deb_hash(const deb_pkg *p, const void *data, size_t len);                 /* pool file against the Packages entry */

/* apt_preferences: "Package: foo*\nPin: release a=stable, o=Ubuntu\nPin-Priority: 990" */
typedef struct { char pattern[64]; char release[128]; char version[64]; int priority; } apt_pin;
typedef struct { apt_pin pin[32]; int n; } apt_prefs;
bool apt_prefs_parse(const char *text, size_t len, apt_prefs *out);
int  apt_prefs_priority(const apt_prefs *p, const char *pkg, const char *version, const apt_release *rel, bool installed);   /* 500 default, 100 installed */

#ifdef __cplusplus
}
#endif
#endif
