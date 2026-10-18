#ifndef LOOS_APISET_TABLE_H
#define LOOS_APISET_TABLE_H
#include <stddef.h>
struct apiset_entry { const char *contract; const char *host; };
/* Sorted by strcmp() on .contract (enforced by tests/win32/apisets). */
extern const struct apiset_entry apiset_table[];
extern const size_t apiset_table_len;
#endif
