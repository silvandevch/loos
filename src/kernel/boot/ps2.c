/* PS/2 keyboard (scancode set 1, as translated by the i8042) and mouse, polled. No interrupts yet. */
#include "kernel.h"

#define PS2_DATA 0x60
#define PS2_CMD  0x64

static int wait_write(void) { for (int i = 0; i < 100000; i++) if (!(inb(PS2_CMD) & 2)) return 1; return 0; }
static int wait_read(void) { for (int i = 0; i < 100000; i++) if (inb(PS2_CMD) & 1) return 1; return 0; }

static int mouse_cmd(uint8_t b)
{
    if (!wait_write()) return 0;
    outb(PS2_CMD, 0xd4);
    if (!wait_write()) return 0;
    outb(PS2_DATA, b);
    for (int tries = 0; tries < 8; tries++) {               /* skip stray keyboard/mouse bytes until the ACK */
        if (!wait_read()) return 0;
        uint8_t r = inb(PS2_DATA);
        if (r == 0xfa) return 1;
        if (r == 0xfe) return 0;                             /* resend request */
    }
    return 0;
}

static int have_mouse;
static uint8_t pkt[3]; static int pkt_n;
static int ext;                                              /* saw 0xE0 */

void ps2_init(void)
{
    while (inb(PS2_CMD) & 1) (void)inb(PS2_DATA);            /* drain */
    if (wait_write()) outb(PS2_CMD, 0xa8);                   /* enable the aux port */
    if (wait_write()) outb(PS2_CMD, 0x20);
    uint8_t cfg = wait_read() ? inb(PS2_DATA) : 0;
    cfg = (uint8_t)((cfg & ~0x20) | 0x01 | 0x02);            /* aux clock on; keep IRQ enables set (unused while polling) */
    if (wait_write()) outb(PS2_CMD, 0x60);
    if (wait_write()) outb(PS2_DATA, cfg);
    have_mouse = 0;                                          /* defaults, then enable reporting (retried: the first */
    for (int i = 0; i < 3 && !have_mouse; i++) {             /* command after enabling the aux port is often lost) */
        mouse_cmd(0xf6);
        have_mouse = mouse_cmd(0xf4);
    }
    pkt_n = 0;
    while (inb(PS2_CMD) & 1) (void)inb(PS2_DATA);
}

/* scancode set 1 -> ASCII for the make codes 0x02..0x39 (US layout) */
static const char plain[] = "\0\0331234567890-=\b\tqwertyuiop[]\r\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
static const char shifted[] = "\0\033!@#$%^&*()_+\b\tQWERTYUIOP{}\r\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";

int ps2_poll(struct kin *out)
{
    static int shift, ctrl, alt, super;
    for (int guard = 0; guard < 64 && (inb(PS2_CMD) & 1); guard++) {
        uint8_t st = inb(PS2_CMD), b = inb(PS2_DATA);
        if (st & 0x20) {                                     /* mouse byte */
            if (pkt_n == 0 && !(b & 8)) continue;            /* out of sync */
            pkt[pkt_n++] = b;
            if (pkt_n < 3) continue;
            pkt_n = 0;
            if (pkt[0] & 0xc0) continue;                     /* overflow */
            out->type = KIN_MOUSE;
            out->dx = (int)pkt[1] - ((pkt[0] & 0x10) ? 256 : 0);
            out->dy = -((int)pkt[2] - ((pkt[0] & 0x20) ? 256 : 0));
            out->buttons = pkt[0] & 7;
            return 1;
        }
        if (b == 0xe0) { ext = 1; continue; }
        int down = !(b & 0x80); uint8_t code = b & 0x7f;
        int e = ext; ext = 0;
        uint32_t key = 0, ch = 0;
        if (!e && (code == 0x2a || code == 0x36)) { shift = down; continue; }
        if (code == 0x1d) { ctrl = down; continue; }          /* both ctrl keys (right one is E0 1D) */
        if (code == 0x38) { alt = down; continue; }
        if (e) {
            switch (code) {
            case 0x48: key = 0x100; break; case 0x50: key = 0x101; break; case 0x4b: key = 0x102; break; case 0x4d: key = 0x103; break;
            case 0x47: key = 0x104; break; case 0x4f: key = 0x105; break; case 0x49: key = 0x106; break; case 0x51: key = 0x107; break;
            case 0x53: key = 127; break; case 0x5b: case 0x5c: key = 0x140; super = down; break;
            default: continue;
            }
        } else if (code >= 0x3b && code <= 0x44) key = 0x120u + (code - 0x3b);
        else if (code == 0x57 || code == 0x58) key = 0x120u + 10 + (code - 0x57);
        else if (code >= 0x02 && code <= 0x39) {
            char c = shift ? shifted[code] : plain[code];
            if (!c) continue;
            ch = (uint8_t)c;
            key = (uint8_t)plain[code];                   /* key events carry the unshifted symbol */
            if (key == '\r') key = 13;
            if (ch == '\r' || ch == '\b' || ch == '\t' || ch == 27) ch = 0;   /* editing keys are keys, not characters */
        } else continue;
        out->type = KIN_KEY; out->key = key; out->ch = ch; out->down = down;
        out->mods = (shift ? 1u : 0) | (ctrl ? 2u : 0) | (alt ? 4u : 0) | (super ? 8u : 0);
        return 1;
    }
    return 0;
}

int ps2_has_mouse(void) { return have_mouse; }

/* Raw events for the Linux layer: keys as Linux evdev codes (set 1 make codes, E0 keys mapped), modifiers
 * included; mouse packets as relative motion plus the left/right/middle button bits. */
int ps2_poll_raw(struct kraw *out)
{
    for (int guard = 0; guard < 64 && (inb(PS2_CMD) & 1); guard++) {
        uint8_t st = inb(PS2_CMD), b = inb(PS2_DATA);
        if (st & 0x20) {
            if (pkt_n == 0 && !(b & 8)) continue;
            pkt[pkt_n++] = b;
            if (pkt_n < 3) continue;
            pkt_n = 0;
            if (pkt[0] & 0xc0) continue;
            out->type = KIN_MOUSE;
            out->dx = (int)pkt[1] - ((pkt[0] & 0x10) ? 256 : 0);
            out->dy = -((int)pkt[2] - ((pkt[0] & 0x20) ? 256 : 0));
            out->buttons = pkt[0] & 7;
            return 1;
        }
        if (b == 0xe0) { ext = 1; continue; }
        if (b == 0xe1) continue;                                 /* Pause: ignored */
        int e = ext; ext = 0;
        uint8_t code = b & 0x7f;
        uint32_t ev = code;
        if (e) {
            static const uint8_t emap[][2] = { {0x1c, 96}, {0x1d, 97}, {0x35, 98}, {0x38, 100}, {0x47, 102}, {0x48, 103}, {0x49, 104},
                {0x4b, 105}, {0x4d, 106}, {0x4f, 107}, {0x50, 108}, {0x51, 109}, {0x52, 110}, {0x53, 111}, {0x5b, 125}, {0x5c, 126}, {0x5d, 127} };
            ev = 0;
            for (unsigned i = 0; i < sizeof emap / sizeof emap[0]; i++) if (emap[i][0] == code) ev = emap[i][1];
            if (!ev) continue;                                   /* fake shifts (E0 2A) and unknown keys */
        }
        out->type = KIN_KEY; out->key = ev; out->down = !(b & 0x80);
        return 1;
    }
    return 0;
}
