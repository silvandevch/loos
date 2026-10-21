#include "shell.h"
#include <string.h>

/* tiny space tokenizer (the shell core avoids hidden state and libc extras) */
static char *strtok_r_(char *s)
{
    static char *cur;
    if (s) cur = s;
    if (!cur) return NULL;
    while (*cur == ' ') cur++;
    if (!*cur) { cur = NULL; return NULL; }
    char *start = cur;
    while (*cur && *cur != ' ') cur++;
    if (*cur) *cur++ = 0; else cur = NULL;
    return start;
}

static int lc(int ch) { return (ch >= 'A' && ch <= 'Z') ? ch + 32 : ch; }

/* score one haystack: exact > prefix > word prefix > substring > in-order subsequence (tighter is better) */
static int score_text(const char *q, const char *h, int allow_subseq)
{
    size_t ql = strlen(q), hl = strlen(h);
    if (!ql) return 1;
    if (ql > hl) return 0;
    if (ql == hl) { size_t i; for (i = 0; i < ql && lc(q[i]) == lc(h[i]); i++) { } if (i == ql) return 1000; }
    size_t i = 0;
    for (; i < ql && lc(q[i]) == lc(h[i]); i++) { }
    if (i == ql) return 800;
    for (size_t s = 1; s + ql <= hl; s++) {                        /* word-prefix */
        if (h[s - 1] != ' ' && h[s - 1] != '-' && h[s - 1] != '_' && h[s - 1] != '.') continue;
        size_t k = 0;
        while (k < ql && lc(q[k]) == lc(h[s + k])) k++;
        if (k == ql) return 600 - (int)s;
    }
    for (size_t s = 1; s + ql <= hl; s++) {                        /* substring */
        size_t k = 0;
        while (k < ql && lc(q[k]) == lc(h[s + k])) k++;
        if (k == ql) return 400 - (int)s;
    }
    if (!allow_subseq) return 0;
    size_t qi = 0, first = 0, last = 0;                            /* subsequence */
    for (size_t hi = 0; hi < hl && qi < ql; hi++)
        if (lc(h[hi]) == lc(q[qi])) { if (!qi) first = hi; last = hi; qi++; }
    if (qi == ql) { int span = (int)(last - first) + 1 - (int)ql; return 200 - span * 4 > 20 ? 200 - span * 4 : 20; }
    return 0;
}

int fuzzy_score(const char *q, const char *name, const char *kw)
{
    int s = score_text(q, name, 1);
    int k = 0;
    if (kw && *kw) {                                              /* each keyword scored on its own; weaker than the name */
        char buf[96]; strncpy(buf, kw, sizeof buf - 1); buf[sizeof buf - 1] = 0;
        for (char *w = strtok_r_(buf); w; w = strtok_r_(NULL)) { int t = score_text(q, w, 0) / 2; if (t > k) k = t; }
    }
    return s > k ? s : k;
}

void launcher_init(shell_launcher *l, const shell_inventory *inv, void (*launch)(const shell_item *, void *), void *user)
{
    memset(l, 0, sizeof *l);
    l->inv = inv; l->launch = launch; l->user = user;
    launcher_search(l);
}

void launcher_search(shell_launcher *l)
{
    int score[LAUNCHER_RESULTS];
    l->nhit = 0; l->sel = 0;
    for (int i = 0; i < l->inv->n; i++) {
        const shell_item *it = &l->inv->item[i];
        int s = fuzzy_score(l->query, it->name, it->keywords);
        if (it->kind == ITEM_SETTING && l->nq) { int k = score_text(l->query, "settings", 0) / 2; if (k > s) s = k; }
        if (!s) continue;
        if (it->kind == ITEM_APP) s += 3;                          /* apps edge out files/settings on ties */
        int pos = l->nhit;
        while (pos > 0 && score[pos - 1] < s) pos--;
        if (pos >= LAUNCHER_RESULTS) continue;
        int last = l->nhit < LAUNCHER_RESULTS ? l->nhit : LAUNCHER_RESULTS - 1;
        for (int k = last; k > pos; k--) { score[k] = score[k - 1]; l->hit[k] = l->hit[k - 1]; }
        score[pos] = s; l->hit[pos] = i;
        if (l->nhit < LAUNCHER_RESULTS) l->nhit++;
    }
}

void launcher_open(shell_launcher *l) { l->open = true; l->query[0] = 0; l->nq = 0; launcher_search(l); }
void launcher_close(shell_launcher *l) { l->open = false; }

static void launch_selected(shell_launcher *l)
{
    if (l->sel < l->nhit && l->launch) l->launch(&l->inv->item[l->hit[l->sel]], l->user);
    launcher_close(l);
}

