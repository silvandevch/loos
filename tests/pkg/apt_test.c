#include "apt.h"
#include "../../src/security/crypto.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static void hexs(const uint8_t *b, size_t n, char *o) { for (size_t i = 0; i < n; i++) sprintf(o + 2 * i, "%02x", b[i]); }

int main(void)
{
    /* ---- sources.list ---- */
    const char *list =
        "# main archive\n"
        "deb https://archive.example/ubuntu jammy main restricted universe # trailing comment\n"
        "deb-src https://archive.example/ubuntu jammy main\n"
        "deb [arch=amd64,i386 signed-by=/usr/share/keyrings/k.gpg] https://ppa.example/foo/ubuntu jammy main\n"
        "deb [trusted=yes] http://sketchy.example/repo ./\n"
        "deb http://192.168.1.5/mirror stable main\n"
        "deb [check-date=no by-hash=yes] https://old.example/debian bullseye main contrib non-free\n"
        "\n"
        "deb https://flat.example/repo/ ./\n"
        "garbage line here\n"
        "deb onlytwo https://x\n";
    apt_source src[16];
    int n = apt_parse_sources_list(list, src, 16);
    CHECK(n == 7);
    CHECK(!src[0].is_src && !strcmp(src[0].uri, "https://archive.example/ubuntu") && !strcmp(src[0].suite, "jammy") && src[0].ncomp == 3 && !strcmp(src[0].comp[2], "universe") && src[0].check_date);
    CHECK(src[1].is_src && src[1].ncomp == 1);
    CHECK(src[2].narch == 2 && !strcmp(src[2].arch[1], "i386") && !strcmp(src[2].signed_by, "/usr/share/keyrings/k.gpg") && !strcmp(src[2].uri, "https://ppa.example/foo/ubuntu"));
    CHECK(src[3].trusted_override && !strcmp(src[3].suite, "./") && src[3].ncomp == 0);
    CHECK(!src[5].check_date && src[5].by_hash && src[5].ncomp == 3 && !strcmp(src[6].uri, "https://flat.example/repo"));
    /* policy: https only, loopback/LAN http only when allowed, [trusted=yes] never */
    char why[128]; apt_policy pol = { false, false, false };
    CHECK(apt_source_acceptable(&src[0], &pol, why, sizeof why) && apt_source_acceptable(&src[2], &pol, why, sizeof why));
    CHECK(!apt_source_acceptable(&src[3], &pol, why, sizeof why) && strstr(why, "trusted=yes"));
    CHECK(!apt_source_acceptable(&src[4], &pol, why, sizeof why) && strstr(why, "plain http"));
    pol.allow_http_local = true; CHECK(apt_source_acceptable(&src[4], &pol, why, sizeof why) && !apt_source_acceptable(&src[3], &pol, why, sizeof why));      /* sketchy.example is not private */
    apt_source ext = src[4]; strcpy(ext.uri, "http://mirror.example.org/x"); CHECK(!apt_source_acceptable(&ext, &pol, why, sizeof why));
    strcpy(ext.uri, "http://172.20.0.4/x"); CHECK(apt_source_acceptable(&ext, &pol, why, sizeof why)); strcpy(ext.uri, "http://172.40.0.4/x"); CHECK(!apt_source_acceptable(&ext, &pol, why, sizeof why));
    strcpy(ext.uri, "http://nas.local/deb"); CHECK(apt_source_acceptable(&ext, &pol, why, sizeof why)); strcpy(ext.uri, "ftp://x/y"); CHECK(!apt_source_acceptable(&ext, &pol, why, sizeof why));
    pol.allow_unsigned = true; CHECK(apt_source_acceptable(&src[3], &(apt_policy){ true, true, true }, why, sizeof why));
    src[0].enabled = false; CHECK(!apt_source_acceptable(&src[0], &pol, why, sizeof why));

    /* ---- deb822 ---- */
    const char *d822 =
        "Types: deb deb-src\nURIs: https://deb.example/debian https://mirror.example/debian/\nSuites: bookworm bookworm-updates\nComponents: main contrib\nArchitectures: amd64 arm64\nSigned-By: /usr/share/keyrings/debian.gpg\n\n"
        "Types: deb\nURIs: https://off.example/x\nSuites: stable\nComponents: main\nEnabled: no\n\n"
        "Types: deb\nURIs: https://other.example/x\n\n";
    apt_source s2[32]; n = apt_parse_deb822(d822, s2, 32);
    CHECK(n == 2 * 2 * 2 + 1);                                                                                              /* types x uris x suites, the incomplete stanza adds nothing */
    CHECK(!s2[0].is_src && s2[0].ncomp == 2 && s2[0].narch == 2 && !strcmp(s2[0].signed_by, "/usr/share/keyrings/debian.gpg") && s2[0].enabled);
    bool saw_src = false, saw_mirror = false, saw_updates = false; for (int i = 0; i < 8; i++) { saw_src |= s2[i].is_src; saw_mirror |= !strcmp(s2[i].uri, "https://mirror.example/debian"); saw_updates |= !strcmp(s2[i].suite, "bookworm-updates"); }
    CHECK(saw_src && saw_mirror && saw_updates && !s2[8].enabled);

    /* ---- URLs ---- */
    char url[300];
    apt_release_url(&(apt_source){ .uri = "https://h/ubuntu", .suite = "jammy" }, "InRelease", url, sizeof url); CHECK(!strcmp(url, "https://h/ubuntu/dists/jammy/InRelease"));
    apt_index_url(&(apt_source){ .uri = "https://h/ubuntu", .suite = "jammy" }, "main", "amd64", "Packages.xz", url, sizeof url); CHECK(!strcmp(url, "https://h/ubuntu/dists/jammy/main/binary-amd64/Packages.xz"));
    apt_index_url(&(apt_source){ .uri = "https://h/repo", .suite = "./" }, "", "", "Packages.gz", url, sizeof url); CHECK(!strcmp(url, "https://h/repo/./Packages.gz"));
    CHECK(apt_pool_url(&src[0], "pool/main/b/bash/bash_5.1_amd64.deb", url, sizeof url) > 0 && !strcmp(url, "https://archive.example/ubuntu/pool/main/b/bash/bash_5.1_amd64.deb"));
    CHECK(apt_pool_url(&src[0], "../../etc/passwd", url, sizeof url) == 0 && apt_pool_url(&src[0], "/abs/path.deb", url, sizeof url) == 0);

    /* ---- dates ---- */
    CHECK(apt_parse_date("Thu, 01 Jan 1970 00:00:00 UTC") == 0 && apt_parse_date("Tue, 14 Nov 2023 22:13:20 UTC") == 1700000000 && apt_parse_date("Tue, 14 Nov 2023 23:13:20 +0100") == 1700000000);
    CHECK(apt_parse_date("29 Feb 2024 12:00:00 UTC") == 1709208000 && apt_parse_date("garbage") == (time_t)-1 && apt_parse_date("01 Foo 2024 00:00:00 UTC") == (time_t)-1 && apt_parse_date("31 Dec 2023 24:00:00 UTC") == (time_t)-1);

    /* ---- Release with real hashes ---- */
    static const char pk_plain[] = "Package: demo\nVersion: 1\nArchitecture: amd64\n\n";
    uint8_t dg[32]; char hx[65];
    loos_sha256(pk_plain, strlen(pk_plain), dg); hexs(dg, 32, hx);
    char rel[4096];
    snprintf(rel, sizeof rel,
             "Origin: Ubuntu\nLabel: Ubuntu\nSuite: jammy\nVersion: 22.04\nCodename: jammy\nDate: Thu, 21 Apr 2022 17:16:08 UTC\nValid-Until: Thu, 28 Apr 2122 17:16:08 UTC\nArchitectures: amd64 arm64 i386\nComponents: main restricted universe\nAcquire-By-Hash: yes\n"
             "MD5Sum:\n d41d8cd98f00b204e9800998ecf8427e 0 main/binary-amd64/Packages\nSHA256:\n %s %zu main/binary-amd64/Packages\n %s 99 main/binary-amd64/Packages.gz\n %s 100 main/binary-amd64/Packages.xz\n %s 5 universe/binary-amd64/Packages\n",
             hx, strlen(pk_plain), hx, hx, hx);
    apt_release r;
    CHECK(apt_release_parse(rel, strlen(rel), &r) && !strcmp(r.suite, "jammy") && !strcmp(r.codename, "jammy") && !strcmp(r.origin, "Ubuntu") && r.narch == 3 && r.ncomp == 3 && r.by_hash && r.nf == 4);
    CHECK(r.date_t == apt_parse_date("Thu, 21 Apr 2022 17:16:08 UTC") && r.valid_until_t > r.date_t);
    const apt_release_file *f = apt_release_find(&r, "main/binary-amd64/Packages");
    CHECK(f && f->size == strlen(pk_plain) && !apt_release_find(&r, "nothing") && apt_verify_file(f, pk_plain, strlen(pk_plain)));
    CHECK(!apt_verify_file(f, pk_plain, strlen(pk_plain) - 1));                                                              /* wrong size */
    char tampered[64]; strcpy(tampered, pk_plain); tampered[10] ^= 1; CHECK(!apt_verify_file(f, tampered, strlen(tampered)));    /* one flipped bit */
    char upper[65]; apt_release_file e2 = *f; for (int i = 0; i < 64; i++) e2.sha256[i] = (char)(e2.sha256[i] >= 'a' && e2.sha256[i] <= 'f' ? e2.sha256[i] - 32 : e2.sha256[i]); (void)upper; CHECK(apt_verify_file(&e2, pk_plain, strlen(pk_plain)));   /* hex case does not matter */
    char nm[32], path[192];
    CHECK(apt_best_index(&r, "main", "amd64", nm, sizeof nm, path, sizeof path) && !strcmp(nm, "Packages.xz") && !strcmp(path, "main/binary-amd64/Packages.xz"));
    CHECK(apt_best_index(&r, "universe", "amd64", nm, sizeof nm, path, sizeof path) && !strcmp(nm, "Packages") && !apt_best_index(&r, "main", "riscv64", nm, sizeof nm, path, sizeof path));
    /* validity */
    apt_source cs = { .suite = "jammy", .check_date = true }; time_t now = apt_parse_date("Mon, 01 Jan 2024 00:00:00 UTC");
    CHECK(apt_release_check(&r, &cs, now, 600) == APT_REL_OK);
    CHECK(apt_release_check(&r, &cs, r.date_t - 7200, 600) == APT_REL_FROM_FUTURE && apt_release_check(&r, &cs, r.date_t - 300, 600) == APT_REL_OK);
    CHECK(apt_release_check(&r, &cs, r.valid_until_t + 1, 600) == APT_REL_EXPIRED);
    cs.check_date = false; CHECK(apt_release_check(&r, &cs, r.valid_until_t + 100000, 600) == APT_REL_OK);               /* check-date=no */
    strcpy(cs.suite, "focal"); CHECK(apt_release_check(&r, &cs, now, 600) == APT_REL_WRONG_SUITE);                         /* a valid release for the wrong suite is a downgrade attack */
    strcpy(cs.suite, "jammy"); apt_release e0 = r; e0.nf = 0; CHECK(apt_release_check(&e0, &cs, now, 600) == APT_REL_NO_HASHES);
    apt_release_free(&r);
    CHECK(apt_release_parse("", 0, &r) == false);
    CHECK(apt_release_parse("Suite: x\nSHA256:\n short 12 path\n bad\n", 36, &r) && r.nf == 0); apt_release_free(&r);       /* malformed hash lines are skipped, not trusted */

    /* ---- pool file checks against Packages ---- */
    deb_pkg pk; deb_stanza ps = { 0 };
    static const char deb_bytes[] = "!<arch>\nfake deb payload";
    loos_sha256(deb_bytes, sizeof deb_bytes - 1, dg); hexs(dg, 32, hx);
    char sz[16]; snprintf(sz, sizeof sz, "%zu", sizeof deb_bytes - 1);
    deb_stanza_set(&ps, "Package", "demo"); deb_stanza_set(&ps, "Version", "1.0"); deb_stanza_set(&ps, "SHA256", hx); deb_stanza_set(&ps, "Size", sz);
    CHECK(deb_pkg_from_stanza(&ps, "amd64", &pk) && apt_verify_deb_hash(&pk, deb_bytes, sizeof deb_bytes - 1) && !apt_verify_deb_hash(&pk, deb_bytes, sizeof deb_bytes - 2));
    deb_pkg_free(&pk); deb_stanza_set(&ps, "SHA256", "0000"); CHECK(deb_pkg_from_stanza(&ps, "amd64", &pk) && !apt_verify_deb_hash(&pk, deb_bytes, sizeof deb_bytes - 1)); deb_pkg_free(&pk);
    deb_stanza_free(&ps); deb_stanza ns = { 0 }; deb_stanza_set(&ns, "Package", "nohash"); deb_stanza_set(&ns, "Version", "1.0");
    CHECK(deb_pkg_from_stanza(&ns, "amd64", &pk) && !apt_verify_deb_hash(&pk, deb_bytes, sizeof deb_bytes - 1)); deb_pkg_free(&pk); deb_stanza_free(&ns);        /* no recorded hash: refuse */

    /* ---- preferences ---- */
    const char *prefs =
        "Package: *\nPin: release a=jammy-backports\nPin-Priority: 100\n\n"
        "Package: firefox*\nPin: release o=Mozilla, l=Mozilla\nPin-Priority: 1001\n\n"
        "Package: libssl*\nPin: version 3.0.*\nPin-Priority: 900\n\n"
        "Package: broken\nPin: nonsense\nPin-Priority: 5\n";
    apt_prefs ap; CHECK(apt_prefs_parse(prefs, strlen(prefs), &ap) && ap.n == 3);
    apt_release moz; memset(&moz, 0, sizeof moz); strcpy(moz.origin, "Mozilla"); strcpy(moz.label, "Mozilla"); strcpy(moz.suite, "mozilla");
    apt_release bp; memset(&bp, 0, sizeof bp); strcpy(bp.suite, "jammy-backports");
    apt_release plain; memset(&plain, 0, sizeof plain); strcpy(plain.suite, "jammy");
    CHECK(apt_prefs_priority(&ap, "firefox", "120", &moz, false) == 1001 && apt_prefs_priority(&ap, "firefox-esr", "120", &moz, false) == 1001 && apt_prefs_priority(&ap, "firefox", "120", &plain, false) == 500);
    CHECK(apt_prefs_priority(&ap, "anything", "1", &bp, false) == 100 && apt_prefs_priority(&ap, "anything", "1", &plain, false) == 500 && apt_prefs_priority(&ap, "anything", "1", &plain, true) == 100);
    CHECK(apt_prefs_priority(&ap, "libssl3", "3.0.2-1", &plain, false) == 900 && apt_prefs_priority(&ap, "libssl3", "3.1.0", &plain, false) == 500);
    CHECK(apt_prefs_priority(&ap, "x", "1", NULL, false) == 500);
    DONE("apt_test");
}
