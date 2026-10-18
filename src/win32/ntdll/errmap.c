/* RtlNtStatusToDosError: NTSTATUS -> Win32 error. The table covers the statuses
 * the Win32 layer and common apps actually produce; unknown statuses follow the
 * documented fallback rules (see end of function). Values are the public
 * winerror.h / ntstatus.h contract. */
#include "../include/loos_win.h"

static const struct { NTSTATUS st; ULONG err; } map[] = {
    { STATUS_SUCCESS,               ERROR_SUCCESS },
    { STATUS_BUFFER_OVERFLOW,       234 /* ERROR_MORE_DATA */ },
    { STATUS_NO_MORE_FILES,         18  /* ERROR_NO_MORE_FILES */ },
    { STATUS_UNSUCCESSFUL,          31  /* ERROR_GEN_FAILURE */ },
    { STATUS_NOT_IMPLEMENTED,       ERROR_CALL_NOT_IMPLEMENTED },
    { STATUS_INVALID_INFO_CLASS,    ERROR_INVALID_PARAMETER },
    { STATUS_ACCESS_VIOLATION,      ERROR_NOACCESS },
    { STATUS_INVALID_HANDLE,        ERROR_INVALID_HANDLE },
    { STATUS_INVALID_PARAMETER,     ERROR_INVALID_PARAMETER },
    { STATUS_NO_SUCH_FILE,          ERROR_FILE_NOT_FOUND },
    { STATUS_END_OF_FILE,           ERROR_HANDLE_EOF },
    { STATUS_NO_MEMORY,             ERROR_NOT_ENOUGH_MEMORY },
    { STATUS_CONFLICTING_ADDRESSES, 487 /* ERROR_INVALID_ADDRESS */ },
    { STATUS_ACCESS_DENIED,         ERROR_ACCESS_DENIED },
    { STATUS_BUFFER_TOO_SMALL,      ERROR_INSUFFICIENT_BUFFER },
    { STATUS_OBJECT_TYPE_MISMATCH,  ERROR_INVALID_HANDLE },
    { STATUS_OBJECT_NAME_INVALID,   ERROR_INVALID_NAME },
    { STATUS_OBJECT_NAME_NOT_FOUND, ERROR_FILE_NOT_FOUND },
    { STATUS_OBJECT_NAME_COLLISION, ERROR_ALREADY_EXISTS },
    { STATUS_OBJECT_PATH_NOT_FOUND, ERROR_PATH_NOT_FOUND },
    { STATUS_SHARING_VIOLATION,     ERROR_SHARING_VIOLATION },
    { STATUS_DELETE_PENDING,        ERROR_ACCESS_DENIED },
    { STATUS_DISK_FULL,             ERROR_DISK_FULL },
    { STATUS_PROCEDURE_NOT_FOUND,   ERROR_PROC_NOT_FOUND },
    { STATUS_INVALID_IMAGE_FORMAT,  ERROR_BAD_EXE_FORMAT },
    { STATUS_PIPE_BROKEN,           ERROR_BROKEN_PIPE },
    { STATUS_DLL_NOT_FOUND,         ERROR_MOD_NOT_FOUND },
    { STATUS_DIRECTORY_NOT_EMPTY,   ERROR_DIR_NOT_EMPTY },
    { STATUS_FILE_IS_A_DIRECTORY,   ERROR_ACCESS_DENIED },
    { STATUS_NOT_A_DIRECTORY,       267 /* ERROR_DIRECTORY */ },
    { STATUS_MEDIA_WRITE_PROTECTED, 19  /* ERROR_WRITE_PROTECT */ },
    { STATUS_FILE_CLOSED,           ERROR_INVALID_HANDLE },
    { STATUS_MUTANT_NOT_OWNED,      ERROR_NOT_OWNER },
    { STATUS_SEMAPHORE_LIMIT_EXCEEDED, ERROR_TOO_MANY_POSTS },
    { STATUS_NOT_COMMITTED,         ERROR_INVALID_ADDRESS },
    { STATUS_PRIVILEGE_NOT_HELD,    ERROR_PRIVILEGE_NOT_HELD },
    { STATUS_CANCELLED,             ERROR_OPERATION_ABORTED },
    { STATUS_NAME_TOO_LONG,         ERROR_FILENAME_EXCED_RANGE },
    { STATUS_INSUFFICIENT_RESOURCES, ERROR_NOT_ENOUGH_MEMORY },
    { STATUS_REVISION_MISMATCH,     ERROR_OLD_WIN_VERSION },
    { STATUS_PENDING,               ERROR_IO_PENDING },
    { STATUS_TIMEOUT,               ERROR_TIMEOUT },
    { STATUS_OBJECT_NAME_EXISTS,    ERROR_ALREADY_EXISTS },
};

ULONG NTAPI RtlNtStatusToDosError(NTSTATUS status)
{
    size_t i;
    for (i = 0; i < sizeof map / sizeof map[0]; i++)
        if (map[i].st == status) return map[i].err;
    /* Win32-facility statuses (0xC007xxxx) carry the error in the low 16 bits. */
    if ((((ULONG)status) & 0xFFFF0000u) == 0xC0070000u) return (ULONG)status & 0xFFFF;
    /* HRESULT_FROM_WIN32 round-trip: 0x8007xxxx */
    if ((((ULONG)status) & 0xFFFF0000u) == 0x80070000u) return (ULONG)status & 0xFFFF;
    /* Unknown: informational/warning statuses succeed, errors become MR_MID_NOT_FOUND. */
    if (NT_SUCCESS(status)) return ERROR_SUCCESS;
    return ERROR_MR_MID_NOT_FOUND;
}