bool launcher_char(shell_launcher *l, uint32_t cp)
{
    if (!l->open) return false;
    if (cp >= 32 && cp < 127 && l->nq < LAUNCHER_QUERY_MAX - 1) { l->query[l->nq++] = (char)cp; l->query[l->nq] = 0; launcher_search(l); }
    return true;
}

bool launcher_key(shell_launcher *l, uint32_t key, uint32_t mods)
{
    if (!l->open) return false;
    (void)mods;
    switch (key) {
    case KEY_ESC: launcher_close(l); break;
    case KEY_BACKSPACE: if (l->nq) { l->query[--l->nq] = 0; launcher_search(l); } break;
    case KEY_DOWN: case KEY_TAB: if (l->nhit) l->sel = (l->sel + 1) % l->nhit; break;
    case KEY_UP: if (l->nhit) l->sel = (l->sel + l->nhit - 1) % l->nhit; break;
    case KEY_ENTER: launch_selected(l); break;
    default: break;
    }
    return true;                                                 /* the launcher is modal while open */
}

#define L_W 440
#define L_ROW 36
#define L_INPUT 44
shell_rect launcher_rect(const shell_comp *c)
{
    shell_rect r = { 8, comp_lh(c) - TASKBAR_H - 8 - (L_INPUT + L_ROW * LAUNCHER_RESULTS + 8), L_W, L_INPUT + L_ROW * LAUNCHER_RESULTS + 8 };
    return r;
}

bool launcher_mouse(shell_launcher *l, const shell_comp *c, int x, int y, bool down)
{
    if (!l->open) return false;
    shell_rect r = launcher_rect(c);
    if (!rect_contains(r, x, y)) { if (down) launcher_close(l); return false; }
    if (down) {
        int row = (y - r.y - L_INPUT) / L_ROW;
        if (y >= r.y + L_INPUT && row >= 0 && row < l->nhit) { l->sel = row; launch_selected(l); }
    }
    return true;
}

void launcher_draw(const shell_launcher *l, const shell_comp *c, shell_fb *fb, shell_rect clip)
{
    if (!l->open) return;
    const shell_theme *t = &c->theme;
    shell_rect r = launcher_rect(c);
    fb_fill_round(fb, clip, (shell_rect){ r.x - 2, r.y - 2, r.w + 4, r.h + 4 }, 10, 0x40000000u);
    fb_fill_round(fb, clip, r, 8, t->border);
    fb_fill_round(fb, clip, (shell_rect){ r.x + 1, r.y + 1, r.w - 2, r.h - 2 }, 7, t->surface);
    shell_rect in = { r.x + 10, r.y + 8, r.w - 20, L_INPUT - 14 };
    fb_fill_round(fb, clip, in, 6, t->surface_hi);
    char shown[LAUNCHER_QUERY_MAX + 2];
    int ty = in.y + (in.h - 16) / 2;
    if (l->nq) { strcpy(shown, l->query); fb_text(fb, clip, in.x + 10, ty, shown, t->text); }
    else fb_text(fb, clip, in.x + 10, ty, "Search apps, files and settings", t->text_dim);
    fb_fill(fb, clip, (shell_rect){ in.x + 10 + (l->nq ? shell_text_width(l->query) : 0), ty, 2, 16 }, t->accent);          /* caret */
    for (int i = 0; i < l->nhit; i++) {
        const shell_item *it = &l->inv->item[l->hit[i]];
        shell_rect row = { r.x + 6, r.y + L_INPUT + i * L_ROW, r.w - 12, L_ROW - 2 };
        if (i == l->sel) fb_fill_round(fb, clip, row, 6, color_mix(t->surface_hi, t->accent, 90));
        shell_rect icon = { row.x + 10, row.y + 8, 16, 16 };
        uint32_t tint = it->color ? it->color : (it->kind == ITEM_SETTING ? t->text_dim : t->accent);
        fb_fill_round(fb, clip, icon, it->kind == ITEM_APP ? 4 : 8, tint);
        fb_text(fb, clip, row.x + 36, row.y + (row.h - 16) / 2, it->name, t->text);
        const char *tag = it->kind == ITEM_APP ? "App" : it->kind == ITEM_FILE ? "File" : "Setting";
        fb_text(fb, clip, row.x + row.w - shell_text_width(tag) - 10, row.y + (row.h - 16) / 2, tag, t->text_dim);
    }
    if (!l->nhit) fb_text(fb, clip, r.x + 20, r.y + L_INPUT + 10, "No local results", t->text_dim);
}
