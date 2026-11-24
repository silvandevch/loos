#ifndef _NL_TYPES_H
#define _NL_TYPES_H
#include <_prism_cdefs.h>
typedef struct __nl_cat_d *nl_catd;
typedef int nl_item;
#define NL_CAT_LOCALE 1
__BEGIN_DECLS
nl_catd catopen(const char *, int); char *catgets(nl_catd, int, int, const char *); int catclose(nl_catd);
__END_DECLS
#endif
