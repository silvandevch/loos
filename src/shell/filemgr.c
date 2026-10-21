#include "filemgr.h"
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

void fm_join(const char *dir, const char *name, char *out, size_t cap)
{
    if (!strcmp(dir, "/")) snprintf(out, cap, "/%s", name); else snprintf(out, cap, "%s/%s", dir, name);
}

/* ---- listing, sorting ---- */
static const shell_filemgr *g_sort_ctx;
static int entry_cmp(const void *pa, const void *pb);

static void sort_view(shell_filemgr *f)
{
    /* insertion sort on the small view array: stable, no libc qsort needed in the shell core */
    for (int i = 1; i < f->nview; i++) {
        int v = f->view[i], j = i;
        for (; j > 0; j--) {
            const fs_entry *a = &f->ent[f->view[j - 1]], *b = &f->ent[v];
            int c;
            if (a->is_dir != b->is_dir) c = a->is_dir ? -1 : 1;                 /* directories always first */
            else {
                switch (f->sort) {
                case FM_SORT_SIZE: c = a->size < b->size ? -1 : a->size > b->size; break;
                case FM_SORT_DATE: c = a->mtime < b->mtime ? -1 : a->mtime > b->mtime; break;
                default: c = ci_cmp(a->name, b->name);
                }
                if (c == 0) c = ci_cmp(a->name, b->name);
                if (f->sort_desc) c = -c;
            }
            if (c <= 0) break;
            f->view[j] = f->view[j - 1];
        }
        f->view[j] = v;
    }
    (void)g_sort_ctx; (void)entry_cmp;
}
static int entry_cmp(const void *pa, const void *pb) { (void)pa; (void)pb; return 0; }

void fm_refresh(shell_filemgr *f)
{
    char keep[8][64]; int nkeep = 0;
    for (int i = 0; i < f->n && nkeep < 8; i++) if (f->selected[i]) strcpy(keep[nkeep++], f->ent[i].name);
    int n = f->be->fs_list(f->be->user, f->cwd, f->ent, FM_MAX_ENTRIES);
    f->n = n < 0 ? 0 : n;
    memset(f->selected, 0, sizeof f->selected);
    for (int i = 0; i < f->n; i++) for (int k = 0; k < nkeep; k++) if (!strcmp(f->ent[i].name, keep[k])) f->selected[i] = 1;
    f->nview = 0;
    for (int i = 0; i < f->n; i++) {
        if (!f->show_hidden && (f->ent[i].attrs & FSA_HIDDEN || f->ent[i].name[0] == '.')) continue;
        if (!ci_contains(f->ent[i].name, f->filter)) continue;
        f->view[f->nview++] = i;
    }
    sort_view(f);
    if (f->cursor >= f->nview) f->cursor = f->nview ? f->nview - 1 : 0;
    f->nvols = f->be->fs_volumes(f->be->user, f->vols, 4);
    uint64_t tot = 0; int sel = 0;
    for (int i = 0; i < f->n; i++) if (f->selected[i]) { sel++; tot += f->ent[i].size; }
    if (sel) { char sz[24]; fmt_size(tot, sz, sizeof sz); snprintf(f->status, sizeof f->status, "%d of %d selected (%s)", sel, f->nview, sz); }
    else snprintf(f->status, sizeof f->status, "%d items", f->nview);
}

static void set_status(shell_filemgr *f, const char *m) { strncpy(f->status, m, sizeof f->status - 1); f->status[sizeof f->status - 1] = 0; }

