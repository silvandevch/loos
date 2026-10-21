/* HMAC-SHA256 and PBKDF2-HMAC-SHA256 (RFC 2104 / RFC 8018) on top of the LoOS SHA-256. Used for local account
 * passwords; also the building block for key derivation in disk encryption and TLS. */
#ifndef LOOS_KDF_H
#define LOOS_KDF_H
#include "crypto.h"

#ifdef __cplusplus
extern "C" {
#endif

struct loos_hmac_sha256 {
    struct loos_sha256 inner, outer;
};
void loos_hmac_sha256_init(struct loos_hmac_sha256 *ctx, const void *key, size_t key_len);
void loos_hmac_sha256_update(struct loos_hmac_sha256 *ctx, const void *data, size_t len);
void loos_hmac_sha256_final(struct loos_hmac_sha256 *ctx, uint8_t mac[LOOS_SHA256_DIGEST_SIZE]);
void loos_hmac_sha256(const void *key, size_t key_len, const void *data, size_t len, uint8_t mac[LOOS_SHA256_DIGEST_SIZE]);

/* Derives out_len bytes (any length up to 2^32-1 blocks). iterations must be >= 1. Returns false on bad arguments. */
bool loos_pbkdf2_sha256(const void *password, size_t password_len, const void *salt, size_t salt_len,
                        uint32_t iterations, uint8_t *out, size_t out_len);

#ifdef __cplusplus
}
#endif
#endif
