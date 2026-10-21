#include "kdf.h"
#include <string.h>

void loos_hmac_sha256_init(struct loos_hmac_sha256 *ctx, const void *key, size_t key_len)
{
    uint8_t k[64], pad[64];
    memset(k, 0, sizeof k);
    if (key_len > sizeof k) loos_sha256(key, key_len, k);                 /* long keys are hashed first */
    else if (key_len) memcpy(k, key, key_len);
    for (size_t i = 0; i < 64; i++) pad[i] = (uint8_t)(k[i] ^ 0x36);
    loos_sha256_init(&ctx->inner); loos_sha256_update(&ctx->inner, pad, 64);
    for (size_t i = 0; i < 64; i++) pad[i] = (uint8_t)(k[i] ^ 0x5c);
    loos_sha256_init(&ctx->outer); loos_sha256_update(&ctx->outer, pad, 64);
    memset(k, 0, sizeof k); memset(pad, 0, sizeof pad);
}
void loos_hmac_sha256_update(struct loos_hmac_sha256 *ctx, const void *data, size_t len) { loos_sha256_update(&ctx->inner, data, len); }
void loos_hmac_sha256_final(struct loos_hmac_sha256 *ctx, uint8_t mac[LOOS_SHA256_DIGEST_SIZE])
{
    uint8_t ih[LOOS_SHA256_DIGEST_SIZE];
    loos_sha256_final(&ctx->inner, ih);
    loos_sha256_update(&ctx->outer, ih, sizeof ih);
    loos_sha256_final(&ctx->outer, mac);
    memset(ih, 0, sizeof ih);
}
void loos_hmac_sha256(const void *key, size_t kl, const void *data, size_t len, uint8_t mac[LOOS_SHA256_DIGEST_SIZE])
{
    struct loos_hmac_sha256 c;
    loos_hmac_sha256_init(&c, key, kl);
    loos_hmac_sha256_update(&c, data, len);
    loos_hmac_sha256_final(&c, mac);
}

bool loos_pbkdf2_sha256(const void *pw, size_t pwl, const void *salt, size_t sl, uint32_t iters, uint8_t *out, size_t outl)
{
    if (!iters || (!out && outl) || (!salt && sl)) return false;
    if (outl / LOOS_SHA256_DIGEST_SIZE >= 0xffffffffu) return false;
    /* the keyed state is computed once and copied per iteration, which is where PBKDF2's time goes */
    struct loos_hmac_sha256 base;
    loos_hmac_sha256_init(&base, pw, pwl);
    for (uint32_t block = 1; outl; block++) {
        uint8_t u[LOOS_SHA256_DIGEST_SIZE], t[LOOS_SHA256_DIGEST_SIZE], idx[4] = { (uint8_t)(block >> 24), (uint8_t)(block >> 16), (uint8_t)(block >> 8), (uint8_t)block };
        struct loos_hmac_sha256 c = base;
        if (sl) loos_hmac_sha256_update(&c, salt, sl);
        loos_hmac_sha256_update(&c, idx, 4);
        loos_hmac_sha256_final(&c, u);
        memcpy(t, u, sizeof t);
        for (uint32_t i = 1; i < iters; i++) {
            c = base;
            loos_hmac_sha256_update(&c, u, sizeof u);
            loos_hmac_sha256_final(&c, u);
            for (size_t k = 0; k < sizeof t; k++) t[k] ^= u[k];
        }
        size_t n = outl < sizeof t ? outl : sizeof t;
        memcpy(out, t, n);
        out += n; outl -= n;
        memset(u, 0, sizeof u); memset(t, 0, sizeof t);
    }
    memset(&base, 0, sizeof base);
    return true;
}