bool fm_navigate(shell_filemgr *f, const char *path, bool record)
{
    char p[FM_PATH];
    size_t n = strlen(path);
    if (n >= sizeof p) return false;
    strcpy(p, path);
    while (n > 1 && p[n - 1] == '/') p[--n] = 0;
    fs_entry e;
    if (p[0] != '/' || !f->be->fs_stat(f->be->user, p, &e) || !e.is_dir) { set_status(f, "Not a folder"); return false; }
    if (record) {
        if (f->hist_n && f->hist_pos + 1 < f->hist_n) f->hist_n = f->hist_pos + 1;      /* branching drops the forward history */
        if (f->hist_n == FM_HISTORY) { memmove(f->hist[0], f->hist[1], (FM_HISTORY - 1) * FM_PATH); f->hist_n--; }
        strcpy(f->hist[f->hist_n++], p); f->hist_pos = f->hist_n - 1;
    }
    strcpy(f->cwd, p); strcpy(f->addr, p); f->addr_caret = (int)strlen(p);
    f->cursor = f->scroll = 0;
    memset(f->selected, 0, sizeof f->selected);
    f->filter[0] = 0; f->filter_caret = 0;
    f->mode = FM_MODE_BROWSE;
    fm_refresh(f);
    return true;
}
bool fm_back(shell_filemgr *f) { if (f->hist_pos <= 0) return false; f->hist_pos--; return fm_navigate(f, f->hist[f->hist_pos], false); }
bool fm_forward(shell_filemgr *f) { if (f->hist_pos + 1 >= f->hist_n) return false; f->hist_pos++; return fm_navigate(f, f->hist[f->hist_pos], false); }
bool fm_up(shell_filemgr *f)
{
    if (!strcmp(f->cwd, "/")) return false;
    char p[FM_PATH]; strcpy(p, f->cwd);
    char *s = strrchr(p, '/');
    if (s == p) p[1] = 0; else *s = 0;
    return fm_navigate(f, p, true);
}

void fm_init(shell_filemgr *f, const shell_backend *be, const char *start)
{
    memset(f, 0, sizeof *f);
    f->be = be;
    if (!fm_navigate(f, start && *start ? start : "/", true)) fm_navigate(f, "/", true);
}

void fm_set_sort(shell_filemgr *f, fm_sort s)
{
    if (f->sort == s) f->sort_desc = !f->sort_desc; else { f->sort = s; f->sort_desc = false; }
    fm_refresh(f);
}
int fm_selected_count(const shell_filemgr *f) { int n = 0; for (int i = 0; i < f->n; i++) n += f->selected[i]; return n; }
void fm_select_all(shell_filemgr *f) { memset(f->selected, 0, sizeof f->selected); for (int i = 0; i < f->nview; i++) f->selected[f->view[i]] = 1; fm_refresh(f); }

static bool valid_name(const char *n) { return *n && strcmp(n, ".") && strcmp(n, "..") && !strchr(n, '/') && strlen(n) < 60; }

bool fm_new_folder(shell_filemgr *f, const char *name)
{
    char p[FM_PATH];
    if (!valid_name(name)) { set_status(f, "Invalid name"); return false; }
    fm_join(f->cwd, name, p, sizeof p);
    if (!f->be->fs_mkdir(f->be->user, p)) { set_status(f, "Could not create folder (name in use?)"); return false; }
    fm_refresh(f);
    return true;
}

bool fm_rename_selected(shell_filemgr *f, const char *name)
{
    int sel = -1;
    for (int i = 0; i < f->n; i++) if (f->selected[i]) { if (sel >= 0) { set_status(f, "Select one item to rename"); return false; } sel = i; }
    if (sel < 0 && f->cursor < f->nview) sel = f->view[f->cursor];
    if (sel < 0) return false;
    if (!valid_name(name)) { set_status(f, "Invalid name"); return false; }
    char from[FM_PATH], to[FM_PATH];
    fm_join(f->cwd, f->ent[sel].name, from, sizeof from); fm_join(f->cwd, name, to, sizeof to);
    if (!f->be->fs_rename(f->be->user, from, to)) { set_status(f, "Could not rename (name in use?)"); return false; }
    memset(f->selected, 0, sizeof f->selected);
    fm_refresh(f);
    for (int i = 0; i < f->n; i++) if (!strcmp(f->ent[i].name, name)) f->selected[i] = 1;
    fm_refresh(f);
    return true;
}

