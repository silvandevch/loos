/* File manager: native btrfs + mounted volumes through the Fs backend; no cloud integrations. */
#ifndef LOOS_SHELL_FILEMGR_H
#define LOOS_SHELL_FILEMGR_H
#include "ui.h"
#include "backend.h"

#ifdef __cplusplus
extern "C" {
#endif

#define FM_MAX_ENTRIES 256
#define FM_HISTORY 32
#define FM_PATH 160
typedef enum { FM_SORT_NAME, FM_SORT_SIZE, FM_SORT_DATE } fm_sort;
typedef enum { FM_MODE_BROWSE, FM_MODE_RENAME, FM_MODE_NEWDIR, FM_MODE_CONFIRM_DELETE } fm_mode;

typedef struct {
    const shell_backend *be;
    char cwd[FM_PATH];
    fs_entry ent[FM_MAX_ENTRIES]; int n;
    uint8_t selected[FM_MAX_ENTRIES];            /* multi-select flags */
    int cursor, scroll, vol_sel, vol_scroll;
    fs_volume vols[4]; int nvols;
    char addr[FM_PATH]; int addr_caret;          /* address bar text */
    char filter[40]; int filter_caret;
    char edit[64]; int edit_caret;               /* rename / new folder name */
    fm_mode mode;
    bool show_hidden;
    fm_sort sort; bool sort_desc;
    char hist[FM_HISTORY][FM_PATH]; int hist_n, hist_pos;
    char clip[16][FM_PATH]; int nclip; bool clip_cut;     /* copy/cut buffer */
    char status[96];
    int view[FM_MAX_ENTRIES]; int nview;         /* indices into ent[] after filter/sort */
    void (*open_file)(const char *path, void *user);     /* double-click on a file */
    void *user;
} shell_filemgr;

void fm_init(shell_filemgr *f, const shell_backend *be, const char *start_path);
bool fm_navigate(shell_filemgr *f, const char *path, bool record);    /* false if not a directory */
void fm_refresh(shell_filemgr *f);
bool fm_back(shell_filemgr *f);
bool fm_forward(shell_filemgr *f);
bool fm_up(shell_filemgr *f);
void fm_set_sort(shell_filemgr *f, fm_sort s);
void fm_select_all(shell_filemgr *f);
void fm_copy(shell_filemgr *f, bool cut);
bool fm_paste(shell_filemgr *f);                                      /* name collisions get " (copy)", " (copy 2)", ... */
bool fm_delete_selected(shell_filemgr *f);
bool fm_new_folder(shell_filemgr *f, const char *name);
bool fm_rename_selected(shell_filemgr *f, const char *name);
int  fm_selected_count(const shell_filemgr *f);
void fm_join(const char *dir, const char *name, char *out, size_t cap);
void fm_frame(shell_filemgr *f, ui_ctx *ui, int w, int h);

#ifdef __cplusplus
}
#endif
#endif
