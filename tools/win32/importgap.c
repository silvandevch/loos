/* importgap: how much of a Windows binary's import surface does LoOS provide?
 *
 *   importgap [--alias from.dll=to.dll]... <provider-dir> <target.exe|dll>...
 *
 * Providers are the *.dll files in <provider-dir> (LoOS system DLLs). Each target's imports are
 * resolved the way the loader would (API sets -> host module), then looked up in the provider's
 * export table. Output is a Markdown report: per target, what is missing from modules we do
 * ship, and which modules we do not ship at all. Read-only; never executes anything.
 */
#include "pe.h"
#include "../../src/win32/apisets/apiset.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <dirent.h>

#define MAXMOD 64
#define MAXSYM 8192

typedef struct { char name[64]; uint8_t *data; size_t size; pe_image img; } provider;
typedef struct { char from[64], to[64]; } alias;

static provider P[MAXMOD]; static int np;
static alias A[16]; static int na;

static uint8_t *slurp(const char *path, size_t *n)
{
    FILE *f = fopen(path, "rb"); long l; uint8_t *b;
    if (!f) return NULL;
    fseek(f, 0, SEEK_END); l = ftell(f); fseek(f, 0, SEEK_SET);
    b = malloc((size_t)l);
    if (b && fread(b, 1, (size_t)l, f) != (size_t)l) { free(b); b = NULL; }
    fclose(f); *n = (size_t)l; return b;
}

static provider *find_provider(const char *host)
{
    int i;
    for (i = 0; i < na; i++) if (!strcasecmp(A[i].from, host)) { host = A[i].to; break; }
    for (i = 0; i < np; i++) if (!strcasecmp(P[i].name, host)) return &P[i];
    return NULL;
}

typedef struct { char mod[64]; char sym[96]; int ordinal; } missing_t;
typedef struct {
    int total, found, missing_n, nomod_syms;
    missing_t *missing; int missing_cap;
    char nomod[MAXMOD][64]; int nomod_count[MAXMOD]; int nnomod;
} report;

static int visit(void *ctx, const pe_import_ref *r)
{
    report *rep = ctx;
    provider *p = find_provider(r->host);
    rep->total++;
    if (!p) {
        int i;
        for (i = 0; i < rep->nnomod; i++) if (!strcasecmp(rep->nomod[i], r->host)) break;
        if (i == rep->nnomod && i < MAXMOD) { strncpy(rep->nomod[i], r->host, 63); rep->nnomod++; }
        if (i < MAXMOD) rep->nomod_count[i]++;
        rep->nomod_syms++;
        return 0;
    }
    {
        pe_export ex;
        pe_status s = r->name ? pe_find_export(&p->img, r->name, 0, &ex) : pe_find_export(&p->img, NULL, r->ordinal, &ex);
        if (s == PE_OK) { rep->found++; return 0; }
        if (rep->missing_n < rep->missing_cap) {
            missing_t *m = &rep->missing[rep->missing_n];
            strncpy(m->mod, p->name, 63); m->mod[63] = 0;
            if (r->name) strncpy(m->sym, r->name, 95); else snprintf(m->sym, sizeof m->sym, "#%u", r->ordinal);
            m->sym[95] = 0;
        }
        rep->missing_n++;
    }
    return 0;
}

int main(int argc, char **argv)
{
    int i, ai = 1;
    DIR *d; struct dirent *de;
    char path[600];

    while (ai + 1 < argc && !strcmp(argv[ai], "--alias")) {
        char *eq = strchr(argv[ai + 1], '=');
        if (!eq || na >= 16) { fprintf(stderr, "bad --alias\n"); return 2; }
        *eq = 0; strncpy(A[na].from, argv[ai + 1], 63); strncpy(A[na].to, eq + 1, 63); na++; ai += 2;
    }
    if (argc - ai < 2) { fprintf(stderr, "usage: importgap [--alias a.dll=b.dll] <provider-dir> <target>...\n"); return 2; }
    if (!(d = opendir(argv[ai]))) { perror(argv[ai]); return 2; }
    while ((de = readdir(d)) && np < MAXMOD) {
        size_t l = strlen(de->d_name);
        provider *p = &P[np];
        if (l < 5 || strcasecmp(de->d_name + l - 4, ".dll")) continue;
        snprintf(path, sizeof path, "%s/%s", argv[ai], de->d_name);
        if (!(p->data = slurp(path, &p->size))) continue;
        if (pe_open(&p->img, p->data, p->size, PE_LAYOUT_FILE)) { free(p->data); continue; }
        strncpy(p->name, de->d_name, 63); np++;
    }
    closedir(d);
    printf("# Import gap report\n\nProviders: ");
    for (i = 0; i < np; i++) printf("%s%s", i ? ", " : "", P[i].name);
    printf("\n\n");

    for (i = ai + 1; i < argc; i++) {
        size_t n; uint8_t *buf = slurp(argv[i], &n); pe_image img; report rep; int k;
        const char *base = strrchr(argv[i], '/'); base = base ? base + 1 : argv[i];
        if (!buf) { printf("## %s\n\nunreadable\n\n", base); continue; }
        if (pe_open(&img, buf, n, PE_LAYOUT_FILE)) { printf("## %s\n\nnot a loadable PE\n\n", base); free(buf); continue; }
        memset(&rep, 0, sizeof rep);
        rep.missing_cap = MAXSYM; rep.missing = calloc(MAXSYM, sizeof(missing_t));
        pe_enum_imports(&img, visit, &rep);
        printf("## %s (%s)\n\n- imports: %d, satisfied by LoOS modules: %d, missing from LoOS modules: %d, in modules LoOS lacks: %d\n",
               base, img.is64 ? "PE32+" : "PE32", rep.total, rep.found, rep.missing_n, rep.nomod_syms);
        if (rep.missing_n) {
            printf("- **missing from shipped modules:**");
            for (k = 0; k < rep.missing_n && k < rep.missing_cap; k++) printf(" `%s!%s`", rep.missing[k].mod, rep.missing[k].sym);
            printf("\n");
        }
        if (rep.nnomod) {
            printf("- **modules not shipped (symbols):**");
            for (k = 0; k < rep.nnomod; k++) printf(" %s(%d)", rep.nomod[k], rep.nomod_count[k]);
            printf("\n");
        }
        printf("\n");
        free(rep.missing); free(buf);
    }
    return 0;
}
