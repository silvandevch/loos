/* OpenPGP (RFC 4880/9580) signature verification for repository metadata: RSA keys of 2048..4096 bits, SHA-256/384/512
 * signatures, armored or binary keyrings, clear-signed (InRelease) and detached (Release.gpg) signatures. SHA-1
 * signatures are refused. Key expiry, signature expiry and key flags are honoured; EdDSA/ECDSA keys are reported as
 * unsupported rather than ignored. Verification never trusts anything outside the keyring it was given. */
#ifndef LOOS_PKG_PGP_H
#define LOOS_PKG_PGP_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    uint8_t keyid[8], fpr[20];
    uint8_t algo;                        /* 1 = RSA */
    uint32_t created;
    uint32_t expires_after;              /* seconds after creation, 0 = never */
    bool can_sign, is_subkey, revoked;
    uint8_t *n, *e; size_t nlen, elen;   /* big-endian magnitudes */
    int primary;                         /* index of the primary key for a subkey (expiry and revocation inherit) */
} pgp_key;
typedef struct { pgp_key *k; int n, cap; int min_bits; } pgp_keyring;

void pgp_keyring_init(pgp_keyring *r);                              /* min_bits defaults to 2048 */
void pgp_keyring_free(pgp_keyring *r);
int  pgp_keyring_add(pgp_keyring *r, const uint8_t *data, size_t len);   /* armored or binary; returns keys added, -1 on malformed input */
void pgp_fingerprint_hex(const uint8_t fpr[20], char out[41]);
const pgp_key *pgp_keyring_find(const pgp_keyring *r, const uint8_t keyid[8]);

typedef enum {
    PGP_OK = 0, PGP_BAD_SIGNATURE, PGP_NO_KEY, PGP_KEY_EXPIRED, PGP_KEY_REVOKED, PGP_SIG_EXPIRED, PGP_SIG_FROM_FUTURE,
    PGP_UNSUPPORTED, PGP_WEAK_HASH, PGP_WEAK_KEY, PGP_MALFORMED, PGP_NOT_A_SIGNER
} pgp_result;
typedef struct { pgp_result result; uint8_t keyid[8]; uint8_t fpr[20]; bool have_fpr; uint32_t sig_created; uint8_t hash_algo; } pgp_verdict;
const char *pgp_result_text(pgp_result r);

pgp_result pgp_verify_detached(const pgp_keyring *ring, const uint8_t *sig, size_t sig_len, const uint8_t *data, size_t data_len, time_t now, pgp_verdict *v);
/* Verifies a clear-signed document. On success `content` receives the *signed text* (dash-unescaped, LF line ends, without the
 * armor): callers must parse that, never the surrounding file. content_cap must be >= text length. */
pgp_result pgp_verify_clearsigned(const pgp_keyring *ring, const char *text, size_t len, time_t now, pgp_verdict *v, char *content, size_t content_cap, size_t *content_len);

/* building blocks, exposed for tests and for other signature users (package trust) */
int  pgp_dearmor(const char *in, size_t len, uint8_t *out, size_t cap, char *label, size_t label_cap);   /* decoded length or -1; verifies the CRC-24 when present */
bool pgp_rsa_public(const uint8_t *n, size_t nlen, const uint8_t *e, size_t elen, const uint8_t *in, size_t inlen, uint8_t *out, size_t outlen);

#ifdef __cplusplus
}
#endif
#endif
