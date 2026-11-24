#ifndef _MACH_MACH_TIME_H
#define _MACH_MACH_TIME_H
#include <_prism_cdefs.h>
typedef int kern_return_t;
struct mach_timebase_info { uint32_t numer, denom; };
typedef struct mach_timebase_info *mach_timebase_info_t;
typedef struct mach_timebase_info mach_timebase_info_data_t;
__BEGIN_DECLS
uint64_t mach_absolute_time(void); uint64_t mach_continuous_time(void);
kern_return_t mach_timebase_info(mach_timebase_info_t);
__END_DECLS
#endif
