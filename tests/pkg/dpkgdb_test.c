#include "dpkgdb.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static const char *STATUS =
    "Package: libc6\nStatus: install ok installed\nPriority: optional\nSection: libs\nInstalled-Size: 13000\nMaintainer: Maint <m@x>\nArchitecture: amd64\nMulti-Arch: same\nSource: glibc\nVersion: 2.35-0ubuntu3.4\n"
    "Provides: libc6-amd64 (= 2.35)\nDepends: libgcc-s1\nBreaks: fakeroot (<< 1.25.3-1.1ubuntu2)\nDescription: GNU C Library: Shared libraries\n Contains the standard libraries.\n\n"
    "Package: libgcc-s1\nStatus: install ok installed\nArchitecture: amd64\nVersion: 12.3.0-1ubuntu1~22.04\nDepends: gcc-12-base (= 12.3.0-1ubuntu1~22.04)\nDescription: GCC support library\n\n"
    "Package: gcc-12-base\nStatus: install ok installed\nArchitecture: amd64\nVersion: 12.3.0-1ubuntu1~22.04\nDescription: GCC base\n\n"
    "Package: fakeroot\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1.28-1ubuntu1\nDepends: libc6 (>= 2.34)\nDescription: fake root\n\n"
    "Package: oldconf\nStatus: deinstall ok config-files\nArchitecture: all\nVersion: 1.0\nDescription: removed, configs remain\n\n"
    "Package: half\nStatus: install ok half-configured\nArchitecture: amd64\nVersion: 2.0\nDescription: stuck\n\n";

