/* Exact-area anti-aliased scanline rasteriser for quadratic outlines (integer only, usable in the kernel). */
#ifndef LOOS_SHELL_RASTER_H
#define LOOS_SHELL_RASTER_H
#include <stdint.h>

#define RASTER_MAX_SIZE 256           /* longest side of a bitmap */

typedef struct {
    const int16_t *xy;               /* x,y pairs in path units, y up */
    const uint8_t *on;               /* on-curve flags */
    const uint16_t *ends;            /* exclusive end index of each contour (relative to xy/on) */
    int ncont;
} raster_path;

/* Renders into cov (w*h bytes, cleared first). Path unit -> device: k_q24 is pixels per unit * 2^24; (ox_q8, oy_q8) is the
 * device position of the path origin in 1/256 px from the bitmap's top-left (y flipped: path y up becomes bitmap y down).
 * Non-zero winding. Returns 0 on success, -1 if the path is too complex for the edge buffer. */
int raster_fill(const raster_path *p, int32_t k_q24, int ox_q8, int oy_q8, int w, int h, uint8_t *cov);
#endif
