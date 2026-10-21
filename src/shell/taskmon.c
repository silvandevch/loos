#include "taskmon.h"
#include <string.h>
#include <stdio.h>

static int ci_cmp(const char *a, const char *b)
{
    for (;; a++, b++) {
        int x = *a, y = *b;
        if (x >= 'A' && x <= 'Z') x += 32;
        if (y >= 'A' && y <= 'Z') y += 32;
        if (x != y || !x) return x - y;
    }
}
static bool ci_contains(const char *h, const char *n)
{
    if (!*n) return true;
    for (; *h; h++) {
        const char *a = h, *b = n;
        for (; *a && *b; a++, b++) { int x = *a, y = *b; if (x >= 'A' && x <= 'Z') x += 32; if (y >= 'A' && y <= 'Z') y += 32; if (x != y) break; }
        if (!*b) return true;
    }
    return false;
}

static int cmp_proc(const shell_taskmon *t, const ps_proc *a, const ps_proc *b)
{
    int c;
    switch (t->sort) {
    case TM_SORT_PID: c = a->pid < b->pid ? -1 : a->pid > b->pid; break;
    case TM_SORT_CPU: c = a->cpu_x10 < b->cpu_x10 ? -1 : a->cpu_x10 > b->cpu_x10; break;
    case TM_SORT_MEM: c = a->mem < b->mem ? -1 : a->mem > b->mem; break;
    case TM_SORT_THREADS: c = a->threads < b->threads ? -1 : a->threads > b->threads; break;
    case TM_SORT_SOCKETS: c = a->sockets < b->sockets ? -1 : a->sockets > b->sockets; break;
    default: c = ci_cmp(a->name, b->name);
    }
    if (c == 0) c = a->pid < b->pid ? -1 : a->pid > b->pid;
    return t->desc ? -c : c;
}

static bool visible(const shell_taskmon *t, int i) { return ci_contains(t->p[i].name, t->filter); }

static void sort_indices(const shell_taskmon *t, int *idx, int n)
{
    for (int i = 1; i < n; i++) {
        int v = idx[i], j = i;
        while (j > 0 && cmp_proc(t, &t->p[idx[j - 1]], &t->p[v]) > 0) { idx[j] = idx[j - 1]; j--; }
        idx[j] = v;
    }
}

static void add_subtree(shell_taskmon *t, int root, int depth, bool *done)
{
    if (done[root] || t->nview >= TM_MAX) return;
    done[root] = true;
    t->view[t->nview] = root; t->depth[t->nview] = depth; t->nview++;
    int kids[TM_MAX], nk = 0;
    for (int i = 0; i < t->n; i++) if (!done[i] && t->p[i].ppid == t->p[root].pid && t->p[i].pid != t->p[root].pid) kids[nk++] = i;
    sort_indices(t, kids, nk);
    for (int k = 0; k < nk; k++) add_subtree(t, kids[k], depth + 1, done);
}

static void build_view(shell_taskmon *t)
{
    t->nview = 0;
    if (t->filter[0] || !t->tree) {                                   /* filtering or flat mode: plain sorted list */
        int idx[TM_MAX], n = 0;
        for (int i = 0; i < t->n; i++) if (visible(t, i)) idx[n++] = i;
        sort_indices(t, idx, n);
        for (int i = 0; i < n; i++) { t->view[i] = idx[i]; t->depth[i] = 0; }
        t->nview = n;
    } else {
        bool done[TM_MAX] = { false };
        int roots[TM_MAX], nr = 0;
        for (int i = 0; i < t->n; i++) {
            bool has_parent = false;
            for (int k = 0; k < t->n; k++) if (t->p[k].pid == t->p[i].ppid && k != i) has_parent = true;
            if (!has_parent) roots[nr++] = i;
        }
        sort_indices(t, roots, nr);
        for (int r = 0; r < nr; r++) add_subtree(t, roots[r], 0, done);
        for (int i = 0; i < t->n; i++) if (!done[i]) add_subtree(t, i, 0, done);          /* cycles / orphans */
    }
    t->sel = 0;
    for (int i = 0; i < t->nview; i++) if (t->p[t->view[i]].pid == t->selected_pid) t->sel = i;
    if (t->nview) t->selected_pid = t->p[t->view[t->sel]].pid; else t->selected_pid = 0;
}

void tm_refresh(shell_taskmon *t)
{
    t->n = t->be->ps_snapshot(t->be->user, t->p, TM_MAX);
    t->nconns = t->be->net_query(t->be->user, t->conns, 32);
    t->total_cpu_x10 = 0; t->total_mem = 0; t->total_sockets = 0;
    for (int i = 0; i < t->n; i++) {
        uint32_t s = 0;
        for (int k = 0; k < t->nconns; k++) if (t->conns[k].pid == t->p[i].pid) s++;
        t->p[i].sockets = s;
        t->total_cpu_x10 += t->p[i].cpu_x10; t->total_mem += t->p[i].mem; t->total_sockets += s;
    }
    build_view(t);
}

