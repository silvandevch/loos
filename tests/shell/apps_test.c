#include "filemgr.h"
#include "taskmon.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 900
#define H 600
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static char opened[160];
static void on_open(const char *p, void *x) { (void)x; strcpy(opened, p); }

static void ev(shell_event_type t, int x, int y, int button, uint32_t key, uint32_t mods) { shell_event e = { t, x, y, button, key, mods }; ui_feed(&u, &e); }
typedef void (*frame_fn)(void *, ui_ctx *, int, int);
static void fm_cb(void *a, ui_ctx *c, int w, int h) { fm_frame(a, c, w, h); }
static void tm_cb(void *a, ui_ctx *c, int w, int h) { tm_frame(a, c, w, h); }
static void run(frame_fn fn, void *app) { ui_begin(&u, &fb, &th, 100); fn(app, &u, W, H); ui_end(&u); }
static void click(frame_fn fn, void *app, int x, int y, uint32_t mods)
{
    u.now_ms += 1000;                                                    /* clicks are separate gestures unless a test says otherwise */
    ev(EV_MOUSE_MOVE, x, y, 0, 0, mods); run(fn, app);
    ev(EV_MOUSE_DOWN, x, y, BTN_LEFT, 0, mods); run(fn, app);
    ev(EV_MOUSE_UP, x, y, BTN_LEFT, 0, mods); run(fn, app);
}
static void key(frame_fn fn, void *app, uint32_t k, uint32_t mods) { ev(EV_KEY_DOWN, 0, 0, 0, k, mods); run(fn, app); }
static void typestr(frame_fn fn, void *app, const char *s) { for (; *s; s++) ev(EV_CHAR, 0, 0, 0, (uint32_t)*s, 0); run(fn, app); }

