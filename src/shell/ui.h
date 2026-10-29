/* Immediate-mode widget toolkit for LoOS apps and the shell's own panels. Each frame the app calls ui_begin(),
 * feeds window events with ui_feed(), emits widgets (each returns its interaction result), then ui_end().
 * Rendering goes to a caller-supplied framebuffer (usually the window's client surface); there is no hidden
 * allocation and no retained widget tree, so state lives in the caller's own variables. */
#ifndef LOOS_SHELL_UI_H
#define LOOS_SHELL_UI_H
#include "shell.h"

#ifdef __cplusplus
extern "C" {
#endif

#define UI_KEYQ 128
typedef enum { A11Y_LABEL, A11Y_BUTTON, A11Y_TOGGLE, A11Y_RADIO, A11Y_TEXT, A11Y_LIST, A11Y_TAB, A11Y_SLIDER, A11Y_PROGRESS } ui_role;
#define UI_A11Y_MAX 192
typedef struct { ui_role role; char name[48]; char value[48]; shell_rect rect; bool focused, checked; uint32_t id; } ui_a11y_item;

typedef struct {
    shell_fb *fb;
    const shell_theme *theme;
    shell_rect clip;                       /* current clip (scroll areas narrow it) */
    int mx, my;                            /* pointer, client coordinates */
    bool down, pressed, released;          /* primary button state / edges this frame */
    int wheel;                             /* wheel delta accumulated this frame */
    struct { uint32_t key; uint32_t mods; bool is_char; } keys[UI_KEYQ]; int nkeys;
    uint32_t active;                       /* widget id currently pressed */
    uint32_t focus;                        /* widget id with keyboard focus */
    uint32_t hot;                          /* widget id under the pointer */
    uint32_t next_hot;
    bool consumed_click;                   /* a widget used this frame's press */
    int scale;                             /* percent, for paddings */
    uint32_t mods;                         /* modifier state stamped on the latest input event */
    uint32_t frame;
    uint32_t now_ms;                       /* caller sets each frame; drives double-click detection */
    uint32_t lc_id, lc_ms; int lc_row;     /* last list click */
    /* accessibility: every widget registers itself each frame; Tab walks `order`, screen readers read `a11y` */
    ui_a11y_item a11y[UI_A11Y_MAX]; int na11y;
    uint32_t order[UI_A11Y_MAX]; int norder;
    bool reduce_motion, high_contrast;     /* from the settings page; caret blink off / thicker focus ring */
    bool mask_next;                        /* the next text field is a password field (see ui_textfield_password) */
} ui_ctx;

uint32_t ui_id(const char *s);                                    /* FNV-1a of a widget label/name */
void ui_begin(ui_ctx *u, shell_fb *fb, const shell_theme *t, int scale_pct);
void ui_feed(ui_ctx *u, const shell_event *e);
void ui_end(ui_ctx *u);
bool ui_key(ui_ctx *u, uint32_t key, bool want_char, uint32_t *mods_out);   /* pop one queued key for the focused widget */

void ui_clear(ui_ctx *u, uint32_t color);
void ui_label(ui_ctx *u, int x, int y, const char *s, uint32_t color);
int  ui_paragraph(ui_ctx *u, int x, int y, int w, const char *s, uint32_t color);   /* word-wrapped; returns height used */
void ui_label_right(ui_ctx *u, int right_x, int y, const char *s, uint32_t color);
void ui_heading(ui_ctx *u, int x, int y, const char *s);          /* 2x size title */
void ui_separator(ui_ctx *u, int x, int y, int w);
void ui_panel(ui_ctx *u, shell_rect r, uint32_t color);
void ui_backdrop(ui_ctx *u, int w, int h);                         /* gradient + soft glows for full-screen pages */
bool ui_button(ui_ctx *u, shell_rect r, const char *label);        /* true on click (release over the pressed button) */
bool ui_button_accent(ui_ctx *u, shell_rect r, const char *label);
bool ui_toggle(ui_ctx *u, shell_rect r, const char *label, bool *value);   /* switch + label; true when flipped */
bool ui_radio(ui_ctx *u, shell_rect r, const char *label, int *group, int value);
uint32_t ui_textfield_id(const char *placeholder, shell_rect r);      /* lets a modal give its field focus */
bool ui_textfield(ui_ctx *u, shell_rect r, char *buf, size_t cap, int *caret, const char *placeholder);   /* true on Enter */
bool ui_textfield_password(ui_ctx *u, shell_rect r, char *buf, size_t cap, int *caret, const char *placeholder);   /* bullets instead of text */
int  ui_tabs(ui_ctx *u, shell_rect r, const char *const *labels, int n, int *current);   /* returns newly selected tab or -1 */
void ui_progress(ui_ctx *u, shell_rect r, int pct);
typedef void (*ui_row_fn)(ui_ctx *u, shell_rect row, int index, bool selected, void *user);
int  ui_list(ui_ctx *u, shell_rect r, const char *id, int n, int row_h, ui_row_fn draw, void *user, int *selected, int *scroll);   /* returns activated row (double-click/Enter) or -1 */
bool ui_slider(ui_ctx *u, shell_rect r, const char *id, int *value, int lo, int hi);
int  ui_a11y_count(const ui_ctx *u);
const ui_a11y_item *ui_a11y_get(const ui_ctx *u, int i);
size_t ui_a11y_dump(const ui_ctx *u, char *out, size_t cap);      /* "button: OK\ntoggle: Dark mode [on]\n..." for screen readers */
void ui_focus_next(ui_ctx *u, bool backwards);
void ui_focus(ui_ctx *u, uint32_t id);                            /* move keyboard focus to a widget id (0 = none) */
bool ui_focused(const ui_ctx *u, uint32_t id);                     /* true if that widget currently has keyboard focus */
void ui_clip_push(ui_ctx *u, shell_rect r, shell_rect *saved);
void ui_clip_pop(ui_ctx *u, shell_rect saved);
bool ui_hovered(const ui_ctx *u, shell_rect r);
bool ui_take_key(ui_ctx *u, uint32_t key, uint32_t need_mods);   /* remove a queued shortcut (lowercase key + required mods) */

/* accessors for apps written in Rust (crates/loos-shell/src/apps) */
shell_fb *ui_fb(ui_ctx *u);
const shell_theme *ui_theme_of(const ui_ctx *u);
void ui_pointer(const ui_ctx *u, int *x, int *y, bool *pressed, int *wheel);
void ui_blur(ui_ctx *u);                                         /* no widget has keyboard focus */
bool ui_next_input(ui_ctx *u, uint32_t *key, uint32_t *mods, bool *is_char);   /* pops keys and chars in arrival order */

/* formatting helpers shared by the apps */
void fmt_size(uint64_t bytes, char *out, size_t cap);          /* "812 B", "1.5 MB" */
void fmt_time(int64_t unix_seconds, char *out, size_t cap);    /* "2026-10-19 14:05" (UTC) */
void fmt_pct10(uint32_t x10, char *out, size_t cap);           /* 123 -> "12.3%" */

#ifdef __cplusplus
}
#endif
#endif