void tm_init(shell_taskmon *t, const shell_backend *be) { memset(t, 0, sizeof *t); t->be = be; t->tree = true; t->sort = TM_SORT_NAME; tm_refresh(t); }
void tm_set_sort(shell_taskmon *t, tm_sort s)
{
    if (t->sort == s) t->desc = !t->desc; else { t->sort = s; t->desc = s != TM_SORT_NAME && s != TM_SORT_PID; }
    build_view(t);
}
const ps_proc *tm_selected(const shell_taskmon *t) { return t->nview && t->sel < t->nview ? &t->p[t->view[t->sel]] : NULL; }

bool tm_kill_selected(shell_taskmon *t)
{
    const ps_proc *p = tm_selected(t);
    if (!p) return false;
    char name[32]; strcpy(name, p->name);
    uint32_t pid = p->pid;
    if (!t->be->ps_kill(t->be->user, pid)) { snprintf(t->status, sizeof t->status, "Cannot end %s (%u): protected or already gone", name, pid); return false; }
    snprintf(t->status, sizeof t->status, "Ended %s (%u)", name, pid);
    tm_refresh(t);
    return true;
}

int tm_proc_sockets(const shell_taskmon *t, uint32_t pid, net_conn *out, int max)
{
    int n = 0;
    for (int i = 0; i < t->nconns && n < max; i++) if (t->conns[i].pid == pid) out[n++] = t->conns[i];
    return n;
}

static void row_proc(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_taskmon *t = user;
    const ps_proc *p = &t->p[t->view[i]];
    const shell_theme *th = u->theme;
    uint32_t tc = sel ? th->accent_text : th->text, dc = sel ? th->accent_text : th->text_dim;
    char b[32];
    int indent = t->depth[i] * 16;
    if (t->tree && !t->filter[0]) {
        bool has_kids = false;
        for (int k = 0; k < t->n; k++) if (t->p[k].ppid == p->pid && t->p[k].pid != p->pid) has_kids = true;
        if (indent) fb_fill(u->fb, u->clip, (shell_rect){ row.x + 8 + indent - 8, row.y + 10, 6, 1 }, th->border);
        if (has_kids) fb_fill(u->fb, u->clip, (shell_rect){ row.x + 8 + indent - 2, row.y + 6, 5, 5 }, th->text_dim);
    }
    ui_label(u, row.x + 14 + indent, row.y + 3, p->name, tc);
    snprintf(b, sizeof b, "%u", p->pid); ui_label_right(u, row.x + row.w - 400, row.y + 3, b, dc);
    fmt_pct10(p->cpu_x10, b, sizeof b); ui_label_right(u, row.x + row.w - 320, row.y + 3, b, p->cpu_x10 >= 500 ? th->danger : dc);
    fmt_size(p->mem, b, sizeof b); ui_label_right(u, row.x + row.w - 220, row.y + 3, b, dc);
    snprintf(b, sizeof b, "%u", p->threads); ui_label_right(u, row.x + row.w - 130, row.y + 3, b, dc);
    snprintf(b, sizeof b, "%u", p->sockets); ui_label_right(u, row.x + row.w - 20, row.y + 3, b, p->sockets ? 0xFFF59E0Bu : dc);
}