int main(void)
{
    dpkg_db db; dpkg_db_init(&db, "amd64");
    CHECK(dpkg_db_load_status(&db, STATUS, strlen(STATUS)) == 6 && db.n == 6);
    const dpkg_entry *libc = dpkg_db_find_const(&db, "libc6", NULL);
    CHECK(libc && dpkg_entry_installed(libc) && libc->want == WANT_INSTALL && libc->flag == FLAG_OK && libc->status == ST_INSTALLED && !strcmp(libc->pkg.version, "2.35-0ubuntu3.4"));
    CHECK(libc->pkg.nprovides == 1 && libc->pkg.breaks.n == 1 && libc->pkg.multiarch_same && libc->pkg.installed_size == 13000u * 1024);
    const dpkg_entry *oc = dpkg_db_find_const(&db, "oldconf", NULL);
    CHECK(oc && oc->want == WANT_DEINSTALL && oc->status == ST_CONFIG_FILES && !dpkg_entry_installed(oc));
    CHECK(dpkg_db_find_const(&db, "half", "amd64")->status == ST_HALF_CONFIGURED && !dpkg_db_find_const(&db, "libc6", "arm64") && !dpkg_db_find_const(&db, "nothing", NULL));

    /* the saved text loads back identically, and dpkg's own state words round-trip */
    static char saved[32768]; size_t n = dpkg_db_save_status(&db, saved, sizeof saved);
    CHECK(n > 500 && strstr(saved, "Status: install ok installed") && strstr(saved, "Status: deinstall ok config-files") && strstr(saved, "Provides: libc6-amd64 (= 2.35)"));
    dpkg_db db2; dpkg_db_init(&db2, "amd64");
    CHECK(dpkg_db_load_status(&db2, saved, n) == 6);
    for (int i = 0; i < db.n; i++) {
        const dpkg_entry *a = &db.e[i], *b = dpkg_db_find_const(&db2, a->pkg.name, a->pkg.arch);
        CHECK(b && !strcmp(a->pkg.version, b->pkg.version) && a->status == b->status && a->want == b->want && a->pkg.depends.n == b->pkg.depends.n && a->pkg.nprovides == b->pkg.nprovides);
    }
    static char again[32768]; size_t n2 = dpkg_db_save_status(&db2, again, sizeof again); CHECK(n2 == n && !strcmp(saved, again));      /* stable fixed point */
    dpkg_db_free(&db2);
    dpkg_db_free(&db2); dpkg_db_init(&db2, "amd64");
    CHECK(dpkg_db_load_status(&db2, "Package: x\nVersion: 1\n\n", 22) == -1);                                                       /* no Status line */
    dpkg_db_free(&db2); dpkg_db_init(&db2, "amd64");
    CHECK(dpkg_db_load_status(&db2, "Package: x\nStatus: install ok installed\nVersion: not a version\n\n", 62) == -1);
    dpkg_db_free(&db2);

    /* auto-installed marks */
    const char *ext = "Package: libgcc-s1\nArchitecture: amd64\nAuto-Installed: 1\n\nPackage: gcc-12-base\nArchitecture: amd64\nAuto-Installed: 1\n\nPackage: ghost\nArchitecture: amd64\nAuto-Installed: 1\n\n";
    dpkg_db_load_extended_states(&db, ext, strlen(ext));
    CHECK(dpkg_db_find(&db, "libgcc-s1", NULL)->auto_installed && dpkg_db_find(&db, "gcc-12-base", NULL)->auto_installed && !dpkg_db_find(&db, "libc6", NULL)->auto_installed);
    char es[512]; size_t el = dpkg_db_save_extended_states(&db, es, sizeof es);
    CHECK(el > 40 && strstr(es, "Package: libgcc-s1") && !strstr(es, "libc6") && !strstr(es, "ghost"));

    /* file lists and ownership */
    const char *libc_files[] = { "/lib/x86_64-linux-gnu/libc.so.6", "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2", "/usr/share/doc/libc6/copyright", "/usr/share/doc/shared" };
    CHECK(dpkg_db_set_files(&db, "libc6", "amd64", libc_files, 4) && !dpkg_db_set_files(&db, "ghost", NULL, libc_files, 4));
    CHECK(dpkg_db_owner(&db, "/lib/x86_64-linux-gnu/libc.so.6") == dpkg_db_find(&db, "libc6", NULL) && !dpkg_db_owner(&db, "/etc/passwd"));
    char lst[512]; size_t ll = dpkg_db_save_list(&db, "libc6", "amd64", lst, sizeof lst);
    CHECK(ll > 80 && strstr(lst, "/usr/share/doc/shared\n"));
    CHECK(dpkg_db_load_list(&db, "fakeroot", "amd64", "/usr/bin/fakeroot\n/usr/share/doc/shared\n\nnot-absolute\n/usr/lib/faketime.so\n") == 3);
    CHECK(dpkg_db_owner(&db, "/usr/bin/fakeroot") != NULL && dpkg_db_find(&db, "fakeroot", NULL)->nfiles == 3);

    /* conflicts between packages: replaced files are fine, others are not */
    deb_pkg newp; deb_stanza st = { 0 };
    deb_stanza_set(&st, "Package", "libc6-compat"); deb_stanza_set(&st, "Version", "1.0"); deb_stanza_set(&st, "Architecture", "amd64");
    CHECK(deb_pkg_from_stanza(&st, "amd64", &newp));
    const char *new_files[] = { "/lib/x86_64-linux-gnu/libc.so.6", "/usr/new/file", "/usr/bin/fakeroot" };
    dpkg_file_conflict cf[4];
    int nc = dpkg_db_check_files(&db, &newp, new_files, 3, cf, 4);
    CHECK(nc == 2 && !strcmp(cf[0].owner, "libc6") && !strcmp(cf[1].owner, "fakeroot"));
    deb_pkg_free(&newp); deb_stanza_set(&st, "Replaces", "libc6 (<< 3), fakeroot");
    CHECK(deb_pkg_from_stanza(&st, "amd64", &newp) && dpkg_db_check_files(&db, &newp, new_files, 3, cf, 4) == 0);                  /* Replaces covers both */
    deb_pkg_free(&newp); deb_stanza_set(&st, "Replaces", "libc6 (<< 2.0), fakeroot");
    CHECK(deb_pkg_from_stanza(&st, "amd64", &newp) && dpkg_db_check_files(&db, &newp, new_files, 3, cf, 4) == 1 && !strcmp(cf[0].owner, "libc6"));   /* version-bounded Replaces */
    deb_pkg_free(&newp); deb_stanza_free(&st);
    deb_stanza_set(&st, "Package", "libc6"); deb_stanza_set(&st, "Version", "2.36"); deb_stanza_set(&st, "Architecture", "amd64");
    CHECK(deb_pkg_from_stanza(&st, "amd64", &newp) && dpkg_db_check_files(&db, &newp, libc_files, 4, cf, 4) == 0);                  /* a package may overwrite its own files on upgrade */
    deb_pkg_free(&newp); deb_stanza_free(&st);
    deb_stanza_set(&st, "Package", "usurper"); deb_stanza_set(&st, "Version", "1"); deb_stanza_set(&st, "Architecture", "amd64");
    CHECK(deb_pkg_from_stanza(&st, "amd64", &newp));
    const char *shared[] = { "/usr/share/doc/shared" };
    CHECK(dpkg_db_check_files(&db, &newp, shared, 1, cf, 4) == 1);
    dpkg_db_add_diversion(&db, "/usr/share/doc/shared", "/usr/share/doc/shared.distrib", "usurper");
    CHECK(dpkg_db_check_files(&db, &newp, shared, 1, cf, 4) == 0);                                                                      /* a diversion makes room */
    deb_pkg_free(&newp); deb_stanza_free(&st);

    /* audit: a healthy db is clean; breaking things shows up */
    char issues[8][160];
    int base = dpkg_db_audit(&db, issues, 8);
    CHECK(base == 0);
    dpkg_db_remove(&db, "gcc-12-base", NULL, true);
    int a = dpkg_db_audit(&db, issues, 8);
    CHECK(a == 1 && strstr(issues[0], "libgcc-s1: unmet dependency gcc-12-base"));
    dpkg_entry *fr = dpkg_db_find(&db, "fakeroot", NULL);
    deb_pkg old; deb_stanza os = { 0 }; deb_stanza_set(&os, "Package", "fakeroot"); deb_stanza_set(&os, "Version", "1.20"); deb_stanza_set(&os, "Architecture", "amd64"); deb_pkg_from_stanza(&os, "amd64", &old);
    CHECK(fr && dpkg_db_put(&db, &old, ST_INSTALLED) == fr);                                                                            /* replace in place */
    CHECK(strcmp(fr->pkg.version, "1.20") == 0 && fr->nfiles == 3);
    a = dpkg_db_audit(&db, issues, 8);
    bool breaks_seen = false; for (int i = 0; i < a; i++) if (strstr(issues[i], "libc6: breaks fakeroot")) breaks_seen = true;
    CHECK(breaks_seen);
    deb_pkg_free(&old); deb_stanza_free(&os);
    dpkg_entry *hf = dpkg_db_find(&db, "half", NULL); hf->flag = FLAG_REINSTREQ; hf->status = ST_UNPACKED;
    a = dpkg_db_audit(&db, issues, 8); bool reinst = false; for (int i = 0; i < a; i++) if (strstr(issues[i], "half: reinstallation required")) reinst = true; CHECK(reinst);

    /* remove vs purge */
    CHECK(dpkg_db_remove(&db, "fakeroot", NULL, false) && dpkg_db_find(&db, "fakeroot", NULL)->status == ST_CONFIG_FILES && dpkg_db_find(&db, "fakeroot", NULL)->nfiles == 0 && !dpkg_db_owner(&db, "/usr/bin/fakeroot"));
    CHECK(dpkg_db_remove(&db, "fakeroot", NULL, true) && !dpkg_db_find(&db, "fakeroot", NULL) && !dpkg_db_remove(&db, "fakeroot", NULL, true));
    int before = db.n;
    for (int i = 0; i < 300; i++) { char nm[16]; snprintf(nm, sizeof nm, "bulk%d", i); deb_pkg bp; deb_stanza bs = { 0 }; deb_stanza_set(&bs, "Package", nm); deb_stanza_set(&bs, "Version", "1.0"); deb_stanza_set(&bs, "Architecture", "all"); deb_pkg_from_stanza(&bs, "amd64", &bp); dpkg_db_put(&db, &bp, ST_INSTALLED); deb_pkg_free(&bp); deb_stanza_free(&bs); }
    CHECK(db.n == before + 300 && dpkg_db_find(&db, "bulk299", NULL));
    dpkg_db_free(&db);
    DONE("dpkgdb_test");
}
