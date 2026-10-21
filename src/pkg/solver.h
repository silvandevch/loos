/* Dependency resolver for the apt engine. Input: the available packages (repo), what is installed (dpkg_db) and a
 * request. Output: an ordered plan of installs/upgrades/removals, or a readable explanation of why no consistent
 * system exists. Handles Depends/Pre-Depends alternatives, virtual packages (Provides), Conflicts/Breaks, version
 * constraints, Essential and held packages, multiarch, "upgrade everything that can be upgraded safely" and autoremove. */
#ifndef LOOS_PKG_SOLVER_H
#define LOOS_PKG_SOLVER_H
#include "dpkgdb.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    deb_pkg p;
    int source;                    /* which repository it came from */
    int priority;                  /* pin priority: higher wins (default 500; 100 = installed-only, 990 = target release) */
} repo_pkg;
typedef struct { repo_pkg *a; int n, cap; char native[16]; } deb_repo;

void repo_init(deb_repo *r, const char *native_arch);
void repo_free(deb_repo *r);
int  repo_add_packages(deb_repo *r, const char *text, size_t len, int source, int priority);     /* Packages file text; returns count, -1 on a syntax error */
const repo_pkg *repo_best(const deb_repo *r, const char *name);                                  /* highest version by priority then version */

typedef struct { const char *name; deb_rel rel; const char *version; } solver_target;           /* "pkg" or "pkg (>= 1.2)" */
typedef struct {
    solver_target install[32]; int ninstall;
    const char *remove[32]; int nremove;
    bool upgrade_all;               /* upgrade every installed package that has a safe newer version; never removes anything */
    bool allow_remove;              /* installing may remove conflicting packages (apt's --allow-remove) */
    bool allow_downgrade;
    bool purge;                     /* removals also drop configuration */
    bool install_recommends;        /* pull Recommends in as soft dependencies */
    const char *hold[32]; int nhold;
} solver_request;

typedef enum { ACT_INSTALL, ACT_UPGRADE, ACT_DOWNGRADE, ACT_REMOVE, ACT_PURGE } solver_act;
typedef struct {
    solver_act act;
    const deb_pkg *pkg;             /* the version to end up with (installs/upgrades) or the installed one (removals) */
    const deb_pkg *old;             /* installed version for upgrades/downgrades */
    bool automatic;                 /* installed only to satisfy a dependency */
    char why[96];
} solver_action;
typedef struct {
    solver_action *act; int n;
    char error[512];
    char kept_back[16][64]; int nkept;
    uint64_t download_size, install_size_delta_kb;
    int steps;
} solver_plan;

bool solver_resolve(const deb_repo *repo, const dpkg_db *db, const solver_request *req, solver_plan *plan);
void solver_plan_free(solver_plan *plan);
int  solver_autoremove(const dpkg_db *db, bool keep_recommends, char out[][64], int max);       /* auto-installed packages nothing needs any more */
bool solver_parse_target(const char *spec, solver_target *t, char *namebuf, size_t cap);        /* "name", "name=1.2", "name (>= 1.2)" */

#ifdef __cplusplus
}
#endif
#endif
