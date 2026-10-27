#include "theme.h"
#include <string.h>
#include <stdio.h>

#define RGB(r, g, b) (0xFF000000u | (uint32_t)(r) << 16 | (uint32_t)(g) << 8 | (uint32_t)(b))
#define PAL(d, s, h, b, t, td, dg, ti) { (d), (s), (h), (b), (t), (td), 0, 0xFFFFFFFFu, (dg), (ti) }

const char *const theme_base_name[TB_COUNT] = { "dark", "light", "hc", "amoled", "nord", "dracula", "solarized", "gruvbox", "paper" };
const char *const theme_token_name[TK_COUNT] = { "desktop", "surface", "surface_hi", "border", "text", "text_dim", "accent", "accent_text", "danger", "title_inactive" };

static void base_palette(theme_base b, shell_theme *t)
{
    switch (b) {
    case TB_LIGHT:     shell_theme_light(t, 0xFF3B82F6u); return;
    case TB_HC:        shell_theme_high_contrast(t); return;
    case TB_AMOLED: { shell_theme p = PAL(RGB(0, 0, 0), RGB(0, 0, 0), RGB(0x10, 0x10, 0x12), RGB(0x28, 0x28, 0x2E), RGB(0xF5, 0xF5, 0xF7), RGB(0x9A, 0x9A, 0xA3), RGB(0xEF, 0x44, 0x44), RGB(0x0A, 0x0A, 0x0C)); *t = p; return; }
    case TB_NORD:   { shell_theme p = PAL(RGB(0x2E, 0x34, 0x40), RGB(0x3B, 0x42, 0x52), RGB(0x43, 0x4C, 0x5E), RGB(0x4C, 0x56, 0x6A), RGB(0xEC, 0xEF, 0xF4), RGB(0xA9, 0xB4, 0xC8), RGB(0xBF, 0x61, 0x6A), RGB(0x35, 0x3C, 0x4A)); *t = p; return; }
    case TB_DRACULA:{ shell_theme p = PAL(RGB(0x21, 0x22, 0x2C), RGB(0x28, 0x2A, 0x36), RGB(0x34, 0x37, 0x46), RGB(0x44, 0x47, 0x5A), RGB(0xF8, 0xF8, 0xF2), RGB(0xA9, 0xAE, 0xD0), RGB(0xFF, 0x55, 0x55), RGB(0x25, 0x27, 0x32)); *t = p; return; }
    case TB_SOLARIZED:{shell_theme p = PAL(RGB(0x00, 0x1F, 0x27), RGB(0x00, 0x2B, 0x36), RGB(0x07, 0x40, 0x4F), RGB(0x58, 0x6E, 0x75), RGB(0xEE, 0xE8, 0xD5), RGB(0x93, 0xA1, 0xA1), RGB(0xDC, 0x32, 0x2F), RGB(0x04, 0x36, 0x42)); *t = p; return; }
    case TB_GRUVBOX:{ shell_theme p = PAL(RGB(0x1D, 0x20, 0x21), RGB(0x28, 0x28, 0x28), RGB(0x3C, 0x38, 0x36), RGB(0x50, 0x49, 0x45), RGB(0xEB, 0xDB, 0xB2), RGB(0xA8, 0x99, 0x84), RGB(0xFB, 0x49, 0x34), RGB(0x2C, 0x2B, 0x2A)); *t = p; return; }
    case TB_PAPER:  { shell_theme p = PAL(RGB(0xED, 0xE6, 0xD6), RGB(0xFA, 0xF6, 0xEC), RGB(0xFF, 0xFD, 0xF7), RGB(0xD8, 0xCF, 0xBB), RGB(0x2B, 0x2A, 0x26), RGB(0x6B, 0x64, 0x55), RGB(0xC0, 0x39, 0x2B), RGB(0xF1, 0xEC, 0xDF)); *t = p; return; }
    default:           shell_theme_dark(t, 0xFF3B82F6u); return;
    }
}

static int iabs(int v) { return v < 0 ? -v : v; }   /* the freestanding kernel build has no iabs()/strtol() */

