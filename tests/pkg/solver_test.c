#include "solver.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>

static deb_repo repo; static dpkg_db db; static solver_plan plan; static solver_request rq;

static void reset(void) { repo_free(&repo); dpkg_db_free(&db); solver_plan_free(&plan); repo_init(&repo, "amd64"); dpkg_db_init(&db, "amd64"); memset(&rq, 0, sizeof rq); }
static void avail(const char *text, int prio) { CHECK(repo_add_packages(&repo, text, strlen(text), 0, prio) >= 0); }
static void installed(const char *name, const char *ver, const char *extra, bool auto_)
{
    char buf[1024]; snprintf(buf, sizeof buf, "Package: %s\nStatus: install ok installed\nVersion: %s\nArchitecture: amd64\n%s\n\n", name, ver, extra ? extra : "");
    CHECK(dpkg_db_load_status(&db, buf, strlen(buf)) == 1);
    dpkg_db_find(&db, name, NULL)->auto_installed = auto_;
}
static void want(const char *spec) { static char names[32][64]; solver_target t; CHECK(solver_parse_target(spec, &t, names[rq.ninstall], 64)); t.name = names[rq.ninstall]; static char vers[32][128]; if (t.version) { strcpy(vers[rq.ninstall], t.version); t.version = vers[rq.ninstall]; } rq.install[rq.ninstall++] = t; }
static int idx_of(const char *name) { for (int i = 0; i < plan.n; i++) if (!strcmp(plan.act[i].pkg->name, name)) return i; return -1; }
static bool plan_has(const char *name, solver_act a) { int i = idx_of(name); return i >= 0 && plan.act[i].act == a; }
static bool solve(void) { solver_plan_free(&plan); return solver_resolve(&repo, &db, &rq, &plan); }

