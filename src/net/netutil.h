/* Network helpers shared by the LoOS network stack modules: byte order, IPv4/IPv6 text and CIDR handling, the
 * Internet checksum. Pure C11, no allocation, no OS calls. */
#ifndef LOOS_NETUTIL_H
#define LOOS_NETUTIL_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

static inline uint16_t net_rd16(const uint8_t *p) { return (uint16_t)(p[0] << 8 | p[1]); }
static inline uint32_t net_rd32(const uint8_t *p) { return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3]; }
static inline void net_wr16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)(v >> 8); p[1] = (uint8_t)v; }
static inline void net_wr32(uint8_t *p, uint32_t v) { p[0] = (uint8_t)(v >> 24); p[1] = (uint8_t)(v >> 16); p[2] = (uint8_t)(v >> 8); p[3] = (uint8_t)v; }

typedef struct { uint8_t b[4]; } ip4_addr;
typedef struct { uint8_t b[16]; } ip6_addr;
typedef struct { ip4_addr addr; uint8_t prefix; } ip4_cidr;
typedef struct { ip6_addr addr; uint8_t prefix; } ip6_cidr;

static inline uint32_t ip4_u32(ip4_addr a) { return net_rd32(a.b); }
static inline ip4_addr ip4_from_u32(uint32_t v) { ip4_addr a; net_wr32(a.b, v); return a; }
static inline bool ip4_eq(ip4_addr a, ip4_addr b) { return ip4_u32(a) == ip4_u32(b); }

bool ip4_parse(const char *s, ip4_addr *out);                     /* strict dotted quad: no leading zeros, no trailing junk */
size_t ip4_format(ip4_addr a, char *out, size_t cap);             /* returns length excluding NUL */
bool ip6_parse(const char *s, ip6_addr *out);                     /* RFC 4291 text incl. "::" and embedded IPv4 tail */
size_t ip6_format(const ip6_addr *a, char *out, size_t cap);      /* RFC 5952 canonical form */
bool ip4_cidr_parse(const char *s, ip4_cidr *out);
bool ip6_cidr_parse(const char *s, ip6_cidr *out);
uint32_t ip4_mask(unsigned prefix);
bool ip4_in_cidr(ip4_addr a, ip4_cidr c);
bool ip6_in_cidr(const ip6_addr *a, ip6_cidr c);
bool ip4_is_private(ip4_addr a);                                  /* RFC 1918 */
bool ip4_is_loopback(ip4_addr a);
bool ip4_is_link_local(ip4_addr a);                               /* 169.254/16 */
bool ip4_is_multicast(ip4_addr a);
int  ip4_prefix_from_mask(ip4_addr mask);                         /* -1 for a non-contiguous mask */

uint16_t net_checksum(const void *data, size_t len, uint32_t seed);   /* ones-complement sum folded and inverted */
uint32_t net_checksum_add(const void *data, size_t len, uint32_t acc);   /* running sum for pseudo-headers */
uint16_t net_checksum_finish(uint32_t acc);

#ifdef __cplusplus
}
#endif
#endif
