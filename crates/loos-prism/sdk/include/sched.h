#ifndef _SCHED_H
#define _SCHED_H
#include <_prism_cdefs.h>
struct sched_param { int sched_priority; char __opaque[4]; };
__BEGIN_DECLS
int sched_yield(void);
__END_DECLS
#endif
