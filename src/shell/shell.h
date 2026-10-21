/* LoOS shell core: software compositor, window model and drawing primitives. Pure C11, no allocation, no OS
 * calls: the caller owns the framebuffer and feeds input events, so the same code runs in the real compositor
 * process and in host tests. Colors are 0xAARRGGBB. Palette comes from branding/brand.h. */
#ifndef LOOS_SHELL_H
#define LOOS_SHELL_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "font.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ---------- geometry ---------- */
typedef struct { int x, y, w, h; } shell_rect;
static inline bool rect_empty(shell_rect r) { return r.w <= 0 || r.h <= 0; }
static inline bool rect_contains(shell_rect r, int px, int py) { return px >= r.x && py >= r.y && px < r.x + r.w && py < r.y + r.h; }
shell_rect rect_intersect(shell_rect a, shell_rect b);
shell_rect rect_union(shell_rect a, shell_rect b);
bool rect_overlaps(shell_rect a, shell_rect b);

/* ---------- palette (dark default; accent-driven) ---------- */
typedef struct {
    uint32_t desktop, surface, surface_hi, border, text, text_dim, accent, accent_text, danger, title_inactive;
} shell_theme;
void shell_theme_dark(shell_theme *t, uint32_t accent);
void shell_theme_light(shell_theme *t, uint32_t accent);
void shell_theme_high_contrast(shell_theme *t);

/* ---------- framebuffer + drawing ---------- */
typedef struct { uint32_t *px; int w, h, stride; int scale; } shell_fb;   /* w/h/stride in device px; scale = UI scale in percent (0 = 100) */
int fb_lw(const shell_fb *fb);                                       /* logical size = device size * 100 / scale */
int fb_lh(const shell_fb *fb);
uint32_t color_blend(uint32_t dst, uint32_t src);                  /* src-over, src carries alpha */
uint32_t color_mix(uint32_t a, uint32_t b, int t256);              /* 0 -> a, 256 -> b */
void fb_fill(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t c);
void fb_fill_round(shell_fb *fb, shell_rect clip, shell_rect r, int radius, uint32_t c);
void fb_frame(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t c);          /* 1px outline */
void fb_hline(shell_fb *fb, shell_rect clip, int x, int y, int len, uint32_t c);
void fb_vline(shell_fb *fb, shell_rect clip, int x, int y, int len, uint32_t c);
void fb_blit(shell_fb *fb, shell_rect clip, int dx, int dy, const uint32_t *src, int sw, int sh, int sstride, shell_rect srcrect);
int  fb_text(shell_fb *fb, shell_rect clip, int x, int y, const char *s, uint32_t c);   /* body font; returns advance in logical px */
int  fb_text_ex(shell_fb *fb, shell_rect clip, int x, int y, const char *s, uint32_t c, shell_font f);   /* y = top of the em box */
int  shell_text_width(const char *s);
int  shell_text_width_ex(const char *s, shell_font f);
int  shell_text_height(void);                                       /* line height of the body font, 16 logical px */
void shell_ellipsize(char *out, size_t cap, const char *s, int max_px);   /* copy of s shortened with an ellipsis to fit max_px */
int  shell_text_fit(const char *s, int max_px);                     /* bytes of s that fit in max_px */
int  shell_text_index_at(const char *s, int x);                     /* caret byte offset nearest to x px from the text origin */
void fb_blit_rounded(shell_fb *fb, shell_rect clip, int dx, int dy, const uint32_t *src, int sw, int sh, int sstride, shell_rect srcrect,
                     shell_rect round, int radius);     /* device space; masks the corners of `round` */
void fb_stroke_round(shell_fb *fb, shell_rect clip, shell_rect r, int radius, int thick, uint32_t c);
void fb_fill_circle(shell_fb *fb, shell_rect clip, int cx, int cy, int radius, uint32_t c);
void fb_shadow(shell_fb *fb, shell_rect clip, shell_rect r, int radius, int blur, int dy, uint32_t c);   /* soft shadow, never under r */
void fb_radial_glow(shell_fb *fb, shell_rect clip, int cx, int cy, int radius, uint32_t color, int strength);
void fb_gradient_v(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t top, uint32_t bottom);