bool fm_delete_selected(shell_filemgr *f)
{
    int done = 0, failed = 0;
    for (int i = 0; i < f->n; i++) {
        if (!f->selected[i]) continue;
        char p[FM_PATH]; fm_join(f->cwd, f->ent[i].name, p, sizeof p);
        if (f->be->fs_remove(f->be->user, p)) done++; else failed++;
    }
    memset(f->selected, 0, sizeof f->selected);
    fm_refresh(f);
    if (failed) { char m[96]; snprintf(m, sizeof m, "Deleted %d, %d could not be deleted", done, failed); set_status(f, m); }
    return done > 0 && !failed;
}

void fm_copy(shell_filemgr *f, bool cut)
{
    f->nclip = 0; f->clip_cut = cut;
    for (int i = 0; i < f->n && f->nclip < 16; i++) if (f->selected[i]) fm_join(f->cwd, f->ent[i].name, f->clip[f->nclip++], FM_PATH);
    if (!f->nclip && f->cursor < f->nview) fm_join(f->cwd, f->ent[f->view[f->cursor]].name, f->clip[f->nclip++], FM_PATH);
    char m[64]; snprintf(m, sizeof m, "%d item(s) %s", f->nclip, cut ? "cut" : "copied"); set_status(f, m);
}

static void unique_dest(shell_filemgr *f, const char *name, char *out, size_t cap)
{
    fs_entry e;
    fm_join(f->cwd, name, out, cap);
    if (!f->be->fs_stat(f->be->user, out, &e)) return;
    const char *dot = strrchr(name, '.');
    size_t stem = dot && dot != name ? (size_t)(dot - name) : strlen(name);
    for (int k = 1; k < 1000; k++) {
        char nm[96];
        if (k == 1) snprintf(nm, sizeof nm, "%.*s (copy)%s", (int)stem, name, name + stem);
        else snprintf(nm, sizeof nm, "%.*s (copy %d)%s", (int)stem, name, k, name + stem);
        fm_join(f->cwd, nm, out, cap);
        if (!f->be->fs_stat(f->be->user, out, &e)) return;
    }
}

bool fm_paste(shell_filemgr *f)
{
    int ok = 0;
    for (int i = 0; i < f->nclip; i++) {
        const char *name = strrchr(f->clip[i], '/') + 1;
        char dest[FM_PATH];
        if (f->clip_cut) {
            fm_join(f->cwd, name, dest, sizeof dest);
            char srcdir[FM_PATH]; strcpy(srcdir, f->clip[i]); *strrchr(srcdir, '/') = 0; if (!*srcdir) strcpy(srcdir, "/");
            if (!strcmp(srcdir, f->cwd)) continue;                              /* cut + paste in place is a no-op */
            unique_dest(f, name, dest, sizeof dest);
            if (f->be->fs_rename(f->be->user, f->clip[i], dest)) ok++;
        } else {
            unique_dest(f, name, dest, sizeof dest);
            if (f->be->fs_copy(f->be->user, f->clip[i], dest)) ok++;
        }
    }
    if (f->clip_cut && ok) f->nclip = 0;
    fm_refresh(f);
    return ok > 0;
}