/* Decimal integer with optional sign; false on empty or trailing garbage. */
static bool parse_int(const char *s, long *out)
{
    bool neg = *s == '-';
    if (*s == '-' || *s == '+') s++;
    if (!*s) return false;
    long v = 0;
    for (; *s; s++) { if (*s < '0' || *s > '9') return false; v = v * 10 + (*s - '0'); if (v > 100000000) v = 100000000; }
    *out = neg ? -v : v;
    return true;
}

static int clampi(int v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : v; }

/* ---------- color math ---------- */
static const int lin_tab[17] = { 0, 561, 1600, 3310, 5781, 9084, 13287, 18447, 24620, 31855, 40198, 49693, 60383, 72306, 85499, 100000, 100000 };
static int lin(int c)                                     /* sRGB code -> linear light * 100000, interpolated */
{
    int i = c / 17, f = c % 17;
    return lin_tab[i] + (lin_tab[i + 1] - lin_tab[i]) * f / 17;
}
static int lum(uint32_t c) { return (2126 * lin((int)((c >> 16) & 255)) + 7152 * lin((int)((c >> 8) & 255)) + 722 * lin((int)(c & 255))) / 10000; }
int theme_contrast_x100(uint32_t a, uint32_t b)
{
    int la = lum(a), lb = lum(b);
    if (la < lb) { int t = la; la = lb; lb = t; }
    return (int)(((long long)(la + 5000) * 100) / (lb + 5000));       /* both scaled by 1e5: (L1+0.05)/(L2+0.05) */
}

void theme_to_hsl(uint32_t c, int *h, int *s, int *l)
{
    int r = (int)((c >> 16) & 255), g = (int)((c >> 8) & 255), b = (int)(c & 255);
    int mx = r > g ? (r > b ? r : b) : (g > b ? g : b), mn = r < g ? (r < b ? r : b) : (g < b ? g : b);
    int d = mx - mn, L = (mx + mn) * 50 / 255;                          /* 0..100 */
    int S = 0, H = 0;
    if (d) {
        int den = 255 - iabs(mx + mn - 255);
        S = den ? d * 100 / den : 0;
        if (mx == r) H = 60 * (g - b) / d;
        else if (mx == g) H = 120 + 60 * (b - r) / d;
        else H = 240 + 60 * (r - g) / d;
        if (H < 0) H += 360;
    }
    if (h) *h = H % 360;
    if (s) *s = S > 100 ? 100 : S;
    if (l) *l = L;
}

uint32_t theme_hsl(int h, int s, int l)
{
    h = ((h % 360) + 360) % 360; s = clampi(s, 0, 100); l = clampi(l, 0, 100);
    int c = (100 - iabs(2 * l - 100)) * s / 100;                         /* chroma 0..100 */
    int hp = h * 100 / 60;                                               /* sector * 100 */
    int x = c * (100 - iabs(hp % 200 - 100)) / 100;
    int r = 0, g = 0, b = 0;
    switch (hp / 100) { case 0: r = c; g = x; break; case 1: r = x; g = c; break; case 2: g = c; b = x; break;
                        case 3: g = x; b = c; break; case 4: r = x; b = c; break; default: r = c; b = x; break; }
    int m = l - c / 2;
    int R = (r + m) * 255 / 100, G = (g + m) * 255 / 100, B = (b + m) * 255 / 100;
    R = clampi(R, 0, 255); G = clampi(G, 0, 255); B = clampi(B, 0, 255);
    return RGB(R, G, B);
}

bool theme_parse_hex(const char *s, uint32_t *out)
{
    if (*s == '#') s++;
    if (strlen(s) != 6) return false;
    uint32_t v = 0;
    for (int i = 0; i < 6; i++) {
        char ch = s[i]; int d;
        if (ch >= '0' && ch <= '9') d = ch - '0'; else if (ch >= 'a' && ch <= 'f') d = ch - 'a' + 10; else if (ch >= 'A' && ch <= 'F') d = ch - 'A' + 10; else return false;
        v = v << 4 | (uint32_t)d;
    }
    *out = 0xFF000000u | v;
    return true;
}
void theme_format_hex(uint32_t c, char out[8]) { snprintf(out, 8, "#%06X", (unsigned)(c & 0xFFFFFFu)); }