/* ---------- damage tracking ---------- */
#define SHELL_DAMAGE_MAX 16
typedef struct { shell_rect r[SHELL_DAMAGE_MAX]; int n; } shell_damage;
void damage_clear(shell_damage *d);
void damage_add(shell_damage *d, shell_rect r);       /* merges overlaps; collapses to one bounding rect when full */
shell_rect damage_bounds(const shell_damage *d);

/* ---------- events ---------- */
typedef enum {
    EV_NONE, EV_MOUSE_MOVE, EV_MOUSE_DOWN, EV_MOUSE_UP, EV_MOUSE_WHEEL, EV_KEY_DOWN, EV_KEY_UP, EV_CHAR,
    EV_FOCUS, EV_BLUR, EV_CLOSE_REQUEST, EV_RESIZE, EV_MOVE, EV_ENTER, EV_LEAVE, EV_DPI_CHANGED /* x = new dpi */
} shell_event_type;
enum { MOD_SHIFT = 1, MOD_CTRL = 2, MOD_ALT = 4, MOD_SUPER = 8 };
enum { BTN_LEFT = 1, BTN_RIGHT = 2, BTN_MIDDLE = 4 };
typedef struct {
    shell_event_type type;
    int x, y;          /* client-relative for mouse events; new size for EV_RESIZE; position for EV_MOVE */
    int button;        /* BTN_* or wheel delta */
    uint32_t key;      /* key code (KEY_* below) or Unicode scalar for EV_CHAR */
    uint32_t mods;
} shell_event;
enum { KEY_ESC = 27, KEY_TAB = 9, KEY_ENTER = 13, KEY_BACKSPACE = 8, KEY_DELETE = 127,
       KEY_UP = 0x100, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_HOME, KEY_END, KEY_PGUP, KEY_PGDN,
       KEY_F1 = 0x120, KEY_F4 = 0x123, KEY_F12 = 0x12b, KEY_SUPER = 0x140, KEY_SPACE = 32 };

/* ---------- windows + compositor ---------- */
struct shell_comp;
#define SHELL_MAX_WINDOWS 128
#define SHELL_TITLE_MAX 96
#define SHELL_EVQ 64
#define SHELL_DESKTOPS 8

enum {
    WF_VISIBLE = 1, WF_MINIMIZED = 2, WF_MAXIMIZED = 4, WF_NORESIZE = 8, WF_NOCHROME = 16, WF_TOPMOST = 32,
    WF_NOFOCUS = 64, WF_STICKY = 128 /* shown on every virtual desktop */, WF_SNAPPED = 256, WF_NOTASKBAR = 512
};
typedef enum {
    Z_NONE, Z_CLIENT, Z_TITLE, Z_CLOSE, Z_MAXIMIZE, Z_MINIMIZE,
    Z_EDGE_L, Z_EDGE_R, Z_EDGE_T, Z_EDGE_B, Z_CORNER_TL, Z_CORNER_TR, Z_CORNER_BL, Z_CORNER_BR
} shell_zone;

typedef struct {
    int id;                          /* 0 = free slot */
    shell_rect rect;                 /* outer rectangle incl. chrome */
    shell_rect restore;              /* geometry before maximize/snap */
    char title[SHELL_TITLE_MAX];
    uint32_t flags;
    uint32_t owner;                  /* pid of the owning process */
    int desktop;                     /* virtual desktop index */
    int min_w, min_h;
    uint32_t *client;                /* optional client surface (ARGB) */
    int client_w, client_h, client_stride;
    shell_event q[SHELL_EVQ];
    int q_head, q_count;
    uint32_t dpi;                    /* per-window effective DPI (96 = 100%) */
} shell_window;

typedef struct {
    int title_h, border, button_w, edge_grab, corner_r;
} shell_metrics;

