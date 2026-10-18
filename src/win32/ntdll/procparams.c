/* Process parameter blocks (RTL_USER_PROCESS_PARAMETERS).
 *
 * RtlCreateProcessParametersEx returns a block in the *denormalized* form the kernel expects for a new
 * process: one allocation holding the structure followed by every string and the environment, with the
 * UNICODE_STRING.Buffer fields and Environment holding byte offsets from the start of the block.
 * Normalizing turns those offsets into pointers; the kernel normalizes its copy in the child. */
#include "../include/loos_nt.h"
#include <string.h>

static size_t block_wchars(const WCHAR *env)
{
    const WCHAR *e = env;
    if (!env) return 0;
    while (*e) { while (*e) e++; e++; }
    return (size_t)(e - env) + 1;
}


static UNICODE_STRING *field(RTL_USER_PROCESS_PARAMETERS *p, int i)
{
    switch (i) {
    case 0: return &p->CurrentDirectoryDosPath;
    case 1: return &p->DllPath;
    case 2: return &p->ImagePathName;
    case 3: return &p->CommandLine;
    case 4: return &p->WindowTitle;
    case 5: return &p->DesktopInfo;
    case 6: return &p->ShellInfo;
    default: return &p->RuntimeData;
    }
}
#define NFIELDS 8

RTL_USER_PROCESS_PARAMETERS *NTAPI RtlNormalizeProcessParams(RTL_USER_PROCESS_PARAMETERS *p)
{
    int i;
    if (!p || (p->Flags & RTL_USER_PROC_PARAMS_NORMALIZED)) return p;
    for (i = 0; i < NFIELDS; i++) { UNICODE_STRING *u = field(p, i); if (u->Buffer) u->Buffer = (PWSTR)((char *)p + (uintptr_t)u->Buffer); }
    if (p->Environment) p->Environment = (char *)p + (uintptr_t)p->Environment;
    p->Flags |= RTL_USER_PROC_PARAMS_NORMALIZED;
    return p;
}

RTL_USER_PROCESS_PARAMETERS *NTAPI RtlDeNormalizeProcessParams(RTL_USER_PROCESS_PARAMETERS *p)
{
    int i;
    if (!p || !(p->Flags & RTL_USER_PROC_PARAMS_NORMALIZED)) return p;
    for (i = 0; i < NFIELDS; i++) { UNICODE_STRING *u = field(p, i); if (u->Buffer) u->Buffer = (PWSTR)(uintptr_t)((char *)u->Buffer - (char *)p); }
    if (p->Environment) p->Environment = (PVOID)(uintptr_t)((char *)p->Environment - (char *)p);
    p->Flags &= ~RTL_USER_PROC_PARAMS_NORMALIZED;
    return p;
}

NTSTATUS NTAPI RtlCreateProcessParametersEx(RTL_USER_PROCESS_PARAMETERS **out, UNICODE_STRING *image, UNICODE_STRING *dll_path,
                                            UNICODE_STRING *cwd, UNICODE_STRING *cmd, PVOID env, UNICODE_STRING *title,
                                            UNICODE_STRING *desktop, UNICODE_STRING *shell, UNICODE_STRING *runtime, ULONG flags)
{
    UNICODE_STRING *src[NFIELDS] = { cwd, dll_path, image, cmd, title, desktop, shell, runtime };
    size_t total = sizeof(RTL_USER_PROCESS_PARAMETERS), env_bytes = block_wchars((const WCHAR *)env) * sizeof(WCHAR), off;
    RTL_USER_PROCESS_PARAMETERS *p;
    char *blk;
    int i;

    if (!out) return STATUS_INVALID_PARAMETER;
    for (i = 0; i < NFIELDS; i++) if (src[i]) total += (size_t)src[i]->Length + sizeof(WCHAR);
    total = (total + 15u) & ~(size_t)15u;
    off = total;
    total += env_bytes;
    p = RtlAllocateHeap(RtlGetProcessHeap(), HEAP_ZERO_MEMORY, total);
    if (!p) return STATUS_NO_MEMORY;
    blk = (char *)p;
    p->MaximumLength = (ULONG)total; p->Length = (ULONG)total;
    off = sizeof(RTL_USER_PROCESS_PARAMETERS);
    for (i = 0; i < NFIELDS; i++) {
        UNICODE_STRING *d = field(p, i);
        if (!src[i] || !src[i]->Buffer) continue;
        memcpy(blk + off, src[i]->Buffer, src[i]->Length);
        *(WCHAR *)(blk + off + src[i]->Length) = 0;
        d->Length = src[i]->Length; d->MaximumLength = (USHORT)(src[i]->Length + sizeof(WCHAR));
        d->Buffer = (PWSTR)(uintptr_t)off;                      /* denormalized: offset from the block */
        off += (size_t)src[i]->Length + sizeof(WCHAR);
    }
    if (env_bytes) {
        size_t eoff = (off + 15u) & ~(size_t)15u;
        memcpy(blk + eoff, env, env_bytes);
        p->Environment = (PVOID)(uintptr_t)eoff;
        p->EnvironmentSize = env_bytes;
    }
    p->Flags = 0;
    if (flags & RTL_USER_PROC_PARAMS_NORMALIZED) RtlNormalizeProcessParams(p);
    *out = p;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI RtlDestroyProcessParameters(RTL_USER_PROCESS_PARAMETERS *p)
{
    return RtlFreeHeap(RtlGetProcessHeap(), 0, p) ? STATUS_SUCCESS : STATUS_INVALID_PARAMETER;
}