/* ---------- spec ---------- */
void theme_spec_default(theme_spec *s, uint32_t accent)
{
    memset(s, 0, sizeof *s);
    s->base = TB_DARK; s->accent = accent | 0xFF000000u; s->radius = -1; s->shadow = 100; s->glow = 100; s->guard = true;
}

static uint32_t* slot(shell_theme *t, theme_token k)
{
    switch (k) { case TK_DESKTOP: return &t->desktop; case TK_SURFACE: return &t->surface; case TK_SURFACE_HI: return &t->surface_hi;
                 case TK_BORDER: return &t->border; case TK_TEXT: return &t->text; case TK_TEXT_DIM: return &t->text_dim;
                 case TK_ACCENT: return &t->accent; case TK_ACCENT_TEXT: return &t->accent_text; case TK_DANGER: return &t->danger;
                 default: return &t->title_inactive; }
}

/* move `fg` toward white or black (whichever helps) until it reaches `want` contrast against `bg` */
static uint32_t fix_contrast(uint32_t fg, uint32_t bg, int want_x100)
{
    if (theme_contrast_x100(fg, bg) >= want_x100) return fg;
    uint32_t target = lum(bg) > 18000 ? 0xFF000000u : 0xFFFFFFFFu;     /* away from the background */
    for (int t = 16; t <= 256; t += 16) {
        uint32_t c = color_mix(fg, target, t);
        if (theme_contrast_x100(c, bg) >= want_x100) return c;
    }
    return target;
}

void theme_resolve(const theme_spec *s, shell_theme *out)
{
    shell_theme t;
    base_palette(s->base, &t);
    if (s->base != TB_HC) t.accent = s->accent | 0xFF000000u;
    int tint = s->tint < 0 ? 0 : s->tint > 100 ? 100 : s->tint;
    if (tint && s->base != TB_HC) {                                      /* up to ~25% of the accent hue into the neutrals */
        int k = tint * 64 / 100;
        t.desktop = color_mix(t.desktop, t.accent, k); t.surface = color_mix(t.surface, t.accent, k);
        t.surface_hi = color_mix(t.surface_hi, t.accent, k); t.border = color_mix(t.border, t.accent, k);
        t.title_inactive = color_mix(t.title_inactive, t.accent, k);
    }
    for (int i = 0; i < TK_COUNT; i++) if (s->ovr_mask & (1u << i)) *slot(&t, (theme_token)i) = s->ovr[i] | 0xFF000000u;
    if (s->guard && s->base != TB_HC) {
        t.text = fix_contrast(t.text, t.surface, 450);
        t.text_dim = fix_contrast(t.text_dim, t.surface, 300);
        t.accent_text = fix_contrast(t.accent_text, t.accent, 300);
    }
    *out = t;
}

void theme_apply(const theme_spec *s, shell_comp *c)
{
    shell_style st = { s->radius, s->title_h, s->border, s->shadow, s->glow };
    theme_resolve(s, &c->theme);
    comp_set_style(c, &st);                                              /* recomputes metrics and damages everything */
}

/* ---------- persistence ---------- */

static void sanitize(theme_spec *s)
{
    s->tint = clampi(s->tint, 0, 100); s->radius = clampi(s->radius, -1, 24);
    s->title_h = s->title_h ? clampi(s->title_h, 20, 48) : 0; s->border = s->border ? clampi(s->border, 1, 4) : 0;
    s->shadow = clampi(s->shadow, 0, 150); s->glow = clampi(s->glow, 0, 150);
    if ((unsigned)s->base >= TB_COUNT) s->base = TB_DARK;
    s->accent |= 0xFF000000u;
}

