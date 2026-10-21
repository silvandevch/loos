#include "crypto.h"

#include <string.h>

static const uint32_t round_constants[64] = {
    0x428a2f98U, 0x71374491U, 0xb5c0fbcfU, 0xe9b5dba5U,
    0x3956c25bU, 0x59f111f1U, 0x923f82a4U, 0xab1c5ed5U,
    0xd807aa98U, 0x12835b01U, 0x243185beU, 0x550c7dc3U,
    0x72be5d74U, 0x80deb1feU, 0x9bdc06a7U, 0xc19bf174U,
    0xe49b69c1U, 0xefbe4786U, 0x0fc19dc6U, 0x240ca1ccU,
    0x2de92c6fU, 0x4a7484aaU, 0x5cb0a9dcU, 0x76f988daU,
    0x983e5152U, 0xa831c66dU, 0xb00327c8U, 0xbf597fc7U,
    0xc6e00bf3U, 0xd5a79147U, 0x06ca6351U, 0x14292967U,
    0x27b70a85U, 0x2e1b2138U, 0x4d2c6dfcU, 0x53380d13U,
    0x650a7354U, 0x766a0abbU, 0x81c2c92eU, 0x92722c85U,
    0xa2bfe8a1U, 0xa81a664bU, 0xc24b8b70U, 0xc76c51a3U,
    0xd192e819U, 0xd6990624U, 0xf40e3585U, 0x106aa070U,
    0x19a4c116U, 0x1e376c08U, 0x2748774cU, 0x34b0bcb5U,
    0x391c0cb3U, 0x4ed8aa4aU, 0x5b9cca4fU, 0x682e6ff3U,
    0x748f82eeU, 0x78a5636fU, 0x84c87814U, 0x8cc70208U,
    0x90befffaU, 0xa4506cebU, 0xbef9a3f7U, 0xc67178f2U,
};

static uint32_t load_be32(const uint8_t *data)
{
    return ((uint32_t)data[0] << 24) | ((uint32_t)data[1] << 16) |
           ((uint32_t)data[2] << 8) | data[3];
}

static void store_be32(uint8_t *data, uint32_t value)
{
    data[0] = (uint8_t)(value >> 24);
    data[1] = (uint8_t)(value >> 16);
    data[2] = (uint8_t)(value >> 8);
    data[3] = (uint8_t)value;
}

static uint32_t rotate_right(uint32_t value, unsigned count)
{
    return (value >> count) | (value << (32U - count));
}

static void transform(struct loos_sha256 *context, const uint8_t block[64])
{
    uint32_t words[64];
    for (unsigned i = 0; i < 16U; ++i) {
        words[i] = load_be32(block + i * 4U);
    }
    for (unsigned i = 16U; i < 64U; ++i) {
        const uint32_t s0 = rotate_right(words[i - 15U], 7U) ^
                             rotate_right(words[i - 15U], 18U) ^
                             (words[i - 15U] >> 3U);
        const uint32_t s1 = rotate_right(words[i - 2U], 17U) ^
                             rotate_right(words[i - 2U], 19U) ^
                             (words[i - 2U] >> 10U);
        words[i] = words[i - 16U] + s0 + words[i - 7U] + s1;
    }
    uint32_t a = context->state[0], b = context->state[1];
    uint32_t c = context->state[2], d = context->state[3];
    uint32_t e = context->state[4], f = context->state[5];
    uint32_t g = context->state[6], h = context->state[7];
    for (unsigned i = 0; i < 64U; ++i) {
        const uint32_t s1 = rotate_right(e, 6U) ^ rotate_right(e, 11U) ^
                            rotate_right(e, 25U);
        const uint32_t choose = (e & f) ^ (~e & g);
        const uint32_t temporary1 = h + s1 + choose + round_constants[i] +
                                    words[i];
        const uint32_t s0 = rotate_right(a, 2U) ^ rotate_right(a, 13U) ^
                            rotate_right(a, 22U);
        const uint32_t majority = (a & b) ^ (a & c) ^ (b & c);
        const uint32_t temporary2 = s0 + majority;
        h = g;
        g = f;
        f = e;
        e = d + temporary1;
        d = c;
        c = b;
        b = a;
        a = temporary1 + temporary2;
    }
    context->state[0] += a;
    context->state[1] += b;
    context->state[2] += c;
    context->state[3] += d;
    context->state[4] += e;
    context->state[5] += f;
    context->state[6] += g;
    context->state[7] += h;
}

void loos_sha256_init(struct loos_sha256 *context)
{
    if (context == NULL) {
        return;
    }
    context->state[0] = 0x6a09e667U;
    context->state[1] = 0xbb67ae85U;
    context->state[2] = 0x3c6ef372U;
    context->state[3] = 0xa54ff53aU;
    context->state[4] = 0x510e527fU;
    context->state[5] = 0x9b05688cU;
    context->state[6] = 0x1f83d9abU;
    context->state[7] = 0x5be0cd19U;
    context->bit_count = 0;
    context->block_length = 0;
}

void loos_sha256_update(struct loos_sha256 *context,
                        const void *data,
                        size_t length)
{
    if (context == NULL || (length != 0 && data == NULL)) {
        return;
    }
    const uint8_t *bytes = data;
    context->bit_count += (uint64_t)length * 8U;
    while (length != 0) {
        const size_t available = 64U - context->block_length;
        const size_t copy = length < available ? length : available;
        memcpy(context->block + context->block_length, bytes, copy);
        context->block_length += copy;
        bytes += copy;
        length -= copy;
        if (context->block_length == 64U) {
            transform(context, context->block);
            context->block_length = 0;
        }
    }
}

void loos_sha256_final(struct loos_sha256 *context,
                       uint8_t digest[LOOS_SHA256_DIGEST_SIZE])
{
    if (context == NULL || digest == NULL) {
        return;
    }
    const uint64_t bit_count = context->bit_count;
    context->block[context->block_length++] = 0x80U;
    if (context->block_length > 56U) {
        memset(context->block + context->block_length, 0,
               64U - context->block_length);
        transform(context, context->block);
        context->block_length = 0;
    }
    memset(context->block + context->block_length, 0,
           56U - context->block_length);
    for (unsigned i = 0; i < 8U; ++i) {
        context->block[56U + i] = (uint8_t)(bit_count >> (56U - i * 8U));
    }
    transform(context, context->block);
    for (unsigned i = 0; i < 8U; ++i) {
        store_be32(digest + i * 4U, context->state[i]);
    }
    memset(context, 0, sizeof(*context));
}

void loos_sha256(const void *data,
                 size_t length,
                 uint8_t digest[LOOS_SHA256_DIGEST_SIZE])
{
    struct loos_sha256 context;
    loos_sha256_init(&context);
    loos_sha256_update(&context, data, length);
    loos_sha256_final(&context, digest);
}

bool loos_constant_time_equal(const void *left, const void *right, size_t length)
{
    if ((length != 0 && (left == NULL || right == NULL))) {
        return false;
    }
    const uint8_t *a = left;
    const uint8_t *b = right;
    uint8_t difference = 0;
    for (size_t i = 0; i < length; ++i) {
        difference |= (uint8_t)(a[i] ^ b[i]);
    }
    return difference == 0;
}
