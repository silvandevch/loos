/* Per-thread / per-process block accessors shared by ntdll and kernel32.
 *
 * Real builds read the TEB through gs (x64 Windows layout): ClientId at 0x40/0x48, PEB at 0x60,
 * LastErrorValue at 0x68, TlsSlots[64] at 0x1480, TlsExpansionSlots at 0x1780, FlsData at 0x17C8;
 * PEB->ImageBaseAddress at +0x10, PEB->ProcessParameters at +0x20. Host tests (LOOS_HOST_TEST)
 * keep plain thread-local state so the logic can run under an ordinary Linux process. */
#ifndef LOOS_TEB_H
#define LOOS_TEB_H
#include "loos_nt.h"

#ifdef LOOS_HOST_TEST
extern RTL_USER_PROCESS_PARAMETERS *k32_host_params;
extern _Thread_local DWORD k32_host_last_error;
extern _Thread_local PVOID k32_host_tls[64];
extern _Thread_local PVOID *k32_host_tls_expansion;
extern _Thread_local PVOID k32_host_fls;
extern DWORD k32_host_pid;
extern _Thread_local DWORD k32_host_tid;
extern void *k32_host_image_base;
static inline DWORD *k32_last_error_ptr(void) { return &k32_host_last_error; }
static inline DWORD k32_pid(void) { return k32_host_pid; }
static inline DWORD k32_tid(void) { return k32_host_tid; }
static inline void *k32_image_base(void) { return k32_host_image_base; }
static inline RTL_USER_PROCESS_PARAMETERS *k32_params(void) { return k32_host_params; }
static inline PVOID *k32_tls_slots(void) { return k32_host_tls; }
static inline PVOID **k32_tls_expansion_slot(void) { return &k32_host_tls_expansion; }
static inline PVOID *k32_fls_slot(void) { return &k32_host_fls; }
#else
static inline uint64_t k32_gs_q(unsigned off) { uint64_t v; __asm__ volatile("movq %%gs:%1, %0" : "=r"(v) : "m"(*(const uint64_t *)(uintptr_t)off)); return v; }
static inline uint8_t *k32_teb(void) { return (uint8_t *)k32_gs_q(0x30); }
static inline DWORD *k32_last_error_ptr(void) { return (DWORD *)(k32_teb() + 0x68); }
static inline DWORD k32_pid(void) { return (DWORD)k32_gs_q(0x40); }
static inline DWORD k32_tid(void) { return (DWORD)k32_gs_q(0x48); }
static inline void *k32_image_base(void) { return *(void **)(k32_gs_q(0x60) + 0x10); }
static inline RTL_USER_PROCESS_PARAMETERS *k32_params(void) { return *(RTL_USER_PROCESS_PARAMETERS **)(k32_gs_q(0x60) + 0x20); }
static inline PVOID *k32_tls_slots(void) { return (PVOID *)(k32_teb() + 0x1480); }
static inline PVOID **k32_tls_expansion_slot(void) { return (PVOID **)(k32_teb() + 0x1780); }
static inline PVOID *k32_fls_slot(void) { return (PVOID *)(k32_teb() + 0x17C8); }
#endif
#endif