void theme_spec_load(theme_spec *s, const shell_backend *be, uint32_t legacy_accent)
{
    theme_spec_default(s, legacy_accent);
    char v[96];
    int legacy = be_cfg_int(be, "ui.theme", 0);
    s->base = legacy == 1 ? TB_LIGHT : legacy == 2 ? TB_HC : TB_DARK;
    if (be->cfg_get(be->user, "ui.base", v, sizeof v)) {
        for (int i = 0; i < TB_COUNT; i++) if (!strcmp(v, theme_base_name[i])) s->base = (theme_base)i;
    }
    uint32_t a;
    if (be->cfg_get(be->user, "ui.accent_hex", v, sizeof v) && theme_parse_hex(v, &a)) s->accent = a;
    s->tint = be_cfg_int(be, "ui.tint", 0);
    s->radius = be_cfg_int(be, "ui.radius", -1);
    s->title_h = be_cfg_int(be, "ui.title_h", 0);
    s->border = be_cfg_int(be, "ui.border", 0);
    s->shadow = be_cfg_int(be, "ui.shadow", 100);
    s->glow = be_cfg_int(be, "ui.glow", 100);
    s->guard = be_cfg_bool(be, "ui.guard", true);
    for (int i = 0; i < TK_COUNT; i++) {
        char k[48]; snprintf(k, sizeof k, "ui.c.%s", theme_token_name[i]);
        if (be->cfg_get(be->user, k, v, sizeof v) && theme_parse_hex(v, &a)) { s->ovr[i] = a; s->ovr_mask |= (uint16_t)(1u << i); }
    }
    sanitize(s);
}

void theme_spec_store(const theme_spec *s, const shell_backend *be)
{
    char hex[8];
    be->cfg_set(be->user, "ui.base", theme_base_name[s->base]);
    theme_format_hex(s->accent, hex); be->cfg_set(be->user, "ui.accent_hex", hex);
    be_cfg_set_int(be, "ui.tint", s->tint); be_cfg_set_int(be, "ui.radius", s->radius); be_cfg_set_int(be, "ui.title_h", s->title_h);
    be_cfg_set_int(be, "ui.border", s->border); be_cfg_set_int(be, "ui.shadow", s->shadow); be_cfg_set_int(be, "ui.glow", s->glow);
    be_cfg_set_bool(be, "ui.guard", s->guard);
    for (int i = 0; i < TK_COUNT; i++) {
        char k[48]; snprintf(k, sizeof k, "ui.c.%s", theme_token_name[i]);
        if (s->ovr_mask & (1u << i)) { theme_format_hex(s->ovr[i], hex); be->cfg_set(be->user, k, hex); }
        else be->cfg_set(be->user, k, "");                              /* empty = no override (parse_hex rejects it) */
    }
    be_cfg_set_int(be, "ui.theme", s->base == TB_LIGHT || s->base == TB_PAPER ? 1 : s->base == TB_HC ? 2 : 0);   /* legacy readers */
    be_cfg_set_bool(be, "ui.high_contrast", s->base == TB_HC);
}

/* ---------- text form ---------- */
size_t theme_export(const theme_spec *s, char *out, size_t cap)
{
    char hex[8];
    int n = snprintf(out, cap, "base=%s;", theme_base_name[s->base]);
    theme_format_hex(s->accent, hex);
    n += snprintf(out + (n < (int)cap ? n : (int)cap), cap > (size_t)n ? cap - (size_t)n : 0, "accent=%s;tint=%d;radius=%d;title_h=%d;border=%d;shadow=%d;glow=%d;guard=%d", hex, s->tint, s->radius, s->title_h, s->border, s->shadow, s->glow, s->guard);
    for (int i = 0; i < TK_COUNT; i++) if (s->ovr_mask & (1u << i)) {
        theme_format_hex(s->ovr[i], hex);
        n += snprintf(out + (n < (int)cap ? n : (int)cap), cap > (size_t)n ? cap - (size_t)n : 0, ";c.%s=%s", theme_token_name[i], hex);
    }
    return (size_t)n;
}