/* ---- UI ---- */
static void row_entry(ui_ctx *u, shell_rect row, int i, bool cursor, void *user)
{
    shell_filemgr *f = user;
    const fs_entry *e = &f->ent[f->view[i]];
    bool sel = f->selected[f->view[i]];
    const shell_theme *t = u->theme;
    if (sel && !cursor) fb_fill(u->fb, u->clip, row, color_mix(t->surface, t->accent, 60));
    uint32_t tc = cursor ? t->accent_text : t->text;
    shell_rect icon = { row.x + 8, row.y + 4, 14, 14 };
    fb_fill_round(u->fb, u->clip, icon, e->is_dir ? 3 : 2, e->is_dir ? 0xFFF59E0Bu : t->text_dim);
    char name[80]; strncpy(name, e->name, sizeof name - 1); name[sizeof name - 1] = 0;
    shell_ellipsize(name, sizeof name, e->name, row.w - 330);
    ui_label(u, row.x + 30, row.y + 3, name, (e->attrs & FSA_HIDDEN) || e->name[0] == '.' ? t->text_dim : tc);
    char sz[24] = "", tm[24];
    if (!e->is_dir) fmt_size(e->size, sz, sizeof sz);
    ui_label_right(u, row.x + row.w - 176, row.y + 3, sz, cursor ? t->accent_text : t->text_dim);
    fmt_time(e->mtime, tm, sizeof tm);
    ui_label_right(u, row.x + row.w - 10, row.y + 3, tm, cursor ? t->accent_text : t->text_dim);
}
static void row_vol(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_filemgr *f = user;
    const fs_volume *v = &f->vols[i];
    char sz[24], line[48];
    fmt_size(v->avail, sz, sizeof sz);
    ui_label(u, row.x + 8, row.y + 2, v->label[0] ? v->label : v->mount, sel ? u->theme->accent_text : u->theme->text);
    snprintf(line, sizeof line, "%s %s%s", v->fstype, sz, v->readonly ? " ro" : "");
    ui_label(u, row.x + 8, row.y + 18, line, u->theme->text_dim);
}

static void open_cursor(shell_filemgr *f)
{
    if (f->cursor >= f->nview) return;
    const fs_entry *e = &f->ent[f->view[f->cursor]];
    char p[FM_PATH]; fm_join(f->cwd, e->name, p, sizeof p);
    if (e->is_dir) fm_navigate(f, p, true); else if (f->open_file) f->open_file(p, f->user);
}

