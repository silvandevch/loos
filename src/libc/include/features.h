/* LoOS libc: internal feature macros shared by all public headers. */
#ifndef LOOS_LIBC_FEATURES_H
#define LOOS_LIBC_FEATURES_H
#ifdef __cplusplus
#define __LOOS_BEGIN extern "C" {
#define __LOOS_END }
#else
#define __LOOS_BEGIN
#define __LOOS_END
#endif
#define __LOOS_LIBC__ 1
#if defined(__GNUC__)
#define __LOOS_PRINTF(a, b) __attribute__((format(printf, a, b)))
#define __LOOS_NORETURN __attribute__((noreturn))
#else
#define __LOOS_PRINTF(a, b)
#define __LOOS_NORETURN
#endif
#endif
