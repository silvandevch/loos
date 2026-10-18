#include "../include/loos_nt.h"

void NTAPI RtlExitUserProcess(NTSTATUS code)
{
    NtTerminateProcess(NtCurrentProcess(), code);
    for (;;) { }
}
