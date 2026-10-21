#include "features.h"
#undef assert
#ifdef NDEBUG
#define assert(x) ((void)0)
#else
__LOOS_BEGIN
__LOOS_NORETURN void __assert_fail(const char *expr, const char *file, int line, const char *func);
__LOOS_END
#define assert(x) ((x) ? (void)0 : __assert_fail(#x, __FILE__, __LINE__, __func__))
#endif
#ifndef static_assert
#define static_assert _Static_assert
#endif
