#include "k32_internal.h"

LPVOID WINAPI VirtualAlloc(LPVOID addr, SIZE_T size, DWORD type, DWORD protect)
{
    PVOID base = addr;
    SIZE_T sz = size;
    NTSTATUS st;
    if (!size) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    st = NtAllocateVirtualMemory(NtCurrentProcess(), &base, 0, &sz, type, protect);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return base;
}

BOOL WINAPI VirtualFree(LPVOID addr, SIZE_T size, DWORD type)
{
    PVOID base = addr;
    SIZE_T sz = size;
    NTSTATUS st;
    /* MEM_RELEASE requires size 0 (whole reservation); exactly one of RELEASE/DECOMMIT must be given. */
    if ((type & (MEM_RELEASE | MEM_DECOMMIT)) == 0 || (type & MEM_RELEASE && type & MEM_DECOMMIT) ||
        ((type & MEM_RELEASE) && size != 0) || !addr) {
        *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
        return FALSE;
    }
    st = NtFreeVirtualMemory(NtCurrentProcess(), &base, &sz, type);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI VirtualProtect(LPVOID addr, SIZE_T size, DWORD newprot, DWORD *oldprot)
{
    PVOID base = addr;
    SIZE_T sz = size;
    ULONG old = 0;
    NTSTATUS st;
    if (!oldprot) { *k32_last_error_ptr() = ERROR_NOACCESS; return FALSE; }
    st = NtProtectVirtualMemory(NtCurrentProcess(), &base, &sz, newprot, &old);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    *oldprot = old;
    return TRUE;
}

SIZE_T WINAPI VirtualQuery(LPCVOID addr, MEMORY_BASIC_INFORMATION *info, SIZE_T len)
{
    SIZE_T ret = 0;
    NTSTATUS st;
    if (len < sizeof *info) { *k32_last_error_ptr() = ERROR_BAD_LENGTH; return 0; }
    st = NtQueryVirtualMemory(NtCurrentProcess(), (PVOID)addr, 0 /* MemoryBasicInformation */, info, len, &ret);
    if (!NT_SUCCESS(st)) {
        /* an address beyond the user range is a parameter error, not "no such memory" */
        k32_set_nt_error(st == STATUS_INVALID_PARAMETER ? STATUS_INVALID_PARAMETER : st);
        return 0;
    }
    return ret;
}