int main(void)
{
    repo_init(&repo, "amd64"); dpkg_db_init(&db, "amd64");

    /* parse targets */
    solver_target t; char nb[64];
    CHECK(solver_parse_target("foo", &t, nb, sizeof nb) && !strcmp(t.name, "foo") && t.rel == DREL_NONE);
    CHECK(solver_parse_target("foo=1.2-3", &t, nb, sizeof nb) && t.rel == DREL_EQ && !strcmp(t.version, "1.2-3"));
    CHECK(solver_parse_target("foo (>= 2:1.0)", &t, nb, sizeof nb) && t.rel == DREL_GE && !strcmp(t.version, "2:1.0") && !solver_parse_target("foo (~ 1)", &t, nb, sizeof nb) && !solver_parse_target("foo=bad version!", &t, nb, sizeof nb) && !solver_parse_target("", &t, nb, sizeof nb));

    /* 1. dependency chain, installed in dependency order, dependencies marked automatic */
    reset();
    avail("Package: app\nVersion: 1.0\nArchitecture: amd64\nDepends: libb (>= 1)\nSize: 100\nInstalled-Size: 10\n\nPackage: libb\nVersion: 1.5\nArchitecture: amd64\nDepends: libc\nSize: 50\n\nPackage: libc\nVersion: 2.0\nArchitecture: amd64\nSize: 25\n", 500);
    want("app");
    CHECK(solve() && plan.n == 3 && idx_of("libc") < idx_of("libb") && idx_of("libb") < idx_of("app"));
    CHECK(plan.act[idx_of("app")].automatic == false && plan.act[idx_of("libb")].automatic && plan.act[idx_of("libc")].automatic && plan.download_size == 175);
    /* 2. what is already installed is not touched */
    installed("libb", "1.5", "Depends: libc", true); installed("libc", "2.0", "", true);
    CHECK(solve() && plan.n == 1 && plan_has("app", ACT_INSTALL));
    /* 3. version constraints force an upgrade of an installed dependency */
    reset();
    installed("libz", "1.0", "", true);
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: libz (>= 2)\n\nPackage: libz\nVersion: 1.0\nArchitecture: amd64\n\nPackage: libz\nVersion: 2.5\nArchitecture: amd64\n\nPackage: libz\nVersion: 3.0\nArchitecture: amd64\n", 500);
    want("app");
    CHECK(solve() && plan_has("libz", ACT_UPGRADE) && !strcmp(plan.act[idx_of("libz")].pkg->version, "3.0") && !strcmp(plan.act[idx_of("libz")].old->version, "1.0"));
    /* an upper bound picks the newest version that still fits */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: libz (<< 3)\n\nPackage: libz\nVersion: 2.5\nArchitecture: amd64\n\nPackage: libz\nVersion: 3.0\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && !strcmp(plan.act[idx_of("libz")].pkg->version, "2.5"));

    /* 4. alternatives: first choice, then fall through when the first is impossible */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: editor-a | editor-b\n\nPackage: editor-a\nVersion: 1\nArchitecture: amd64\n\nPackage: editor-b\nVersion: 1\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && plan_has("editor-a", ACT_INSTALL) && idx_of("editor-b") < 0);
    reset();
    installed("blocker", "1", "Conflicts: editor-a", false);
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: editor-a | editor-b\n\nPackage: editor-a\nVersion: 1\nArchitecture: amd64\n\nPackage: editor-b\nVersion: 1\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && plan_has("editor-b", ACT_INSTALL) && idx_of("editor-a") < 0);
    /* an installed alternative wins over a fresh one */
    reset(); installed("editor-b", "1", "", false);
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: editor-a | editor-b\n\nPackage: editor-a\nVersion: 1\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && plan.n == 1);
    /* backtracking: the first alternative's own dependency cannot be met */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: first | second\n\nPackage: first\nVersion: 1\nArchitecture: amd64\nDepends: missing-lib\n\nPackage: second\nVersion: 1\nArchitecture: amd64\nDepends: present\n\nPackage: present\nVersion: 1\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && plan_has("second", ACT_INSTALL) && plan_has("present", ACT_INSTALL) && idx_of("first") < 0);

    /* 5. virtual packages */
    reset();
    avail("Package: mailer\nVersion: 1\nArchitecture: amd64\nDepends: mail-transport-agent\n\nPackage: postfix\nVersion: 3\nArchitecture: amd64\nProvides: mail-transport-agent\n\nPackage: exim4\nVersion: 4\nArchitecture: amd64\nProvides: mail-transport-agent\n", 500);
    want("mailer"); CHECK(solve() && plan.n == 2 && (plan_has("postfix", ACT_INSTALL) || plan_has("exim4", ACT_INSTALL)));
    installed("exim4", "4", "Provides: mail-transport-agent", false);
    CHECK(solve() && plan.n == 1 && plan_has("mailer", ACT_INSTALL));                                                     /* the installed provider satisfies it */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: abi (= 3)\n\nPackage: old\nVersion: 1\nArchitecture: amd64\nProvides: abi (= 2)\n\nPackage: new\nVersion: 1\nArchitecture: amd64\nProvides: abi (= 3)\n\nPackage: unversioned\nVersion: 1\nArchitecture: amd64\nProvides: abi\n", 500);
    want("app"); CHECK(solve() && plan_has("new", ACT_INSTALL) && idx_of("old") < 0 && idx_of("unversioned") < 0);          /* only a versioned provider can meet a versioned dependency */

    /* 7. conflicts: refuse without allow_remove, remove with it */
    reset();
    installed("old-tool", "1", "", false);
    avail("Package: new-tool\nVersion: 2\nArchitecture: amd64\nConflicts: old-tool\nReplaces: old-tool\n", 500);
    want("new-tool"); CHECK(!solve() && strstr(plan.error, "new-tool") && strstr(plan.error, "old-tool"));
    rq.allow_remove = true; CHECK(solve() && plan_has("old-tool", ACT_REMOVE) && plan_has("new-tool", ACT_INSTALL) && idx_of("old-tool") < idx_of("new-tool"));    /* removals first */
    /* the other direction: an installed package conflicting with the newcomer */
    reset();
    installed("legacy", "1", "Conflicts: shiny", false);
    avail("Package: shiny\nVersion: 1\nArchitecture: amd64\n", 500);
    want("shiny"); CHECK(!solve()); rq.allow_remove = true; CHECK(solve() && plan_has("legacy", ACT_REMOVE));
    /* conflict against a virtual name */
    reset();
    installed("exim4", "4", "Provides: mta", false);
    avail("Package: postfix\nVersion: 3\nArchitecture: amd64\nProvides: mta\nConflicts: mta\n", 500);
    want("postfix"); rq.allow_remove = true; CHECK(solve() && plan_has("exim4", ACT_REMOVE) && plan_has("postfix", ACT_INSTALL));       /* a package never conflicts with its own Provides */

    /* 8. Breaks resolved by upgrading the broken package */
    reset();
    installed("libc6", "2.35", "", false); installed("fakeroot", "1.28", "Depends: libc6", false);
    avail("Package: libc6\nVersion: 2.36\nArchitecture: amd64\nBreaks: fakeroot (<< 1.30)\n\nPackage: fakeroot\nVersion: 1.28\nArchitecture: amd64\nDepends: libc6\n\nPackage: fakeroot\nVersion: 1.31\nArchitecture: amd64\nDepends: libc6\n", 500);
    want("libc6"); CHECK(solve() && plan_has("libc6", ACT_UPGRADE) && plan_has("fakeroot", ACT_UPGRADE) && !strcmp(plan.act[idx_of("fakeroot")].pkg->version, "1.31"));
    /* no fixed version available: refuse unless removal is allowed */
    reset(); installed("libc6", "2.35", "", false); installed("fakeroot", "1.28", "Depends: libc6", false);
    avail("Package: libc6\nVersion: 2.36\nArchitecture: amd64\nBreaks: fakeroot (<< 1.30)\n", 500);
    want("libc6"); CHECK(solve() && plan.n == 0);                                          /* the upgrade is impossible, so the installed libc6 simply stays */
    rq.ninstall = 0; want("libc6=2.36"); CHECK(!solve() && strstr(plan.error, "libc6"));                             /* an explicit version is a hard requirement */
    rq.allow_remove = true; CHECK(solve() && plan_has("fakeroot", ACT_REMOVE) && plan_has("libc6", ACT_UPGRADE));
    rq.ninstall = 0; want("libc6"); CHECK(solve() && plan_has("fakeroot", ACT_REMOVE) && plan_has("libc6", ACT_UPGRADE));

    /* 9. diamonds and cycles */
    reset();
    avail("Package: a\nVersion: 1\nArchitecture: amd64\nDepends: b, c\n\nPackage: b\nVersion: 1\nArchitecture: amd64\nDepends: d\n\nPackage: c\nVersion: 1\nArchitecture: amd64\nDepends: d (>= 1)\n\nPackage: d\nVersion: 1\nArchitecture: amd64\nDepends: b\n", 500);
    want("a"); CHECK(solve() && plan.n == 4);
    /* 12. Pre-Depends are configured first */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nPre-Depends: pre\nDepends: other\n\nPackage: pre\nVersion: 1\nArchitecture: amd64\nDepends: base\n\nPackage: base\nVersion: 1\nArchitecture: amd64\n\nPackage: other\nVersion: 1\nArchitecture: amd64\n", 500);
    want("app"); CHECK(solve() && plan.n == 4 && idx_of("base") < idx_of("pre") && idx_of("pre") < idx_of("app") && idx_of("other") < idx_of("app"));

    /* 10. unsatisfiable requests explain themselves */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nDepends: ghost-lib\n", 500);
    want("app"); CHECK(!solve() && strstr(plan.error, "app") && strstr(plan.error, "ghost-lib"));
    reset(); want("nonexistent-pkg"); CHECK(!solve() && strstr(plan.error, "unable to locate package nonexistent-pkg"));
    reset(); avail("Package: p\nVersion: 1\nArchitecture: amd64\n", 500); want("p=9.9"); CHECK(!solve() && strstr(plan.error, "p"));
    reset(); avail("Package: p\nVersion: 1\nArchitecture: amd64\nDepends: q (>= 5)\n\nPackage: q\nVersion: 2\nArchitecture: amd64\n", 500); want("p"); CHECK(!solve() && strstr(plan.error, "q"));
    /* mutually exclusive requests */
    reset(); avail("Package: x\nVersion: 1\nArchitecture: amd64\nConflicts: y\n\nPackage: y\nVersion: 1\nArchitecture: amd64\n", 500); want("x"); want("y"); CHECK(!solve());

    /* 13/14. removal */
    reset();
    installed("lib", "1", "", true); installed("app", "1", "Depends: lib", false);
    rq.remove[rq.nremove++] = "lib";
    CHECK(!solve() && strstr(plan.error, "app"));
    rq.allow_remove = true; CHECK(solve() && plan.n == 2 && plan_has("app", ACT_REMOVE) && plan_has("lib", ACT_REMOVE) && idx_of("app") < idx_of("lib"));    /* dependents go first */
    rq.purge = true; CHECK(solve() && plan_has("lib", ACT_PURGE));
    reset(); installed("base-files", "1", "Essential: yes", false); installed("other", "1", "", false);
    rq.remove[rq.nremove++] = "base-files"; rq.allow_remove = true; CHECK(!solve() && strstr(plan.error, "essential"));
    reset(); installed("core", "1", "Essential: yes", false); installed("sys", "1", "Depends: core", false);
    rq.remove[rq.nremove++] = "sys"; CHECK(solve() && plan.n == 1 && plan_has("sys", ACT_REMOVE));                              /* removing a dependent never touches what it needs */
    reset(); installed("keeper", "1", "Depends: core", false); installed("core", "1", "Essential: yes", false);
    rq.remove[rq.nremove++] = "core"; rq.allow_remove = true; CHECK(!solve());

    /* 15. holds */
    reset(); installed("pinned", "1.0", "", false);
    avail("Package: pinned\nVersion: 2.0\nArchitecture: amd64\n\nPackage: needs-new\nVersion: 1\nArchitecture: amd64\nDepends: pinned (>= 2)\n", 500);
    want("needs-new"); CHECK(solve() && plan_has("pinned", ACT_UPGRADE));
    rq.hold[rq.nhold++] = "pinned"; CHECK(!solve());
    rq.nhold = 0; dpkg_db_find(&db, "pinned", NULL)->want = WANT_HOLD; CHECK(!solve());                                          /* dpkg's own hold state counts */
    rq.nremove = 0; rq.ninstall = 0; rq.remove[rq.nremove++] = "pinned"; rq.allow_remove = true; CHECK(!solve() && strstr(plan.error, "held"));

    /* 16. upgrade everything that can be upgraded */
    reset();
    installed("a", "1", "", false); installed("b", "1", "Depends: c (= 1)", false); installed("c", "1", "", false); installed("d", "1", "", false);
    avail("Package: a\nVersion: 2\nArchitecture: amd64\n\nPackage: b\nVersion: 2\nArchitecture: amd64\nDepends: c (= 2)\n\nPackage: c\nVersion: 2\nArchitecture: amd64\n\nPackage: d\nVersion: 2\nArchitecture: amd64\nConflicts: a (>= 2)\n", 500);
    rq.upgrade_all = true; CHECK(solve());
    CHECK(plan_has("a", ACT_UPGRADE) || plan_has("d", ACT_UPGRADE));
    CHECK(!(plan_has("a", ACT_UPGRADE) && plan_has("d", ACT_UPGRADE)));                                                          /* the conflicting pair cannot both move */
    CHECK(plan.nkept == 1 && (!strcmp(plan.kept_back[0], "a") || !strcmp(plan.kept_back[0], "d")));
    CHECK(plan_has("b", ACT_UPGRADE) && plan_has("c", ACT_UPGRADE) && idx_of("c") < idx_of("b"));                                /* c is a dependency of b's new version */
    for (int i = 0; i < plan.n; i++) CHECK(plan.act[i].act != ACT_REMOVE);                                                       /* upgrade never removes */

    /* 17. downgrades need consent */
    reset(); installed("pkg", "2.0", "", false); avail("Package: pkg\nVersion: 1.0\nArchitecture: amd64\n\nPackage: pkg\nVersion: 2.0\nArchitecture: amd64\n", 500);
    want("pkg=1.0"); CHECK(!solve()); rq.allow_downgrade = true; CHECK(solve() && plan_has("pkg", ACT_DOWNGRADE));
    reset(); installed("pkg", "2.0", "", false); avail("Package: pkg\nVersion: 2.0\nArchitecture: amd64\n", 500); want("pkg"); CHECK(solve() && plan.n == 0);       /* already satisfied */

    /* 18. multiarch */
    reset();
    avail("Package: wine32\nVersion: 1\nArchitecture: i386\nDepends: libc6:i386 (>= 2)\n\nPackage: libc6\nVersion: 2.35\nArchitecture: i386\nMulti-Arch: same\n\nPackage: libc6\nVersion: 2.35\nArchitecture: amd64\nMulti-Arch: same\n", 500);
    installed("libc6", "2.35", "Multi-Arch: same", false);
    want("wine32"); CHECK(solve() && plan.n == 2 && plan_has("libc6", ACT_INSTALL) && !strcmp(plan.act[idx_of("libc6")].pkg->arch, "i386"));
    reset(); avail("Package: native-app\nVersion: 1\nArchitecture: amd64\nDepends: tool\n\nPackage: tool\nVersion: 1\nArchitecture: i386\n\nPackage: tool\nVersion: 1\nArchitecture: amd64\n", 500);
    want("native-app"); CHECK(solve() && plan.n == 2 && !strcmp(plan.act[idx_of("tool")].pkg->arch, "amd64"));                    /* an unqualified dependency means the native architecture */

    /* 19. recommends */
    reset();
    avail("Package: app\nVersion: 1\nArchitecture: amd64\nRecommends: nice, impossible\n\nPackage: nice\nVersion: 1\nArchitecture: amd64\n\nPackage: impossible\nVersion: 1\nArchitecture: amd64\nDepends: nowhere\n", 500);
    want("app"); CHECK(solve() && plan.n == 1);
    rq.install_recommends = true; CHECK(solve() && plan.n == 2 && plan_has("nice", ACT_INSTALL) && plan.act[idx_of("nice")].automatic && idx_of("impossible") < 0);

    /* 22. pin priorities decide between sources */
    reset();
    avail("Package: tool\nVersion: 2.0\nArchitecture: amd64\n", 500);
    avail("Package: tool\nVersion: 1.0\nArchitecture: amd64\n", 990);
    want("tool"); CHECK(solve() && !strcmp(plan.act[idx_of("tool")].pkg->version, "1.0"));
    reset(); avail("Package: tool\nVersion: 2.0\nArchitecture: amd64\n", 500); avail("Package: tool\nVersion: 1.0\nArchitecture: amd64\n", 100);
    want("tool"); CHECK(solve() && !strcmp(plan.act[idx_of("tool")].pkg->version, "2.0"));
    CHECK(repo_best(&repo, "tool") && !strcmp(repo_best(&repo, "tool")->p.version, "2.0") && !repo_best(&repo, "nope"));

    /* 20. autoremove */
    reset();
    installed("manual", "1", "Depends: auto-b", false); installed("auto-b", "1", "Depends: auto-c | other", true); installed("auto-c", "1", "", true); installed("orphan", "1", "", true);
    installed("base", "1", "Essential: yes", true);
    char gone[8][64]; int ng = solver_autoremove(&db, false, gone, 8);
    CHECK(ng == 1 && !strcmp(gone[0], "orphan"));
    dpkg_db_remove(&db, "manual", NULL, true);
    ng = solver_autoremove(&db, false, gone, 8); CHECK(ng == 3);                                                                  /* auto-b, auto-c, orphan; the essential one stays */
    installed("kept", "1", "Recommends: auto-c", false); ng = solver_autoremove(&db, false, gone, 8); CHECK(ng == 3);
    ng = solver_autoremove(&db, true, gone, 8); CHECK(ng == 2);                                                                   /* keeping Recommends saves auto-c */
    dpkg_db_find(&db, "orphan", NULL)->want = WANT_HOLD; ng = solver_autoremove(&db, true, gone, 8); CHECK(ng == 1);

    /* 21. a large archive: thousands of packages, deep chains with alternatives */
    reset();
    {
        size_t cap = 8 << 20; char *buf = malloc(cap); size_t n = 0;
        for (int i = 0; i < 20000; i++) {
            n += (size_t)snprintf(buf + n, cap - n, "Package: p%d\nVersion: %d.0\nArchitecture: amd64\nDepends: p%d (>= 1) | q%d, p%d\nSize: 1000\n\n", i, 1 + i % 3, i > 0 ? i - 1 : 0, i, i > 1 ? i - 2 : 0);
            if (i < 19999 && i % 2) n += (size_t)snprintf(buf + n, cap - n, "Package: q%d\nVersion: 1\nArchitecture: amd64\n\n", i);
        }
        n += (size_t)snprintf(buf + n, cap - n, "Package: top\nVersion: 1\nArchitecture: amd64\nDepends: p400\n\n");
        CHECK(repo_add_packages(&repo, buf, n, 0, 500) > 20000);
        free(buf);
        want("top");
        clock_t t0 = clock();
        bool ok = solve();
        double ms = (double)(clock() - t0) * 1000.0 / CLOCKS_PER_SEC;
        CHECK(ok && plan.n >= 400);
        if (!ok) printf("error: %s\n", plan.error);
        printf("  large archive: %d packages resolved in %.0f ms, %d steps\n", plan.n, ms, plan.steps);
        CHECK(ms < 5000);
        CHECK(idx_of("p0") >= 0 && idx_of("p0") < idx_of("p400") && idx_of("p400") < idx_of("top"));
    }
    repo_free(&repo); dpkg_db_free(&db); solver_plan_free(&plan);
    DONE("solver_test");
}
