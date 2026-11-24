#ifndef _LANGINFO_H
#define _LANGINFO_H
#include <_prism_cdefs.h>
typedef int nl_item;
#define CODESET 0
#define D_T_FMT 1
#define D_FMT 2
#define T_FMT 3
#define T_FMT_AMPM 4
#define AM_STR 5
#define PM_STR 6
#define RADIXCHAR 50
#define THOUSEP 51
__BEGIN_DECLS
char *nl_langinfo(nl_item);
__END_DECLS
#include <xlocale.h>
#endif