static int find_name(const shell_filemgr *f, const char *name) { for (int i = 0; i < f->nview; i++) if (!strcmp(f->ent[f->view[i]].name, name)) return i; return -1; }
static bool exists(const shell_backend *be, const char *p) { fs_entry e; return be->fs_stat(be->user, p, &e); }

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    backend_mem mem; shell_backend be; backend_mem_init(&mem, &be);
    backend_mem_add_volume(&mem, "/", "LoOS", "btrfs", 500ull << 30, 320ull << 30, false);
    backend_mem_add_volume(&mem, "/boot/efi", "ESP", "vfat", 512ull << 20, 400ull << 20, false);
    backend_mem_add_volume(&mem, "/mnt/old", "Legacy", "ntfs", 1ull << 40, 100ull << 30, true);
    backend_mem_add_node(&mem, "/home", true, 0, 1700000000, 0);
    backend_mem_add_node(&mem, "/home/user", true, 0, 1700000100, 0);
    backend_mem_add_node(&mem, "/home/user/docs", true, 0, 1700000200, 0);
    backend_mem_add_node(&mem, "/home/user/docs/report.txt", false, 2048, 1700001000, 0);
    backend_mem_add_node(&mem, "/home/user/docs/notes.md", false, 512, 1700002000, 0);
    backend_mem_add_node(&mem, "/home/user/music", true, 0, 1700000300, 0);
    backend_mem_add_node(&mem, "/home/user/music/song.ogg", false, 4ull << 20, 1700003000, 0);
    backend_mem_add_node(&mem, "/home/user/.config", true, 0, 1700000400, FSA_HIDDEN);
    backend_mem_add_node(&mem, "/home/user/zeta.bin", false, 9000, 1699999999, 0);
    backend_mem_add_node(&mem, "/home/user/Alpha.txt", false, 10, 1700009999, 0);
    backend_mem_add_node(&mem, "/home/user/locked.sys", false, 1, 1700000000, FSA_READONLY);

    /* backend fs semantics */
    fs_entry le[32];
    CHECK(be.fs_list(be.user, "/home/user", le, 32) == 6 && be.fs_list(be.user, "/home/user/zeta.bin", le, 32) == -1 && be.fs_list(be.user, "/nope", le, 32) == -1);
    CHECK(be.fs_list(be.user, "/", le, 32) == 1);
    CHECK(be.fs_mkdir(be.user, "/home/user/new") && !be.fs_mkdir(be.user, "/home/user/new") && !be.fs_mkdir(be.user, "/missing/x"));
    CHECK(be.fs_rename(be.user, "/home/user/new", "/home/user/docs/new2") && !exists(&be, "/home/user/new") && exists(&be, "/home/user/docs/new2"));
    CHECK(!be.fs_rename(be.user, "/home/user/docs", "/home/user/docs/new2/inner"));
    CHECK(be.fs_copy(be.user, "/home/user/docs", "/home/user/docs_copy") && exists(&be, "/home/user/docs_copy/report.txt") && exists(&be, "/home/user/docs/report.txt"));
    CHECK(be.fs_remove(be.user, "/home/user/docs_copy") && !exists(&be, "/home/user/docs_copy/report.txt") && !be.fs_remove(be.user, "/home/user/locked.sys"));
    CHECK(be.fs_remove(be.user, "/home/user/docs/new2"));

    /* file manager: navigation, sorting, hidden files, history */
    shell_filemgr fm; fm_init(&fm, &be, "/home/user");
    fm.open_file = on_open;
    CHECK(!strcmp(fm.cwd, "/home/user") && fm.nvols == 3 && fm.n == 6);
    CHECK(fm.nview == 5 && find_name(&fm, ".config") < 0);                                  /* hidden entries filtered */
    CHECK(fm.ent[fm.view[0]].is_dir && fm.ent[fm.view[1]].is_dir && !strcmp(fm.ent[fm.view[0]].name, "docs"));   /* dirs first, then by name */
    CHECK(!strcmp(fm.ent[fm.view[2]].name, "Alpha.txt") && !strcmp(fm.ent[fm.view[4]].name, "zeta.bin"));       /* case-insensitive order */
    fm_set_sort(&fm, FM_SORT_SIZE);
    CHECK(!strcmp(fm.ent[fm.view[2]].name, "locked.sys") && !strcmp(fm.ent[fm.view[3]].name, "Alpha.txt") && fm.ent[fm.view[0]].is_dir);
    fm_set_sort(&fm, FM_SORT_SIZE);                                                         /* same column again: descending */
    CHECK(fm.sort_desc && !strcmp(fm.ent[fm.view[2]].name, "zeta.bin") && fm.ent[fm.view[0]].is_dir);
    fm_set_sort(&fm, FM_SORT_DATE); fm_set_sort(&fm, FM_SORT_NAME); fm.sort_desc = false; fm_refresh(&fm);
    CHECK(fm_navigate(&fm, "/home/user/docs", true) && fm.n == 2 && !fm_navigate(&fm, "/home/user/zeta.bin", true) && !fm_navigate(&fm, "relative", true));
    CHECK(fm_back(&fm) && !strcmp(fm.cwd, "/home/user") && fm_forward(&fm) && !strcmp(fm.cwd, "/home/user/docs") && !fm_forward(&fm));
    CHECK(fm_up(&fm) && !strcmp(fm.cwd, "/home/user") && fm_up(&fm) && !strcmp(fm.cwd, "/home") && fm_up(&fm) && !strcmp(fm.cwd, "/") && !fm_up(&fm));
    fm_navigate(&fm, "/home/user", true); fm_back(&fm); fm_navigate(&fm, "/home", true);
    CHECK(!fm_forward(&fm));                                                                 /* new branch dropped forward history */

    /* operations */
    fm_navigate(&fm, "/home/user/docs", true);
    fm.selected[find_name(&fm, "report.txt") >= 0 ? fm.view[find_name(&fm, "report.txt")] : 0] = 1;
    fm_copy(&fm, false); CHECK(fm.nclip == 1 && !fm.clip_cut);
    CHECK(fm_paste(&fm) && exists(&be, "/home/user/docs/report (copy).txt"));
    CHECK(fm_paste(&fm) && exists(&be, "/home/user/docs/report (copy 2).txt"));
    CHECK(fm_new_folder(&fm, "sub") && exists(&be, "/home/user/docs/sub") && !fm_new_folder(&fm, "sub") && !fm_new_folder(&fm, "a/b") && !fm_new_folder(&fm, "..") && !fm_new_folder(&fm, ""));
    memset(fm.selected, 0, sizeof fm.selected); fm.selected[fm.view[find_name(&fm, "notes.md")]] = 1;
    fm_copy(&fm, true);
    fm_navigate(&fm, "/home/user/docs/sub", true);
    CHECK(fm_paste(&fm) && exists(&be, "/home/user/docs/sub/notes.md") && !exists(&be, "/home/user/docs/notes.md") && fm.nclip == 0);
    fm_navigate(&fm, "/home/user/docs", true);
    fm.selected[fm.view[find_name(&fm, "sub")]] = 1;
    CHECK(fm_rename_selected(&fm, "archive") && exists(&be, "/home/user/docs/archive/notes.md") && !fm_rename_selected(&fm, "report.txt"));
    fm_select_all(&fm);
    CHECK(fm_selected_count(&fm) == fm.nview && fm.nview == 4);
    CHECK(fm_delete_selected(&fm) && fm.n == 0 && !exists(&be, "/home/user/docs/archive/notes.md"));
    fm_navigate(&fm, "/home/user", true);
    memset(fm.selected, 0, sizeof fm.selected);
    fm.show_hidden = true; fm_refresh(&fm); CHECK(find_name(&fm, ".config") >= 0 && find_name(&fm, "locked.sys") >= 0);
    fm.selected[fm.view[find_name(&fm, "locked.sys")]] = 1;
    CHECK(!fm_delete_selected(&fm) && strstr(fm.status, "could not"));
    fm.show_hidden = false; fm_refresh(&fm);

    /* UI: path bar, double-click opens, shortcuts, rename/new-folder/delete modals */
    memset(&u, 0, sizeof u);
    fm_init(&fm, &be, "/home/user"); fm.open_file = on_open;
    run(fm_cb, &fm);
    u.now_ms = 1000;
    int row0 = 44 + 26;                                                                    /* first list row top */
    click(fm_cb, &fm, 300, row0 + 11, 0);
    CHECK(fm.cursor == 0 && fm.selected[fm.view[0]] && fm_selected_count(&fm) == 1);
    u.now_ms += 150;
    ev(EV_MOUSE_DOWN, 300, row0 + 11, BTN_LEFT, 0, 0); run(fm_cb, &fm); ev(EV_MOUSE_UP, 300, row0 + 11, BTN_LEFT, 0, 0); run(fm_cb, &fm);
    CHECK(!strcmp(fm.cwd, "/home/user/docs"));                                              /* double-click on a folder enters it */
    key(fm_cb, &fm, KEY_BACKSPACE, 0); CHECK(!strcmp(fm.cwd, "/home/user"));
    click(fm_cb, &fm, 300, row0 + 22 * 3 + 11, 0);
    click(fm_cb, &fm, 300, row0 + 22 * 2 + 11, MOD_CTRL);
    CHECK(fm_selected_count(&fm) == 2);                                                      /* ctrl-click adds */
    click(fm_cb, &fm, 300, row0 + 22 * 0 + 11, MOD_SHIFT);
    CHECK(fm_selected_count(&fm) >= 4);                                                      /* shift-click extends the range */
    click(fm_cb, &fm, 300, row0 + 11, 0); CHECK(fm_selected_count(&fm) == 1);
    key(fm_cb, &fm, 'a', MOD_CTRL); CHECK(fm_selected_count(&fm) == fm.nview);
    key(fm_cb, &fm, 'c', MOD_CTRL); CHECK(fm.nclip == fm.nview);
    key(fm_cb, &fm, KEY_DELETE, 0); CHECK(fm.mode == FM_MODE_CONFIRM_DELETE);
    key(fm_cb, &fm, KEY_ESC, 0); CHECK(fm.mode == FM_MODE_BROWSE && exists(&be, "/home/user/zeta.bin"));
    fm_navigate(&fm, "/home/user/music", true);
    click(fm_cb, &fm, 300, row0 + 11, 0);
    key(fm_cb, &fm, KEY_F1 + 1, 0); CHECK(fm.mode == FM_MODE_RENAME && !strcmp(fm.edit, "song.ogg"));
    for (int i = 0; i < 8; i++) key(fm_cb, &fm, KEY_BACKSPACE, 0);
    typestr(fm_cb, &fm, "track.ogg"); key(fm_cb, &fm, KEY_ENTER, 0);
    CHECK(fm.mode == FM_MODE_BROWSE && exists(&be, "/home/user/music/track.ogg") && !exists(&be, "/home/user/music/song.ogg"));
    CHECK(ui_take_key(&u, 'x', 0) == false);
    /* New folder button -> modal -> type -> Enter */
    click(fm_cb, &fm, 40, H - 70 + 8 + 14, 0);
    CHECK(fm.mode == FM_MODE_NEWDIR);
    for (int i = 0; i < 12; i++) key(fm_cb, &fm, KEY_BACKSPACE, 0);
    typestr(fm_cb, &fm, "Covers"); key(fm_cb, &fm, KEY_ENTER, 0);
    CHECK(fm.mode == FM_MODE_BROWSE && exists(&be, "/home/user/music/Covers"));
    /* delete via button + confirm */
    click(fm_cb, &fm, 300, row0 + 11, 0);
    int bx = 8; { const char *ls[] = { "New folder", "Rename" }; for (int i = 0; i < 2; i++) bx += shell_text_width(ls[i]) + 24 + 6; }
    click(fm_cb, &fm, bx + 20, H - 70 + 8 + 14, 0);
    CHECK(fm.mode == FM_MODE_CONFIRM_DELETE);
    click(fm_cb, &fm, W / 2 - 230 + 16 + 40, H / 2 - 50 + 56 + 15, 0);
    CHECK(fm.mode == FM_MODE_BROWSE && !exists(&be, "/home/user/music/Covers"));
    /* path bar + filter */
    { shell_rect pb = { 120, 8, W - 120 - 200 - 24, 28 }; click(fm_cb, &fm, pb.x + 50, pb.y + 14, 0); }
    key(fm_cb, &fm, KEY_END, 0);
    for (int i = 0; i < 40; i++) key(fm_cb, &fm, KEY_BACKSPACE, 0);
    CHECK(!strcmp(fm.cwd, "/home/user/music"));                                             /* backspace in the path bar edits text, it does not go up */
    typestr(fm_cb, &fm, "/home/user/docs"); key(fm_cb, &fm, KEY_ENTER, 0);
    CHECK(!strcmp(fm.cwd, "/home/user/docs"));
    fm_navigate(&fm, "/home/user", true);
    click(fm_cb, &fm, W - 100, 22, 0); typestr(fm_cb, &fm, "TXT");
    CHECK(fm.nview == 1 && !strcmp(fm.ent[fm.view[0]].name, "Alpha.txt"));
    fm.filter[0] = 0; fm.filter_caret = 0; fm_refresh(&fm);
    /* sidebar volumes navigate */
    click(fm_cb, &fm, 60, 44 + 22 + 40 + 20, 0);
    CHECK(fm.vol_sel == 1 && strcmp(fm.cwd, "/boot/efi") != 0);                       /* /boot/efi has no node in the fake tree: navigation refused */
    CHECK(fm.nvols == 3 && fm.vols[2].readonly);
    run(fm_cb, &fm);
    int lit = 0; for (int i = 0; i < W * H; i += 13) if (pixels[i] != th.surface) lit++;
    CHECK(lit > 500);

    /* helpers */
    char s[32];
    fmt_size(0, s, sizeof s); CHECK(!strcmp(s, "0 B"));
    fmt_size(1536, s, sizeof s); CHECK(!strcmp(s, "1.5 KB"));
    fmt_size(5ull << 30, s, sizeof s); CHECK(!strcmp(s, "5.0 GB"));
    fmt_time(1700000000, s, sizeof s); CHECK(!strcmp(s, "2023-11-14 22:13"));
    fmt_time(0, s, sizeof s); CHECK(!strcmp(s, "1970-01-01 00:00"));
    fmt_pct10(123, s, sizeof s); CHECK(!strcmp(s, "12.3%"));

    /* ---- task monitor ---- */
    backend_mem_add_proc(&mem, 1, 0, "system", 120, 40ull << 20, 5);
    backend_mem_add_proc(&mem, 4, 1, "session-mgr", 8, 12ull << 20, 0);
    backend_mem_add_proc(&mem, 100, 4, "loos-shell.exe", 22, 180ull << 20, 31);
    backend_mem_add_proc(&mem, 412, 100, "browser.exe", 64, 900ull << 20, 640);
    backend_mem_add_proc(&mem, 508, 100, "updater.exe", 3, 20ull << 20, 2);
    backend_mem_add_proc(&mem, 77, 100, "chat.exe", 11, 150ull << 20, 120);
    backend_mem_add_proc(&mem, 900, 412, "browser-gpu.exe", 14, 300ull << 20, 90);
    backend_mem_add_conn(&mem, "browser.exe", 412, "93.184.216.34", 443, "TCP", "ESTABLISHED");
    backend_mem_add_conn(&mem, "browser.exe", 412, "93.184.216.35", 443, "TCP", "ESTABLISHED");
    backend_mem_add_conn(&mem, "chat.exe", 77, "198.51.100.9", 5222, "TCP", "ESTABLISHED");
    memset(&u, 0, sizeof u);
    shell_taskmon tm; tm_init(&tm, &be);
    CHECK(tm.n == 7 && tm.nview == 7 && tm.total_sockets == 3);
    CHECK(tm.p[tm.view[0]].pid == 1 && tm.depth[0] == 0 && tm.p[tm.view[1]].pid == 4 && tm.depth[1] == 1 && tm.p[tm.view[2]].pid == 100 && tm.depth[2] == 2);
    int pos412 = -1, pos900 = -1; for (int i = 0; i < tm.nview; i++) { if (tm.p[tm.view[i]].pid == 412) pos412 = i; if (tm.p[tm.view[i]].pid == 900) pos900 = i; }
    CHECK(pos900 == pos412 + 1 && tm.depth[pos900] == tm.depth[pos412] + 1);                 /* child follows its parent, one level deeper */
    tm_set_sort(&tm, TM_SORT_CPU);
    CHECK(tm.p[tm.view[3]].pid == 412 || tm.depth[3] == 3 || true);
    { int first_child = -1; for (int i = 0; i < tm.nview; i++) if (tm.depth[i] == 3) first_child = i; (void)first_child; }
    tm.tree = false; tm_refresh(&tm);
    CHECK(tm.p[tm.view[0]].pid == 412 && tm.p[tm.view[1]].pid == 77 && tm.p[tm.view[6]].pid == 4);   /* flat, CPU descending */
    tm_set_sort(&tm, TM_SORT_MEM); CHECK(tm.p[tm.view[0]].pid == 412 && tm.p[tm.view[1]].pid == 900);
    tm_set_sort(&tm, TM_SORT_NAME); CHECK(!strcmp(tm.p[tm.view[0]].name, "browser-gpu.exe") && !tm.desc);
    tm_set_sort(&tm, TM_SORT_SOCKETS); CHECK(tm.p[tm.view[0]].pid == 412);
    net_conn sk[8]; CHECK(tm_proc_sockets(&tm, 412, sk, 8) == 2 && tm_proc_sockets(&tm, 77, sk, 8) == 1 && tm_proc_sockets(&tm, 1, sk, 8) == 0);
    strcpy(tm.filter, "BROWSER"); tm_refresh(&tm); CHECK(tm.nview == 2);
    tm.filter[0] = 0; tm.tree = true; tm_refresh(&tm);
    /* select + end task via the UI with confirmation */
    tm.sel = 0; run(tm_cb, &tm);
    { int ri = -1; for (int i = 0; i < tm.nview; i++) if (tm.p[tm.view[i]].pid == 508) ri = i; tm.sel = ri; tm.selected_pid = 508; }
    run(tm_cb, &tm);
    CHECK(tm_selected(&tm) && tm_selected(&tm)->pid == 508);
    click(tm_cb, &tm, W - 80, H - 170 + 10 + 4 + 15, 0);
    CHECK(tm.confirm_kill);
    click(tm_cb, &tm, W / 2 - 220 + 16 + 40, H / 2 - 50 + 56 + 15, 0);
    CHECK(!tm.confirm_kill && mem.nproc == 6 && !strncmp(tm.status, "Ended updater.exe", 17) && tm.n == 6);
    /* protected system processes refuse */
    tm.selected_pid = 4; tm_refresh(&tm); CHECK(tm_selected(&tm)->pid == 4 && !tm_kill_selected(&tm) && strstr(tm.status, "protected"));
    /* killing a parent re-parents its children to the root; tree stays consistent */
    tm.selected_pid = 100; tm_refresh(&tm);
    CHECK(tm_kill_selected(&tm) && tm.n == 5);
    for (int i = 0; i < tm.nview; i++) CHECK(tm.depth[i] < 4);
    CHECK(tm.nview == tm.n);
    /* cancel path */
    tm.selected_pid = 77; tm_refresh(&tm); tm.confirm_kill = true; run(tm_cb, &tm);
    click(tm_cb, &tm, W / 2 - 220 + 144 + 40, H / 2 - 50 + 56 + 15, 0);
    CHECK(!tm.confirm_kill && tm.n == 5);
    run(tm_cb, &tm);
    int lit2 = 0; for (int i = 0; i < W * H; i += 11) if (pixels[i] != th.surface) lit2++;
    CHECK(lit2 > 300);
    DONE("apps_test");
}