void tm_frame(shell_taskmon *t, ui_ctx *u, int w, int h)
{
    const shell_theme *th = u->theme;
    ui_clear(u, th->surface);
    bool modal = t->confirm_kill;
    struct { uint32_t key, mods; bool is_char; } parked[UI_KEYQ]; int np = 0; bool pp = false, pr = false; int pw = 0;
    if (modal) {                                                      /* underlying widgets draw without input while the confirm is up */
        np = u->nkeys;
        for (int i = 0; i < np; i++) { parked[i].key = u->keys[i].key; parked[i].mods = u->keys[i].mods; parked[i].is_char = u->keys[i].is_char; }
        pp = u->pressed; pr = u->released; pw = u->wheel; u->nkeys = 0; u->pressed = u->released = false; u->wheel = 0;
    }
    /* summary strip */
    char b[96], c1[16], c2[24];
    fmt_pct10(t->total_cpu_x10 > 1000 ? 1000 : t->total_cpu_x10, c1, sizeof c1); fmt_size(t->total_mem, c2, sizeof c2);
    snprintf(b, sizeof b, "Processes %d    CPU %s    Memory %s    Sockets %u", t->n, c1, c2, t->total_sockets);
    ui_label(u, 12, 12, b, th->text);
    bool tree_before = t->tree;
    ui_toggle(u, (shell_rect){ w - 360, 8, 150, 26 }, "Tree", &t->tree);
    if (tree_before != t->tree) build_view(t);
    char before[32]; strcpy(before, t->filter);
    ui_textfield(u, (shell_rect){ w - 200, 6, 190, 28 }, t->filter, sizeof t->filter, &t->filter_caret, "Search");
    if (strcmp(before, t->filter)) build_view(t);
    /* header */
    int top = 44, bottom = h - 170;
    shell_rect hdr = { 8, top, w - 16, 24 };
    static const char *const names[6] = { "Name", "PID", "CPU", "Memory", "Threads", "Sockets" };
    int rx[6] = { 22, w - 16 - 400, w - 16 - 320, w - 16 - 220, w - 16 - 130, w - 16 - 20 };
    for (int i = 0; i < 6; i++) {
        char lb[24]; snprintf(lb, sizeof lb, "%s%s", names[i], t->sort == (tm_sort)i ? (t->desc ? " v" : " ^") : "");
        if (i == 0) ui_label(u, 8 + rx[i], top + 4, lb, th->text_dim); else ui_label_right(u, 8 + rx[i], top + 4, lb, th->text_dim);
    }
    if (u->pressed && ui_hovered(u, hdr)) {
        int x = u->mx - 8, pick = 0;
        for (int i = 1; i < 6; i++) if (x > rx[i] - 70) pick = i;
        tm_set_sort(t, (tm_sort)pick);
    }
    fb_hline(u->fb, u->clip, 8, top + 24, w - 16, th->border);
    uint32_t keep = t->selected_pid;
    int act = ui_list(u, (shell_rect){ 8, top + 26, w - 16, bottom - top - 26 }, "procs", t->nview, 22, row_proc, t, &t->sel, &t->scroll);
    if (t->nview && t->sel < t->nview) t->selected_pid = t->p[t->view[t->sel]].pid;
    (void)keep; (void)act;
    /* detail panel */
    const ps_proc *p = tm_selected(t);
    int dy = bottom + 10;
    fb_hline(u->fb, u->clip, 8, dy - 4, w - 16, th->border);
    if (p) {
        char io1[24], io2[24];
        fmt_size(p->io_read, io1, sizeof io1); fmt_size(p->io_write, io2, sizeof io2);
        snprintf(b, sizeof b, "%s  pid %u  parent %u  handles %u  disk read %s  write %s", p->name, p->pid, p->ppid, p->handles, io1, io2);
        ui_label(u, 12, dy, b, th->text);
        net_conn sk[8]; int n = tm_proc_sockets(t, p->pid, sk, 8);
        if (!n) ui_label(u, 12, dy + 24, "No open sockets.", th->text_dim);
        for (int i = 0; i < n && i < 4; i++) {
            snprintf(b, sizeof b, "%s %s:%u  %s", sk[i].proto, sk[i].remote, (unsigned)sk[i].port, sk[i].state);
            ui_label(u, 12, dy + 24 + i * 18, b, 0xFFF59E0Bu);
        }
        if (ui_button_accent(u, (shell_rect){ w - 140, dy + 4, 128, 30 }, "End task")) t->confirm_kill = true;
    } else ui_label(u, 12, dy, "Select a process.", th->text_dim);
    ui_label(u, 12, h - 24, t->status, th->text_dim);
    if (modal) {
        for (int i = 0; i < np; i++) { u->keys[i].key = parked[i].key; u->keys[i].mods = parked[i].mods; u->keys[i].is_char = parked[i].is_char; }
        u->nkeys = np; u->pressed = pp; u->released = pr; u->wheel = pw;
        shell_rect m = { w / 2 - 220, h / 2 - 50, 440, 100 };
        fb_fill_round(u->fb, u->clip, (shell_rect){ m.x - 2, m.y - 2, m.w + 4, m.h + 4 }, 10, th->border);
        fb_fill_round(u->fb, u->clip, m, 8, th->surface_hi);
        const ps_proc *k = tm_selected(t);
        snprintf(b, sizeof b, "End %s (%u)? Unsaved data in it is lost.", k ? k->name : "?", k ? k->pid : 0);
        ui_label(u, m.x + 16, m.y + 16, b, th->text);
        if (ui_button_accent(u, (shell_rect){ m.x + 16, m.y + 56, 120, 30 }, "End task")) { tm_kill_selected(t); t->confirm_kill = false; }
        if (ui_button(u, (shell_rect){ m.x + 144, m.y + 56, 120, 30 }, "Cancel") || ui_take_key(u, KEY_ESC, 0)) t->confirm_kill = false;
    }
}