typedef struct shell_comp {
    int screen_w, screen_h;
    shell_window win[SHELL_MAX_WINDOWS];
    int order[SHELL_MAX_WINDOWS];    /* z-order, back to front (window ids) */
    int nwin, next_id;
    int focus;                       /* window id with keyboard focus, 0 = none */
    int cur_desktop;
    shell_theme theme;
    shell_metrics m;
    shell_rect work_area;            /* screen minus reserved bars (taskbar) */
    int bottom_reserve;              /* logical px reserved at the bottom (taskbar); work_area follows scale changes */
    shell_damage damage;
    int mouse_x, mouse_y;
    uint32_t buttons;
    int hover;                       /* window under the cursor */
    /* interaction */
    int grab_win; shell_zone grab_zone; int grab_dx, grab_dy; shell_rect grab_rect;
    int capture;                     /* window receiving mouse events during a client press */
    void (*on_change)(struct shell_comp *, int win, void *user);   /* window set/title/state change, for the taskbar */
    void *on_change_user;
    struct { void (*fn)(struct shell_comp *, shell_fb *, shell_rect clip, void *user); void *user; } overlay[4];   /* drawn above windows, below the cursor, in order */
    uint32_t scale_pct;              /* global UI scale: 100, 125, 150, 200 */
    bool reduce_motion, high_contrast;   /* accessibility settings mirrored from the Settings app */
    uint32_t kbd_mods;               /* last modifier state seen on a key event; stamped onto mouse events */
    int press_win; shell_zone press_zone;     /* chrome button pressed, fires on release */
    int last_click_win; uint32_t last_click_ms, now_ms;   /* title double-click -> maximize; host sets now_ms */
} shell_comp;

void comp_init(shell_comp *c, int screen_w, int screen_h);
void comp_set_work_area(shell_comp *c, shell_rect wa);
bool comp_add_overlay(shell_comp *c, void (*fn)(shell_comp *, shell_fb *, shell_rect, void *), void *user);
void comp_set_scale(shell_comp *c, uint32_t pct);              /* 100..300: chrome metrics follow */
int  comp_create(shell_comp *c, const char *title, shell_rect rect, uint32_t flags, uint32_t owner);   /* id or 0 */
void comp_destroy(shell_comp *c, int id);
shell_window *comp_get(shell_comp *c, int id);
void comp_set_title(shell_comp *c, int id, const char *title);
void comp_set_rect(shell_comp *c, int id, shell_rect r);
void comp_set_client(shell_comp *c, int id, uint32_t *px, int w, int h, int stride);
void comp_invalidate(shell_comp *c, int id, shell_rect client_rect);          /* app drew; mark region dirty */
void comp_raise(shell_comp *c, int id);
void comp_lower(shell_comp *c, int id);
void comp_focus(shell_comp *c, int id);
void comp_minimize(shell_comp *c, int id);
void comp_restore(shell_comp *c, int id);
void comp_toggle_maximize(shell_comp *c, int id);
shell_rect comp_client_rect(const shell_comp *c, const shell_window *w);      /* in screen coordinates */
shell_zone comp_hit_test(const shell_comp *c, int x, int y, int *win_id);     /* topmost visible window at point */
bool comp_is_shown(const shell_comp *c, const shell_window *w);               /* visible, not minimized, on this desktop */

/* input from the device layer */
void comp_mouse_move(shell_comp *c, int x, int y);
void comp_mouse_button(shell_comp *c, int button, bool down);
void comp_mouse_wheel(shell_comp *c, int delta);
void comp_key(shell_comp *c, uint32_t key, bool down, uint32_t mods);
void comp_char(shell_comp *c, uint32_t cp);
bool comp_pop_event(shell_comp *c, int id, shell_event *out);

