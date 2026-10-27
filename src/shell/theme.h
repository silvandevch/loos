/* Theme engine: a theme is a small spec (base palette + accent + tint + shape/effect knobs + per-token color
 * overrides) that resolves to the compositor's palette and style. Everything is local config (ui.* keys), can be
 * exported/imported as text, and passes through a contrast guard so a custom theme cannot make text unreadable. */
#ifndef LOOS_SHELL_THEME_H
#define LOOS_SHELL_THEME_H
#include "shell.h"
#include "backend.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum { TB_DARK, TB_LIGHT, TB_HC, TB_AMOLED, TB_NORD, TB_DRACULA, TB_SOLARIZED, TB_GRUVBOX, TB_PAPER, TB_COUNT } theme_base;
typedef enum { TK_DESKTOP, TK_SURFACE, TK_SURFACE_HI, TK_BORDER, TK_TEXT, TK_TEXT_DIM, TK_ACCENT, TK_ACCENT_TEXT, TK_DANGER, TK_TITLE_INACTIVE, TK_COUNT } theme_token;

typedef struct {
    theme_base base;
    uint32_t accent;                 /* 0xAARRGGBB, alpha forced opaque */
    int tint;                        /* 0..100: how strongly surfaces/borders lean toward the accent hue */
    int radius;                      /* window corner radius, logical px, 0..24 (-1 = scale default) */
    int title_h;                     /* title bar height, 20..48 (0 = default 28) */
    int border;                      /* window border, 1..4 (0 = default) */
    int shadow;                      /* 0..150 percent of the default drop shadow */
    int glow;                        /* 0..150 percent of the wallpaper glows; 0 = flat gradient (fastest) */
    bool guard;                      /* enforce readable contrast (default on) */
    uint32_t ovr[TK_COUNT];          /* per-token override colors */
    uint16_t ovr_mask;               /* bit i set = ovr[i] is active */
} theme_spec;

extern const char *const theme_base_name[TB_COUNT];
extern const char *const theme_token_name[TK_COUNT];

void theme_spec_default(theme_spec *s, uint32_t accent);
void theme_resolve(const theme_spec *s, shell_theme *out);     /* base -> tint -> overrides -> contrast guard */
void theme_apply(const theme_spec *s, shell_comp *c);          /* palette + metrics + effects, then damages the screen */

/* persistence: ui.base, ui.accent_hex, ui.tint, ui.radius, ui.title_h, ui.border, ui.shadow, ui.glow, ui.guard, ui.c.<token> */
void theme_spec_load(theme_spec *s, const shell_backend *be, uint32_t legacy_accent);
void theme_spec_store(const theme_spec *s, const shell_backend *be);

/* text form: "base=nord;accent=#88C0D0;tint=10;radius=12;c.surface=#101820" (';' or newline separated) */
size_t theme_export(const theme_spec *s, char *out, size_t cap);
int    theme_import(theme_spec *s, const char *text);          /* number of rejected entries; accepted ones apply */

/* presets */
int         theme_preset_count(void);
const char *theme_preset_name(int i);
bool        theme_preset(int i, theme_spec *out);

/* color helpers (also used by the Settings page) */
uint32_t theme_hsl(int h, int s, int l);                       /* h 0..359, s/l 0..100 */
void     theme_to_hsl(uint32_t c, int *h, int *s, int *l);
int      theme_contrast_x100(uint32_t a, uint32_t b);          /* WCAG ratio * 100 (e.g. 450 = 4.5:1) */
bool     theme_parse_hex(const char *s, uint32_t *out);        /* "#RRGGBB" or "RRGGBB" */
void     theme_format_hex(uint32_t c, char out[8]);

#ifdef __cplusplus
}
#endif
#endif
