#include "raster.h"
#include <string.h>

#define SS 16                         /* sub-scanlines per pixel row */
#define MAX_EDGES 12000

typedef struct { int32_t x0, y0, x1, y1; } edge;      /* 24.8 device coordinates, y0 != y1 */
static edge edges[MAX_EDGES];
static int nedge, overflow;

static void add_edge(int32_t x0, int32_t y0, int32_t x1, int32_t y1)
{
    if (y0 == y1) return;
    if (nedge >= MAX_EDGES) { overflow = 1; return; }
    edges[nedge++] = (edge){ x0, y0, x1, y1 };
}

static void quad(int32_t x0, int32_t y0, int32_t cx, int32_t cy, int32_t x1, int32_t y1)
{
    /* segment count from the control polygon's deviation (in 1/256 px) */
    int64_t dx = x0 - 2 * (int64_t)cx + x1, dy = y0 - 2 * (int64_t)cy + y1;
    int64_t dev = (dx < 0 ? -dx : dx) + (dy < 0 ? -dy : dy);              /* ~ 4x the max deviation */
    int n = 1;
    while (n < 16 && (dev / 4) > (int64_t)n * n * 24) n++;                /* flatness ~ 0.1 px */
    int32_t px = x0, py = y0;
    for (int i = 1; i <= n; i++) {
        int64_t t = (int64_t)i * 65536 / n, u = 65536 - t;
        int32_t x = (int32_t)((u * u * x0 + 2 * u * t * cx + t * t * x1) >> 32);
        int32_t y = (int32_t)((u * u * y0 + 2 * u * t * cy + t * t * y1) >> 32);
        add_edge(px, py, x, y);
        px = x; py = y;
    }
}

int raster_fill(const raster_path *p, int32_t k, int ox, int oy, int w, int h, uint8_t *cov)
{
    memset(cov, 0, (size_t)w * (size_t)h);
    if (w <= 0 || h <= 0 || w > RASTER_MAX_SIZE || h > RASTER_MAX_SIZE) return -1;
    nedge = 0; overflow = 0;
    static int32_t X[1024], Y[1024]; static uint8_t ON[1024];
    int start = 0;
    for (int c = 0; c < p->ncont; c++) {
        int end = p->ends[c], n = end - start;
        if (n >= 2 && n <= 1024) {
            for (int i = 0; i < n; i++) {
                X[i] = (int32_t)(((int64_t)p->xy[2 * (start + i)] * k) >> 16) + ox;
                Y[i] = oy - (int32_t)(((int64_t)p->xy[2 * (start + i) + 1] * k) >> 16);
                ON[i] = p->on[start + i];
            }
            int s0 = -1;
            for (int i = 0; i < n; i++) if (ON[i]) { s0 = i; break; }
            int32_t sx, sy; int first, count;
            if (s0 >= 0) { sx = X[s0]; sy = Y[s0]; first = s0 + 1; count = n - 1; }
            else { sx = (X[0] + X[1]) / 2; sy = (Y[0] + Y[1]) / 2; first = 1; count = n; }    /* all off-curve: start at an implied point */
            int32_t cx = sx, cy = sy, qx = 0, qy = 0; int have_ctrl = 0;
            for (int j = 0; j <= count; j++) {
                int32_t x, y; int on;
                if (j == count) { x = sx; y = sy; on = 1; }                                  /* close the contour */
                else { int idx = (first + j) % n; x = X[idx]; y = Y[idx]; on = ON[idx]; }
                if (on) {
                    if (have_ctrl) { quad(cx, cy, qx, qy, x, y); have_ctrl = 0; } else add_edge(cx, cy, x, y);
                    cx = x; cy = y;
                } else {
                    if (have_ctrl) {                                                           /* implied on-curve midpoint */
                        int32_t mx = (qx + x) / 2, my = (qy + y) / 2;
                        quad(cx, cy, qx, qy, mx, my);
                        cx = mx; cy = my;
                    }
                    qx = x; qy = y; have_ctrl = 1;
                }
            }
        }
        start = end;
    }
    if (overflow) return -1;

    static uint16_t acc[RASTER_MAX_SIZE];
    for (int row = 0; row < h; row++) {
        memset(acc, 0, (size_t)w * sizeof acc[0]);
        for (int s = 0; s < SS; s++) {
            int32_t ys = row * 256 + (2 * s + 1) * (256 / SS) / 2;            /* sub-scanline centre */
            int32_t xs[64]; int8_t dir[64]; int nx = 0;
            for (int i = 0; i < nedge && nx < 64; i++) {
                const edge *e = &edges[i];
                int32_t lo = e->y0 < e->y1 ? e->y0 : e->y1, hi = e->y0 < e->y1 ? e->y1 : e->y0;
                if (ys < lo || ys >= hi) continue;
                int32_t x = e->x0 + (int32_t)(((int64_t)(ys - e->y0) * (e->x1 - e->x0)) / (e->y1 - e->y0));
                int j = nx++;
                while (j > 0 && xs[j - 1] > x) { xs[j] = xs[j - 1]; dir[j] = dir[j - 1]; j--; }
                xs[j] = x; dir[j] = e->y1 > e->y0 ? 1 : -1;
            }
            int wind = 0; int32_t xa = 0;
            for (int i = 0; i < nx; i++) {
                int was = wind;
                wind += dir[i];
                if (!was && wind) { xa = xs[i]; continue; }
                if (!(was && !wind)) continue;
                int32_t xb = xs[i];
                if (xa < 0) xa = 0;
                if (xb > w * 256) xb = w * 256;
                if (xb <= xa) continue;
                for (int px = xa >> 8, last = (xb - 1) >> 8; px <= last; px++) {
                    int32_t l = px * 256 > xa ? px * 256 : xa, r = (px + 1) * 256 < xb ? (px + 1) * 256 : xb;
                    acc[px] = (uint16_t)(acc[px] + (r - l));
                }
            }
        }
        for (int x = 0; x < w; x++) {
            uint32_t v = acc[x] * 255u / (256u * SS);
            cov[(size_t)row * (size_t)w + (size_t)x] = (uint8_t)(v > 255 ? 255 : v);
        }
    }
    return 0;
}
