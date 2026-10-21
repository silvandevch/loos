#include "../../src/security/kdf.h"
#include <stdio.h>
#include <string.h>

static int failures;
static void hex(const uint8_t *b, size_t n, char *out) { for (size_t i = 0; i < n; i++) sprintf(out + 2 * i, "%02x", b[i]); }
#define EXPECT_HEX(got, n, want, what) do { char h_[200]; hex(got, n, h_); if (strcmp(h_, want)) { failures++; printf("FAIL %s\n  got  %s\n  want %s\n", what, h_, want); } } while (0)

int main(void)
{
    uint8_t mac[32], out[64];
    /* RFC 4231 test cases 1, 2 and 6 (key longer than the block size) */
    uint8_t k1[20]; memset(k1, 0x0b, sizeof k1);
    loos_hmac_sha256(k1, sizeof k1, "Hi There", 8, mac);
    EXPECT_HEX(mac, 32, "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7", "hmac rfc4231 #1");
    loos_hmac_sha256("Jefe", 4, "what do ya want for nothing?", 28, mac);
    EXPECT_HEX(mac, 32, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843", "hmac rfc4231 #2");
    uint8_t k6[131]; memset(k6, 0xaa, sizeof k6);
    loos_hmac_sha256(k6, sizeof k6, "Test Using Larger Than Block-Size Key - Hash Key First", 54, mac);
    EXPECT_HEX(mac, 32, "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54", "hmac rfc4231 #6");
    /* streaming equals one-shot */
    struct loos_hmac_sha256 c; loos_hmac_sha256_init(&c, "Jefe", 4);
    loos_hmac_sha256_update(&c, "what do ya ", 11); loos_hmac_sha256_update(&c, "want for nothing?", 17); loos_hmac_sha256_final(&c, mac);
    EXPECT_HEX(mac, 32, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843", "hmac streaming");
    /* RFC 7914 section 11 PBKDF2-HMAC-SHA-256 vectors */
    loos_pbkdf2_sha256("passwd", 6, "salt", 4, 1, out, 64);
    EXPECT_HEX(out, 64, "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783", "pbkdf2 c=1");
    loos_pbkdf2_sha256("Password", 8, "NaCl", 4, 80000, out, 64);
    EXPECT_HEX(out, 64, "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d", "pbkdf2 c=80000");
    /* other lengths are prefixes of the same stream; zero-length inputs are legal */
    uint8_t part[20]; loos_pbkdf2_sha256("passwd", 6, "salt", 4, 1, part, 20);
    if (memcmp(part, out, 0) != 0) failures++;
    uint8_t full[64]; loos_pbkdf2_sha256("passwd", 6, "salt", 4, 1, full, 64);
    if (memcmp(part, full, 20)) { failures++; puts("FAIL pbkdf2 prefix"); }
    if (!loos_pbkdf2_sha256("", 0, "", 0, 1, out, 32)) { failures++; puts("FAIL empty inputs"); }
    if (loos_pbkdf2_sha256("p", 1, "s", 1, 0, out, 32)) { failures++; puts("FAIL zero iterations accepted"); }
    printf("kdf_test: %d failed\n", failures);
    return failures != 0;
}