int theme_import(theme_spec *s, const char *text)
{
    theme_spec t = *s;
    int bad = 0;
    char buf[640]; strncpy(buf, text, sizeof buf - 1); buf[sizeof buf - 1] = 0;
    char *p = buf;
    while (*p) {
        char *e = p; while (*e && *e != ';' && *e != '\n' && *e != '\r') e++;
        char sep = *e; *e = 0;
        while (*p == ' ') p++;
        if (*p && *p != '#') {
            char *eq = strchr(p, '=');
            if (!eq) { bad++; }
            else {
                *eq = 0; const char *k = p, *v = eq + 1; uint32_t col;
                long n = 0; bool isnum = parse_int(v, &n);
                if (!strcmp(k, "base")) { int f = -1; for (int i = 0; i < TB_COUNT; i++) if (!strcmp(v, theme_base_name[i])) f = i; if (f < 0) bad++; else t.base = (theme_base)f; }
                else if (!strcmp(k, "accent")) { if (theme_parse_hex(v, &col)) t.accent = col; else bad++; }
                else if (!strcmp(k, "tint") && isnum) t.tint = (int)n;
                else if (!strcmp(k, "radius") && isnum) t.radius = (int)n;
                else if (!strcmp(k, "title_h") && isnum) t.title_h = (int)n;
                else if (!strcmp(k, "border") && isnum) t.border = (int)n;
                else if (!strcmp(k, "shadow") && isnum) t.shadow = (int)n;
                else if (!strcmp(k, "glow") && isnum) t.glow = (int)n;
                else if (!strcmp(k, "guard") && isnum) t.guard = n != 0;
                else if (!strncmp(k, "c.", 2)) {
                    int f = -1; for (int i = 0; i < TK_COUNT; i++) if (!strcmp(k + 2, theme_token_name[i])) f = i;
                    if (f >= 0 && theme_parse_hex(v, &col)) { t.ovr[f] = col; t.ovr_mask |= (uint16_t)(1u << f); } else bad++;
                } else bad++;
            }
        }
        if (!sep) break;
        p = e + 1;
    }
    sanitize(&t);
    *s = t;
    return bad;
}

/* ---------- presets ---------- */
typedef struct { const char *name; theme_base base; uint32_t accent; int tint, radius, title_h, border, shadow, glow; } preset;
static const preset presets[] = {
    { "LoOS Dark",   TB_DARK,      0xFF3B82F6u,  0, -1,  0, 0, 100, 100 },
    { "LoOS Light",  TB_LIGHT,     0xFF3B82F6u,  0, -1,  0, 0, 100, 100 },
    { "High contrast", TB_HC,      0xFF00FFFFu,  0, -1,  0, 2,   0,   0 },
    { "AMOLED",      TB_AMOLED,    0xFF22D3EEu,  0, 12,  0, 0,   0,  60 },
    { "Nord",        TB_NORD,      0xFF88C0D0u,  0, 10,  0, 0,  80,  70 },
    { "Dracula",     TB_DRACULA,   0xFFBD93F9u,  8, 10,  0, 0, 110, 120 },
    { "Solarized",   TB_SOLARIZED, 0xFF2AA198u,  0,  6,  0, 0,  70,  60 },
    { "Gruvbox",     TB_GRUVBOX,   0xFFFABD2Fu,  0,  4,  0, 0,  80,  50 },
    { "Paper",       TB_PAPER,     0xFFB45309u,  0,  6,  0, 0,  60,  40 },
    { "Glass",       TB_DARK,      0xFF60A5FAu, 18, 16, 32, 0, 140, 150 },
    { "Compact",     TB_DARK,      0xFF3B82F6u,  0,  4, 24, 1,  40,   0 },
    { "Performance", TB_DARK,      0xFF3B82F6u,  0,  0, 24, 1,   0,   0 },
};
int theme_preset_count(void) { return (int)(sizeof presets / sizeof presets[0]); }
const char *theme_preset_name(int i) { return i >= 0 && i < theme_preset_count() ? presets[i].name : ""; }
bool theme_preset(int i, theme_spec *o)
{
    if (i < 0 || i >= theme_preset_count()) return false;
    const preset *p = &presets[i];
    theme_spec_default(o, p->accent);
    o->base = p->base; o->tint = p->tint; o->radius = p->radius; o->title_h = p->title_h; o->border = p->border; o->shadow = p->shadow; o->glow = p->glow;
    return true;
}
