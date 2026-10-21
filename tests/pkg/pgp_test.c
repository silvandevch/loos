#include "../../src/pkg/pgp.h"
#include "pgp_fixture.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
static int fails;
#define CHECK(c, m) do { if (!(c)) { fprintf(stderr, "FAIL: %s\n", m); fails++; } } while (0)
static pgp_result clear(const char *key, const char *msg, time_t now)
{
    pgp_keyring r; pgp_keyring_init(&r); pgp_keyring_add(&r, (const uint8_t *)key, strlen(key));
    char out[8192]; size_t n; pgp_verdict v;
    pgp_result res = pgp_verify_clearsigned(&r, msg, strlen(msg), now, &v, out, sizeof out, &n);
    pgp_keyring_free(&r); return res;
}
int main(void)
{
    time_t now = time(NULL);
    CHECK(clear(KEY_A, INRELEASE_A_SHA256, now) == PGP_OK, "A sha256");
    CHECK(clear(KEY_A, INRELEASE_A_SHA512, now) == PGP_OK, "A sha512");
    CHECK(clear(KEY_B, INRELEASE_B_SHA256, now) == PGP_OK, "B subkey signer");
    CHECK(clear(KEY_B, INRELEASE_A_SHA256, now) != PGP_OK, "wrong key");
    CHECK(clear(KEY_A, INRELEASE_A_SHA1, now) == PGP_WEAK_HASH, "sha1 refused");
    CHECK(clear(KEY_D, INRELEASE_D, now) != PGP_OK, "ed25519 not accepted");
    CHECK(clear(KEY_C, INRELEASE_C_SHA256, now + 3 * 365 * 86400L) == PGP_KEY_EXPIRED, "expired key");
    char *t = malloc(sizeof INRELEASE_A_SHA256); memcpy(t, INRELEASE_A_SHA256, sizeof INRELEASE_A_SHA256); char *p = strstr(t, "Suite"); if (p) p[0] = 'X';
    CHECK(clear(KEY_A, t, now) == PGP_BAD_SIGNATURE, "tampered text"); free(t);
    pgp_keyring r; pgp_keyring_init(&r); pgp_keyring_add(&r, (const uint8_t *)KEY_A, strlen(KEY_A));
    CHECK(pgp_verify_detached(&r, (const uint8_t *)RELEASE_GPG_A_ARMOR, strlen(RELEASE_GPG_A_ARMOR), (const uint8_t *)RELEASE, strlen(RELEASE), now, NULL) == PGP_OK, "detached armor");
    CHECK(pgp_verify_detached(&r, RELEASE_GPG_A_BIN_BYTES, sizeof RELEASE_GPG_A_BIN_BYTES, (const uint8_t *)RELEASE, strlen(RELEASE), now, NULL) == PGP_OK, "detached binary");
    CHECK(pgp_verify_detached(&r, RELEASE_GPG_A_BIN_BYTES, sizeof RELEASE_GPG_A_BIN_BYTES, (const uint8_t *)RELEASE, strlen(RELEASE) - 1, now, NULL) == PGP_BAD_SIGNATURE, "detached tampered");
    char fh[41]; pgp_fingerprint_hex(r.k[0].fpr, fh); CHECK(strlen(fh) == 40 && r.k[0].keyid[7] == r.k[0].fpr[19], "fingerprint");
    pgp_keyring_free(&r);
    if (!fails) puts("pgp_test: all passed");
    return fails != 0;
}
