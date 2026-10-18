#ifndef FAKE_H
#define FAKE_H
#include "../../../src/win32/include/loos_win.h"
#include <setjmp.h>

#define FAKE_MAX_FILES 24
#define FAKE_MAX_HANDLES 48
#define FAKE_PATH 512
#define FAKE_DATA 16384
enum { FK_FILE = 1, FK_EVENT, FK_MUTEX, FK_SEM };

typedef struct { int used, is_dir, has_children; WCHAR path[FAKE_PATH]; uint8_t data[FAKE_DATA]; size_t size; DWORD attrs; } fake_file;
typedef struct { int used, kind, obj, delete_on_close, state, manual, max, abandoned, shared_with; uint64_t pos; ULONG access; WCHAR name[128]; } fake_handle;

typedef struct {
    fake_file files[FAKE_MAX_FILES];
    fake_handle h[FAKE_MAX_HANDLES];
    /* last NtCreateFile/NtOpenFile arguments */
    int create_calls, open_calls, close_calls, flush_calls, disposition_calls, wait_calls, delay_calls, vm_calls, vm_free_calls;
    ULONG c_access, c_disp, c_options, c_share, c_attrs, c_oattrs; PVOID c_sd; WCHAR c_path[FAKE_PATH];
    int last_read_off_given; LONGLONG last_read_off;
    ULONG dup_attrs;
    /* vm */
    HANDLE vm_proc; PVOID vm_req_base; SIZE_T vm_size; ULONG vm_type, vm_prot; int vm_query_class;
    /* sync */
    ULONG ev_access, ev_type; BOOLEAN ev_initial; int obj_named, obj_inherit; WCHAR obj_name[256];
    int wait_alertable, wait_timeout_null; LONGLONG wait_timeout; WAIT_TYPE wait_type; ULONG wait_n;
    int delay_alertable, delay_apc; LONGLONG delay; LONGLONG perf_counter;
    int module_refs; DWORD exit_code;
    void *vm_blocks[64]; int nvm_blocks;
} fake_state;

extern fake_state F;
extern jmp_buf fake_exit_jmp;
extern void *k32_host_image_base;
extern DWORD k32_host_pid, k32_host_tid;
extern _Thread_local DWORD k32_host_last_error;

void fake_reset(void);
void fake_wset(WCHAR *dst, const char *ascii);
int  fake_add_file(const char *nt_path, int is_dir, const void *data, size_t size);
int  fake_file_exists(const char *nt_path);
size_t fake_file_size(const char *nt_path);
#endif
