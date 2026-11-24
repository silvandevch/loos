#ifndef _OS_SYNC_WAIT_ON_ADDRESS_H
#define _OS_SYNC_WAIT_ON_ADDRESS_H
#include <_prism_cdefs.h>
typedef uint32_t os_sync_wait_on_address_flags_t, os_sync_wake_by_address_flags_t;
typedef uint32_t os_clockid_t;
#define OS_SYNC_WAIT_ON_ADDRESS_NONE 0
#define OS_SYNC_WAIT_ON_ADDRESS_SHARED 1
#define OS_SYNC_WAKE_BY_ADDRESS_NONE 0
#define OS_SYNC_WAKE_BY_ADDRESS_SHARED 1
#define OS_CLOCK_MACH_ABSOLUTE_TIME 32
__BEGIN_DECLS
int os_sync_wait_on_address(void *, uint64_t, size_t, os_sync_wait_on_address_flags_t);
int os_sync_wait_on_address_with_timeout(void *, uint64_t, size_t, os_sync_wait_on_address_flags_t, os_clockid_t, uint64_t);
int os_sync_wake_by_address_any(void *, size_t, os_sync_wake_by_address_flags_t);
int os_sync_wake_by_address_all(void *, size_t, os_sync_wake_by_address_flags_t);
__END_DECLS
#endif
