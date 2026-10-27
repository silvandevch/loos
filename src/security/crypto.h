#ifndef LOOS_CRYPTO_H
#define LOOS_CRYPTO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define LOOS_SHA256_DIGEST_SIZE 32U

#ifdef LOOS_RUST_CRYPTO
/* Backed by crates/loos-security: the context is opaque storage for the Rust hasher (size checked at Rust compile time). */
struct loos_sha256 {
    _Alignas(16) uint8_t opaque[128];
};
#else
struct loos_sha256 {
    uint32_t state[8];
    uint64_t bit_count;
    uint8_t block[64];
    size_t block_length;
};
#endif

void loos_sha256_init(struct loos_sha256 *context);
void loos_sha256_update(struct loos_sha256 *context,
                        const void *data,
                        size_t length);
void loos_sha256_final(struct loos_sha256 *context,
                       uint8_t digest[LOOS_SHA256_DIGEST_SIZE]);
void loos_sha256(const void *data,
                 size_t length,
                 uint8_t digest[LOOS_SHA256_DIGEST_SIZE]);

bool loos_constant_time_equal(const void *left, const void *right, size_t length);

#endif
