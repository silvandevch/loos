/* Delay-load support: LdrResolveDelayLoadedAPI.
 *
 * Called by the delay-import thunk of a module the first time one of its delay-loaded functions is used.
 * Loads the DLL (once; the handle is cached in the descriptor's module-handle slot), looks the symbol up
 * by name or ordinal, stores it in the delay IAT slot and returns it. Descriptors are the RVA-based form.
 * If anything fails and the caller supplied no failure hooks the process ends with the status Windows
 * reports for an unhandled delay-load failure (0xC06D007E module, 0xC06D007F procedure); the hook
 * arguments are accepted but not yet invoked. */
#include "../include/loos_nt.h"
#include <string.h>

typedef struct { ULONG Attributes, DllNameRVA, ModuleHandleRVA, ImportAddressTableRVA, ImportNameTableRVA, BoundImportAddressTableRVA, UnloadImportAddressTableRVA, TimeDateStamp; } DELAYLOAD_DESCRIPTOR;

static void fail(NTSTATUS st) { RtlExitUserProcess(st); }

PVOID NTAPI LdrResolveDelayLoadedAPI(PVOID base, const void *descriptor, PVOID dll_hook, PVOID sys_hook, PVOID *thunk, ULONG flags)
{
    const DELAYLOAD_DESCRIPTOR *d = descriptor;
    const char *dll;
    PVOID *iat, *module_slot, module;
    const uint64_t *int_tbl;
    uint64_t entry;
    size_t idx, i, n;
    WCHAR wname[260];
    UNICODE_STRING us;
    ANSI_STRING as;
    PVOID addr = NULL;
    NTSTATUS st;
    (void)dll_hook; (void)sys_hook; (void)flags;

    if (!base || !d || !thunk || !(d->Attributes & 1)) fail((NTSTATUS)0xC06D007Eu);          /* only the RVA form is accepted */
    dll = (const char *)base + d->DllNameRVA;
    module_slot = (PVOID *)((char *)base + d->ModuleHandleRVA);
    iat = (PVOID *)((char *)base + d->ImportAddressTableRVA);
    int_tbl = (const uint64_t *)((char *)base + d->ImportNameTableRVA);
    idx = (size_t)(thunk - iat);

    module = __atomic_load_n(module_slot, __ATOMIC_ACQUIRE);
    if (!module) {
        n = strlen(dll);
        if (n >= 259) fail((NTSTATUS)0xC06D007Eu);
        for (i = 0; i <= n; i++) wname[i] = (WCHAR)(unsigned char)dll[i];
        us.Buffer = wname; us.Length = (USHORT)(n * sizeof(WCHAR)); us.MaximumLength = (USHORT)(us.Length + sizeof(WCHAR));
        if (!NT_SUCCESS(LdrLoadDll(NULL, NULL, &us, &module))) fail((NTSTATUS)0xC06D007Eu);
        __atomic_store_n(module_slot, module, __ATOMIC_RELEASE);
    }
    entry = int_tbl[idx];
    if (entry >> 63) {
        st = LdrGetProcedureAddress(module, NULL, (ULONG)(entry & 0xFFFF), &addr);
    } else {
        const char *nm = (const char *)base + (uint32_t)entry + 2;                             /* skip the hint */
        as.Buffer = (char *)nm; as.Length = (USHORT)strlen(nm); as.MaximumLength = (USHORT)(as.Length + 1);
        st = LdrGetProcedureAddress(module, &as, 0, &addr);
    }
    if (!NT_SUCCESS(st)) fail((NTSTATUS)0xC06D007Fu);
    __atomic_store_n(thunk, addr, __ATOMIC_RELEASE);
    return addr;
}
