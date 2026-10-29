/* Web fetch bridge ABI for the LoOS Browser: host-assisted HTTP(S) while the native network stack is being built.
 * Implemented in Rust by crates/loos-shell/src/web.rs; in QEMU a second serial port (COM2) carries a small
 * compressed request/response protocol to boot/web-bridge.py. On a real machine, or when the bridge is absent,
 * this returns false and the browser stays offline. */
#ifndef LOOS_SHELL_WEB_H
#define LOOS_SHELL_WEB_H
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Fetch `url` through the QEMU bridge. Writes raw-DEFLATE HTML/CSS into `out` (cap bytes), sets the compressed
 * length, the decompressed length, and the final URL after redirects. Returns false when the bridge is absent or
 * the request failed. No sockets: this is a serial-port protocol to a host helper, never a network call. */
bool loos_web_fetch(const char *url, char *out, size_t cap, size_t *out_len, size_t *raw_len,
                    char *final_url, size_t final_cap);

#ifdef __cplusplus
}
#endif
#endif