void fm_frame(shell_filemgr *f, ui_ctx *u, int w, int h)
{
    const shell_theme *t = u->theme;
    ui_clear(u, t->surface);
    bool browsing = f->mode == FM_MODE_BROWSE;
    /* while a modal is up the widgets underneath draw but receive no input: park the input state, restore it for the modal */
    struct { uint32_t key, mods; bool is_char; } parked[UI_KEYQ]; int nparked = 0; bool p_pressed = false, p_released = false; int p_wheel = 0;
    if (!browsing) {
        nparked = u->nkeys;
        for (int i = 0; i < nparked; i++) { parked[i].key = u->keys[i].key; parked[i].mods = u->keys[i].mods; parked[i].is_char = u->keys[i].is_char; }
        p_pressed = u->pressed; p_released = u->released; p_wheel = u->wheel;
        u->nkeys = 0; u->pressed = u->released = false; u->wheel = 0;
    }
    /* toolbar */
    if (ui_button(u, (shell_rect){ 8, 8, 32, 28 }, "<")) fm_back(f);
    if (ui_button(u, (shell_rect){ 44, 8, 32, 28 }, ">")) fm_forward(f);
    if (ui_button(u, (shell_rect){ 80, 8, 32, 28 }, "^")) fm_up(f);
    int fw = 200;
    if (ui_textfield(u, (shell_rect){ 120, 8, w - 120 - fw - 24, 28 }, f->addr, sizeof f->addr, &f->addr_caret, "Path")) {
        if (!fm_navigate(f, f->addr, true)) strcpy(f->addr, f->cwd);
    }
    char before[40]; strcpy(before, f->filter);
    ui_textfield(u, (shell_rect){ w - fw - 8, 8, fw, 28 }, f->filter, sizeof f->filter, &f->filter_caret, "Filter");
    if (strcmp(before, f->filter)) fm_refresh(f);
    /* sidebar */
    int side = 170, top = 44, bottom = h - 70;
    ui_label(u, 12, top + 2, "Places", t->text_dim);
    int vs = f->vol_sel;
    int act = ui_list(u, (shell_rect){ 8, top + 22, side - 8, 4 * 40 }, "volumes", f->nvols, 40, row_vol, f, &f->vol_sel, &f->vol_scroll);
    if (f->vol_sel != vs || act >= 0) { if (f->vol_sel < f->nvols) fm_navigate(f, f->vols[f->vol_sel].mount, true); }
    /* list header */
    int lx = side + 8, lw = w - lx - 8;
    shell_rect hdr = { lx, top, lw, 24 };
    ui_label(u, lx + 30, top + 4, f->sort == FM_SORT_NAME ? (f->sort_desc ? "Name v" : "Name ^") : "Name", t->text_dim);
    ui_label_right(u, lx + lw - 176, top + 4, f->sort == FM_SORT_SIZE ? (f->sort_desc ? "Size v" : "Size ^") : "Size", t->text_dim);
    ui_label_right(u, lx + lw - 10, top + 4, f->sort == FM_SORT_DATE ? (f->sort_desc ? "Modified v" : "Modified ^") : "Modified", t->text_dim);
    if (u->pressed && ui_hovered(u, hdr)) {
        if (u->mx > lx + lw - 130) fm_set_sort(f, FM_SORT_DATE); else if (u->mx > lx + lw - 260) fm_set_sort(f, FM_SORT_SIZE); else fm_set_sort(f, FM_SORT_NAME);
    }
    fb_hline(u->fb, u->clip, lx, top + 24, lw, t->border);
    /* shortcuts (consumed before the list sees the keys) */
    if (browsing && (u->focus == ui_id("entries") || u->focus == 0 || u->focus == ui_id("volumes"))) {     /* never while a text field is being edited */
        if (ui_take_key(u, 'c', MOD_CTRL)) fm_copy(f, false);
        if (ui_take_key(u, 'x', MOD_CTRL)) fm_copy(f, true);
        if (ui_take_key(u, 'v', MOD_CTRL)) fm_paste(f);
        if (ui_take_key(u, 'a', MOD_CTRL)) fm_select_all(f);
        if (ui_take_key(u, KEY_DELETE, 0) && fm_selected_count(f)) f->mode = FM_MODE_CONFIRM_DELETE;
        if (ui_take_key(u, KEY_F1 + 1, 0)) { f->mode = FM_MODE_RENAME; f->edit[0] = 0; if (f->cursor < f->nview) strcpy(f->edit, f->ent[f->view[f->cursor]].name); f->edit_caret = (int)strlen(f->edit); }
        if (ui_take_key(u, KEY_F1 + 4, 0)) fm_refresh(f);
        if (ui_take_key(u, KEY_BACKSPACE, 0)) fm_up(f);
        if (ui_take_key(u, 'h', MOD_CTRL)) { f->show_hidden = !f->show_hidden; fm_refresh(f); }
    }
    /* entries */
    shell_rect lr = { lx, top + 26, lw, bottom - top - 26 };
    int cursor_before = f->cursor;
    int opened = ui_list(u, lr, "entries", f->nview, 22, row_entry, f, &f->cursor, &f->scroll);
    if (u->pressed && ui_hovered(u, lr) && f->nview && f->cursor < f->nview) {            /* selection model: click, ctrl-click, shift-click */
        int idx = f->view[f->cursor];
        if (u->mods & MOD_CTRL) f->selected[idx] ^= 1;
        else if (u->mods & MOD_SHIFT) { int a = cursor_before < f->cursor ? cursor_before : f->cursor, b = cursor_before < f->cursor ? f->cursor : cursor_before; for (int i = a; i <= b && i < f->nview; i++) f->selected[f->view[i]] = 1; }
        else { memset(f->selected, 0, sizeof f->selected); f->selected[idx] = 1; }
        fm_refresh(f);
    } else if (f->cursor != cursor_before && browsing && f->nview) {                    /* keyboard moved the cursor */
        memset(f->selected, 0, sizeof f->selected); f->selected[f->view[f->cursor]] = 1; fm_refresh(f);
    }
    if (opened >= 0 && browsing) open_cursor(f);
    if (!f->nview) ui_label(u, lx + 14, top + 40, f->filter[0] ? "No matches." : "This folder is empty.", t->text_dim);
    /* action bar */
    int by = bottom + 8;
    static const char *const labels[] = { "New folder", "Rename", "Delete", "Copy", "Cut", "Paste" };
    int bx = 8;
    for (int i = 0; i < 6; i++) {
        int bw = shell_text_width(labels[i]) + 24;
        if (browsing && ui_button(u, (shell_rect){ bx, by, bw, 28 }, labels[i])) {
            switch (i) {
            case 0: f->mode = FM_MODE_NEWDIR; strcpy(f->edit, "New folder"); f->edit_caret = (int)strlen(f->edit); break;
            case 1: if (f->nview) { f->mode = FM_MODE_RENAME; strcpy(f->edit, f->ent[f->view[f->cursor]].name); f->edit_caret = (int)strlen(f->edit); } break;
            case 2: if (fm_selected_count(f)) f->mode = FM_MODE_CONFIRM_DELETE; break;
            case 3: fm_copy(f, false); break;
            case 4: fm_copy(f, true); break;
            default: fm_paste(f);
            }
        } else if (!browsing) fb_fill_round(u->fb, u->clip, (shell_rect){ bx, by, bw, 28 }, 6, t->surface_hi);
        bx += bw + 6;
    }
    ui_label(u, 12, by + 36, f->status, t->text_dim);
    ui_label_right(u, w - 10, by + 36, f->show_hidden ? "hidden files shown" : "", t->text_dim);
    /* modal strip: rename / new folder / delete confirmation */
    if (!browsing) {
        for (int i = 0; i < nparked; i++) { u->keys[i].key = parked[i].key; u->keys[i].mods = parked[i].mods; u->keys[i].is_char = parked[i].is_char; }
        u->nkeys = nparked; u->pressed = p_pressed; u->released = p_released; u->wheel = p_wheel;
        shell_rect m = { w / 2 - 230, h / 2 - 50, 460, 100 };
        fb_fill_round(u->fb, u->clip, (shell_rect){ m.x - 2, m.y - 2, m.w + 4, m.h + 4 }, 10, t->border);
        fb_fill_round(u->fb, u->clip, m, 8, t->surface_hi);
        if (f->mode == FM_MODE_CONFIRM_DELETE) {
            char msg[80]; snprintf(msg, sizeof msg, "Delete %d item(s)? This cannot be undone.", fm_selected_count(f));
            ui_label(u, m.x + 16, m.y + 16, msg, t->text);
            if (ui_button_accent(u, (shell_rect){ m.x + 16, m.y + 56, 120, 30 }, "Delete")) { fm_delete_selected(f); f->mode = FM_MODE_BROWSE; }
            if (ui_button(u, (shell_rect){ m.x + 144, m.y + 56, 120, 30 }, "Cancel") || ui_take_key(u, KEY_ESC, 0)) f->mode = FM_MODE_BROWSE;
        } else {
            ui_label(u, m.x + 16, m.y + 12, f->mode == FM_MODE_RENAME ? "Rename to:" : "New folder name:", t->text);
            u->focus = ui_textfield_id("name", (shell_rect){ m.x + 16, m.y + 34, m.w - 32, 28 });
            if (ui_textfield(u, (shell_rect){ m.x + 16, m.y + 34, m.w - 32, 28 }, f->edit, sizeof f->edit, &f->edit_caret, "name")) {
                bool ok = f->mode == FM_MODE_RENAME ? fm_rename_selected(f, f->edit) : fm_new_folder(f, f->edit);
                if (ok) f->mode = FM_MODE_BROWSE;
            }
            if (ui_take_key(u, KEY_ESC, 0)) f->mode = FM_MODE_BROWSE;
            ui_label(u, m.x + 16, m.y + 72, "Enter to confirm, Esc to cancel", t->text_dim);
        }
    }
    (void)open_cursor;
}