/* output */
void comp_render(shell_comp *c, shell_fb *fb);          /* repaints only damaged regions, clears damage */
void comp_render_full(shell_comp *c, shell_fb *fb);
void comp_damage_all(shell_comp *c);
/* Overlays (taskbar, launcher, notifications, prompts) are laid out in logical px like apps: these map to/from the device screen. */
int  comp_lw(const shell_comp *c);                                       /* logical screen size = device size * 100 / scale */
int  comp_lh(const shell_comp *c);
int  comp_logical(const shell_comp *c, int device_px);                   /* device coordinate -> logical */
shell_rect comp_clip_logical(const shell_comp *c, shell_rect device_clip);
shell_fb comp_view(const shell_comp *c, const shell_fb *fb);             /* the same pixels addressed in logical px */
void comp_damage_logical(shell_comp *c, shell_rect r);
void comp_notify(shell_comp *c, int win);
void comp_post_event(shell_comp *c, int id, shell_event e);        /* shell-internal producers (wm, taskbar) */
void comp_request_close(shell_comp *c, int id);                  /* queue EV_CLOSE_REQUEST (Alt+F4, taskbar menu) */

/* ---------- window manager: snap layouts, virtual desktops, monitors/DPI, focus policy, tiling ---------- */
#define SHELL_MAX_MONITORS 4
typedef struct { shell_rect rect; uint32_t scale_pct; char name[16]; } shell_monitor;
typedef enum { FOCUS_CLICK, FOCUS_FOLLOWS_MOUSE } shell_focus_mode;
typedef enum {
    SNAP_NONE, SNAP_LEFT, SNAP_RIGHT, SNAP_MAXIMIZE, SNAP_TL, SNAP_TR, SNAP_BL, SNAP_BR,
    SNAP_THIRD_L, SNAP_THIRD_C, SNAP_THIRD_R, SNAP_TWO_THIRDS_L, SNAP_TWO_THIRDS_R
} shell_snap;
typedef struct {
    shell_comp *comp;
    shell_monitor mon[SHELL_MAX_MONITORS]; int nmon;
    shell_focus_mode focus_mode;
    bool tiling;                       /* master/stack auto-tiling of the current desktop */
    int gap;                           /* px between tiled/snapped windows */
    int ndesktops;
    shell_snap preview; int preview_mon;   /* live snap preview while dragging a title bar */
    int was_grab;                      /* window being dragged last time we looked */
    int master_pct;                    /* tiling master width, percent */
} shell_wm;

void wm_init(shell_wm *wm, shell_comp *c);
bool wm_add_monitor(shell_wm *wm, shell_rect r, uint32_t scale_pct, const char *name);
int  wm_monitor_at(const shell_wm *wm, int x, int y);                 /* index, nearest monitor if outside all */
shell_rect wm_monitor_work_area(const shell_wm *wm, int mon);         /* monitor 0 loses the taskbar strip */
shell_rect wm_snap_rect(const shell_wm *wm, int mon, shell_snap s);
shell_snap wm_detect_snap(const shell_wm *wm, int x, int y, int *mon);   /* edge/corner proximity of the cursor */
bool wm_snap_window(shell_wm *wm, int win, shell_snap s, int mon);
void wm_unsnap(shell_wm *wm, int win);
void wm_after_mouse(shell_wm *wm);                  /* call after every comp_mouse_*: preview, commit on release, DPI, focus-follows */
bool wm_key(shell_wm *wm, uint32_t key, bool down, uint32_t mods);   /* Super+arrows/digits/T; true = consumed */
void wm_switch_desktop(shell_wm *wm, int n);
bool wm_move_to_desktop(shell_wm *wm, int win, int n);
void wm_move_to_monitor(shell_wm *wm, int win, int mon);
void wm_set_tiling(shell_wm *wm, bool on);
void wm_retile(shell_wm *wm);
void wm_draw_preview(shell_comp *c, shell_fb *fb, shell_rect clip, void *user);   /* overlay: translucent snap target */
void wm_install(shell_wm *wm);                      /* registers the preview overlay */

