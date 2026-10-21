#include "deb.h"
#include "test.h"
#include "debver_cases.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

int main(void)
{
    /* every ordered pair of 47 tricky versions agrees with the reference dpkg */
    int mism = 0;
    for (unsigned i = 0; i < sizeof debver_cases / sizeof *debver_cases; i++) {
        int c = deb_version_compare(debver_cases[i].a, debver_cases[i].b);
        c = c < 0 ? -1 : c > 0;
        if (c != debver_cases[i].cmp) { if (mism++ < 5) printf("version mismatch: %s vs %s: got %d want %d\n", debver_cases[i].a, debver_cases[i].b, c, debver_cases[i].cmp); }
    }
    CHECK(mism == 0);
    CHECK(sizeof debver_cases / sizeof *debver_cases > 2000);
    CHECK(deb_version_valid("1.0-1") && deb_version_valid("1:2.3~rc1+dfsg-4ubuntu1") && !deb_version_valid("") && !deb_version_valid("abc") && !deb_version_valid(":1.0") && !deb_version_valid("a:1.0") && !deb_version_valid("1.0 1") && !deb_version_valid("1.0_1"));
    CHECK(deb_rel_satisfied(DREL_GE, "1.0", "1.0") && !deb_rel_satisfied(DREL_GT, "1.0", "1.0") && deb_rel_satisfied(DREL_LT, "2", "1.9") && deb_rel_satisfied(DREL_NONE, "x", "y") && deb_rel_satisfied(DREL_EQ, "1.0", "1.0") && !deb_rel_satisfied(DREL_EQ, "1.0", "1.0-1"));

    /* stanzas */
    const char *text =
        "# comment\n"
        "Package: libfoo1\nVersion: 1.2-3\nArchitecture: amd64\nDepends: libc6 (>= 2.30), libbar | libbaz\nDescription: foo library\n"
        " The long description\n . \n continues here\n\n\n"
        "Package: foo\nVersion: 1:1.2-3\nArchitecture: all\nEssential: yes\n";
    size_t pos = 0, len = strlen(text); deb_stanza s;
    CHECK(deb_stanza_next(text, len, &pos, &s) == 1 && !strcmp(deb_stanza_get(&s, "package"), "libfoo1") && !strcmp(deb_stanza_get(&s, "VERSION"), "1.2-3"));
    const char *desc = deb_stanza_get(&s, "Description");
    CHECK(desc && !strcmp(desc, "foo library\nThe long description\n.\ncontinues here") && !deb_stanza_get(&s, "Nope"));
    char fmt[512]; deb_stanza_format(&s, fmt, sizeof fmt);
    deb_stanza s2; size_t p2 = 0; CHECK(deb_stanza_next(fmt, strlen(fmt), &p2, &s2) == 1 && s2.n == s.n && !strcmp(deb_stanza_get(&s2, "Description"), desc));
    deb_stanza_free(&s2);
    deb_stanza_free(&s);
    CHECK(deb_stanza_next(text, len, &pos, &s) == 1 && !strcmp(deb_stanza_get(&s, "Package"), "foo") && !strcmp(deb_stanza_get(&s, "Essential"), "yes"));
    deb_stanza_free(&s);
    CHECK(deb_stanza_next(text, len, &pos, &s) == 0);
    deb_stanza bad; p2 = 0;
    CHECK(deb_stanza_next("no colon here\n", 14, &p2, &bad) == -1); p2 = 0;
    CHECK(deb_stanza_next(" starts with continuation\n", 26, &p2, &bad) == -1); p2 = 0;
    CHECK(deb_stanza_next("Bad Key: x\n", 11, &p2, &bad) == -1);
    p2 = 0; CHECK(deb_stanza_next("Key: v\r\nOther: w\r\n\r\n", 20, &p2, &s) == 1 && !strcmp(deb_stanza_get(&s, "Other"), "w") && !strcmp(deb_stanza_get(&s, "Key"), "v"));       /* CRLF tolerated */
    deb_stanza_set(&s, "key", "new"); CHECK(!strcmp(deb_stanza_get(&s, "Key"), "new") && s.n == 2);
    deb_stanza_free(&s);
    CHECK(deb_stanza_next("", 0, &(size_t){ 0 }, &s) == 0);

    /* dependencies */
    deb_deps d; char out[512];
    CHECK(deb_deps_parse("libc6 (>= 2.34), libssl3 (>> 3.0) | libssl1.1, python3:any, debconf (= 1.5) | debconf-2.0", "amd64", &d) && d.n == 4);
    CHECK(d.grp[0].nalt == 1 && !strcmp(d.grp[0].alt[0].name, "libc6") && d.grp[0].alt[0].rel == DREL_GE && !strcmp(d.grp[0].alt[0].version, "2.34"));
    CHECK(d.grp[1].nalt == 2 && d.grp[1].alt[0].rel == DREL_GT && d.grp[1].alt[1].rel == DREL_NONE && !strcmp(d.grp[2].alt[0].arch, "any") && !strcmp(d.grp[3].alt[0].arch, ""));
    deb_deps_format(&d, out, sizeof out);
    CHECK(!strcmp(out, "libc6 (>= 2.34), libssl3 (>> 3.0) | libssl1.1, python3:any, debconf (= 1.5) | debconf-2.0"));
    deb_deps_free(&d);
    CHECK(deb_deps_parse("a (<< 2), b (<= 1) , c(>=1.0)", "amd64", &d) && d.n == 3 && d.grp[0].alt[0].rel == DREL_LT && d.grp[1].alt[0].rel == DREL_LE && d.grp[2].alt[0].rel == DREL_GE); deb_deps_free(&d);
    CHECK(deb_deps_parse("a (< 2), b (> 1)", "amd64", &d) && d.grp[0].alt[0].rel == DREL_LE && d.grp[1].alt[0].rel == DREL_GE); deb_deps_free(&d);        /* deprecated < and > mean <= and >= */
    CHECK(deb_deps_parse("only-amd64 [amd64], not-arm [!arm64], armonly [arm64 armhf], x86 [i386 amd64], neg [!amd64 !i386]", "amd64", &d) && d.n == 3);
    CHECK(!strcmp(d.grp[0].alt[0].name, "only-amd64") && !strcmp(d.grp[1].alt[0].name, "not-arm") && !strcmp(d.grp[2].alt[0].name, "x86")); deb_deps_free(&d);
    CHECK(deb_deps_parse("alt1 [arm64] | alt2, solo [arm64]", "amd64", &d) && d.n == 1 && d.grp[0].nalt == 1 && !strcmp(d.grp[0].alt[0].name, "alt2")); deb_deps_free(&d);   /* excluded atoms vanish, empty groups too */
    CHECK(deb_deps_parse("dh-foo <!nocheck> <cross>, bar", "amd64", &d) && d.n == 2); deb_deps_free(&d);                                          /* profiles ignored */
    CHECK(deb_deps_parse("", "amd64", &d) && d.n == 0 && deb_deps_parse("a,\n b,\n c", "amd64", &d) && d.n == 3); deb_deps_free(&d);
    CHECK(deb_deps_parse("a, ", "amd64", &d) && d.n == 1); deb_deps_free(&d);
    const char *bad_deps[] = { "a (>= )", "a (>= 1", "a (~ 1)", "a [amd64", "| a", "a |", ", a", "a b", "a (>= 1) (<< 2)", "(>= 1)" };
    for (unsigned i = 0; i < sizeof bad_deps / sizeof *bad_deps; i++) CHECK(!deb_deps_parse(bad_deps[i], "amd64", &d));
    for (int t = 0; t < 3000; t++) {                                                                                                              /* mutation fuzz */
        char f[96]; strcpy(f, "libc6 (>= 2.34), libssl3 (>> 3.0) | libssl1.1 [amd64], py:any <!nocheck>");
        unsigned x = (unsigned)t * 2654435761u; for (int k = 0; k < 3; k++) { x = x * 1664525u + 1013904223u; f[(x >> 8) % (unsigned)strlen(f)] = (char)(32 + (x >> 24) % 90); }
        if (deb_deps_parse(f, "amd64", &d)) deb_deps_free(&d);
    }

    /* package records */
    const char *rec =
        "Package: libssl3\nSource: openssl (3.0.2-0ubuntu1)\nVersion: 3.0.2-0ubuntu1.15\nArchitecture: amd64\nInstalled-Size: 5949\nMaintainer: Ubuntu Developers <x@y>\nDepends: libc6 (>= 2.34), debconf\nPre-Depends: dpkg (>= 1.0)\nBreaks: openssl (<< 3.0.2-0ubuntu1.15)\nConflicts: libssl1.0\nReplaces: libssl1.0\nProvides: libssl-abi (= 3), virtual-ssl\nMulti-Arch: same\nSection: libs\nPriority: optional\nFilename: pool/main/o/openssl/libssl3_3.0.2-0ubuntu1.15_amd64.deb\nSize: 1700000\nSHA256: abcdef\nDescription: Secure Sockets Layer toolkit - shared libraries\n It is part of the OpenSSL project.\nHomepage: https://www.openssl.org/\nEssential: no\n";
    size_t rp = 0; deb_stanza rs; deb_pkg pk;
    CHECK(deb_stanza_next(rec, strlen(rec), &rp, &rs) == 1 && deb_pkg_from_stanza(&rs, "amd64", &pk));
    CHECK(!strcmp(pk.name, "libssl3") && !strcmp(pk.version, "3.0.2-0ubuntu1.15") && !strcmp(pk.arch, "amd64") && pk.installed_size == 5949u * 1024 && pk.size == 1700000 && !pk.essential && pk.multiarch_same);
    CHECK(!strcmp(pk.description, "Secure Sockets Layer toolkit - shared libraries") && pk.depends.n == 2 && pk.pre_depends.n == 1 && pk.breaks.n == 1 && pk.breaks.grp[0].alt[0].rel == DREL_LT && pk.conflicts.n == 1 && pk.replaces.n == 1);
    CHECK(pk.nprovides == 2 && !strcmp(pk.provides[0].name, "libssl-abi") && !strcmp(pk.provides[0].version, "3") && !strcmp(pk.provides[1].name, "virtual-ssl") && !pk.provides[1].version);
    CHECK(!strcmp(pk.filename, "pool/main/o/openssl/libssl3_3.0.2-0ubuntu1.15_amd64.deb") && !strcmp(pk.sha256, "abcdef") && !strcmp(pk.section, "libs"));
    deb_pkg_free(&pk); deb_stanza_free(&rs);
    deb_stanza e1 = { 0 }; deb_stanza_set(&e1, "Package", "x"); CHECK(!deb_pkg_from_stanza(&e1, "amd64", &pk)); deb_stanza_free(&e1);                       /* version is mandatory */
    deb_stanza e2 = { 0 }; deb_stanza_set(&e2, "Package", "x"); deb_stanza_set(&e2, "Version", "bad version"); CHECK(!deb_pkg_from_stanza(&e2, "amd64", &pk));
    deb_stanza_set(&e2, "Version", "1.0"); deb_stanza_set(&e2, "Depends", "a (>="); CHECK(!deb_pkg_from_stanza(&e2, "amd64", &pk));
    deb_stanza_set(&e2, "Depends", "a"); CHECK(deb_pkg_from_stanza(&e2, "amd64", &pk) && !strcmp(pk.arch, "all") && deb_arch_ok(pk.arch, "amd64")); deb_pkg_free(&pk); deb_stanza_free(&e2);
    CHECK(deb_arch_ok("amd64", "amd64") && deb_arch_ok("all", "amd64") && !deb_arch_ok("arm64", "amd64"));
    DONE("deb_test");
}