/* ---------- app inventory ---------- */
#define SHELL_MAX_APPS 128
typedef enum { ITEM_APP, ITEM_FILE, ITEM_SETTING } shell_item_kind;
typedef struct {
    char name[48];            /* display name */
    char target[128];         /* exec path / file path / settings page id */
    char keywords[96];        /* extra searchable words, space separated */
    uint32_t color;           /* monochrome-icon tint (0 = theme text) */
    shell_item_kind kind;
} shell_item;
typedef struct { shell_item item[SHELL_MAX_APPS]; int n; } shell_inventory;
void inventory_init(shell_inventory *inv);
bool inventory_add(shell_inventory *inv, const char *name, const char *target, const char *keywords, shell_item_kind kind);
int  inventory_parse(shell_inventory *inv, const char *text);        /* "kind|name|target|keywords" per line, # comments; returns items added */
void inventory_defaults(shell_inventory *inv);                       /* the built-in LoOS apps and settings pages */

/* ---------- launcher (Start menu) ---------- */
#define LAUNCHER_QUERY_MAX 64
#define LAUNCHER_RESULTS 8
typedef struct {
    const shell_inventory *inv;
    char query[LAUNCHER_QUERY_MAX];
    int nq;
    int hit[LAUNCHER_RESULTS]; int nhit;     /* inventory indices, best first */
    int sel;
    bool open;
    void (*launch)(const shell_item *item, void *user);
    void *user;
} shell_launcher;
int  fuzzy_score(const char *query, const char *name, const char *keywords);   /* 0 = no match, higher is better */
void launcher_init(shell_launcher *l, const shell_inventory *inv, void (*launch)(const shell_item *, void *), void *user);
void launcher_open(shell_launcher *l);
void launcher_close(shell_launcher *l);
void launcher_search(shell_launcher *l);
bool launcher_key(shell_launcher *l, uint32_t key, uint32_t mods);     /* returns true when consumed */
bool launcher_char(shell_launcher *l, uint32_t cp);
shell_rect launcher_rect(const shell_comp *c);
void launcher_draw(const shell_launcher *l, const shell_comp *c, shell_fb *fb, shell_rect clip);
bool launcher_mouse(shell_launcher *l, const shell_comp *c, int x, int y, bool down);

/* ---------- taskbar ---------- */
#define TASKBAR_H 40
typedef enum { NET_IDLE, NET_ACTIVE, NET_BLOCKED } shell_net_state;
typedef struct shell_taskbar {
    struct shell_comp *comp;
    shell_launcher *launcher;
    int hover_btn;                     /* -1 none, 0 start, 1..n window buttons, -2 net toggle, -3 LED */
    int net_sockets;                   /* open sockets reported by NetMonitor */
    bool net_hard_off;                 /* tray "no network" switch */
    int hour, minute;                  /* local time, fed by the shell */
    void (*on_net_toggle)(bool hard_off, void *user);
    void *user;
} shell_taskbar;
void taskbar_init(shell_taskbar *t, shell_comp *c, shell_launcher *l);   /* also installs the overlay and reserves the work area */
void taskbar_set_time(shell_taskbar *t, int hour, int minute);
void taskbar_set_net(shell_taskbar *t, int sockets);
shell_net_state taskbar_net_state(const shell_taskbar *t);
shell_rect taskbar_rect(const shell_comp *c);
bool taskbar_mouse(shell_taskbar *t, shell_comp *c, int x, int y, int button, bool down);   /* true = consumed */
bool taskbar_key(shell_taskbar *t, shell_comp *c, uint32_t key, bool down, uint32_t mods);  /* Super toggles the launcher; Alt+Tab cycles */
void taskbar_draw(const shell_taskbar *t, const shell_comp *c, shell_fb *fb, shell_rect clip);
int  taskbar_button_count(const shell_taskbar *t, const shell_comp *c);
shell_rect taskbar_button_rect(const shell_taskbar *t, const shell_comp *c, int index);    /* index into window buttons */
int  taskbar_button_window(const shell_taskbar *t, const shell_comp *c, int index);

#ifdef __cplusplus
}
#endif
#endif
