#ifndef FAKE_H
#define FAKE_H
#include "../../../src/win32/include/loos_win.h"
#include <setjmp.h>

#define FAKE_MAX_FILES 160
#define FAKE_MAX_HANDLES 48
#define FAKE_PATH 512
#define FAKE_DATA 16384
enum { FK_FILE = 1, FK_EVENT, FK_MUTEX, FK_SEM, FK_CONSOLE, FK_THREAD, FK_PROCESS, FK_SECTION };

typedef struct { int used, is_dir, has_children; WCHAR path[FAKE_PATH]; uint8_t data[FAKE_DATA]; size_t size; DWORD attrs; LONGLONG ctime, atime, mtime; } fake_file;
typedef struct { int used, kind, obj, delete_on_close, state, manual, max, abandoned, shared_with, suspend, dir_index; NTSTATUS exit_status; uint64_t pos; ULONG access; WCHAR name[128]; } fake_handle;

typedef struct { int used; int file; size_t size; uint8_t *anon; ULONG protect, attrs; } fake_section;   /* file < 0: backed by anon */
typedef struct { int used; int sec; size_t off, len; uint8_t *base; int writable, shared; } fake_view;
typedef struct {
    fake_file files[FAKE_MAX_FILES];
    fake_handle h[FAKE_MAX_HANDLES];
    /* last NtCreateFile/NtOpenFile arguments */
    int create_calls, open_calls, close_calls, flush_calls, disposition_calls, wait_calls, delay_calls, vm_calls, vm_free_calls;
    ULONG c_access, c_disp, c_options, c_share, c_attrs, c_oattrs; PVOID c_sd; WCHAR c_path[FAKE_PATH];
    int last_read_off_given; LONGLONG last_read_off;
    ULONG dup_attrs;
    fake_section sec[16]; fake_view view[16]; ULONG map_protect; int map_calls, unmap_calls, flush_view_calls;
    /* vm */
    HANDLE vm_proc; PVOID vm_req_base; SIZE_T vm_size; ULONG vm_type, vm_prot; int vm_query_class;
    /* sync */
    ULONG ev_access, ev_type; BOOLEAN ev_initial; int obj_named, obj_inherit; WCHAR obj_name[256];
    int wait_alertable, wait_timeout_null; LONGLONG wait_timeout; WAIT_TYPE wait_type; ULONG wait_n;
    int delay_alertable, delay_apc; LONGLONG delay; LONGLONG perf_counter;
    int module_refs; DWORD exit_code;
    void *vm_blocks[64]; int nvm_blocks;
    LONGLONG system_time; uintptr_t noaccess_page;
    struct {                                   /* the fake console device */
        ULONG mode_in, mode_out, cp_in, cp_out; int16_t cx, cy; uint16_t attr; char title[128];
        unsigned char out[32768]; size_t nout; unsigned char in[8192]; size_t nin, in_pos; size_t read_chunk;
        int last_fill_kind; ULONG last_fill_value, last_fill_count; int16_t fill_x, fill_y; int ioctl_calls; ULONG last_ioctl;
    } con;
    /* threads and processes */
    struct { ULONG access, flags; HANDLE process; PVOID start, arg; SIZE_T commit, reserve; ULONG oattrs; int calls; } thr;
    int next_tid, next_pid, yield_result, suspend_calls, resume_calls;
    NTSTATUS term_status; HANDLE term_handle; int term_calls;
    struct { int calls; NTSTATUS fail; ULONG pflags, tflags, paccess, taccess, poattrs, toattrs; WCHAR image[512]; uint8_t params[8192]; size_t params_len; } cu;
    LONG last_priority;
    NTSTATUS rename_status; size_t write_limit; size_t written_total; int dir_queries; ULONG last_dir_mask_len;
    ULONG devtype_override;                    /* if nonzero, NtQueryVolumeInformationFile reports this DeviceType */
} fake_state;

extern fake_state F;
extern RTL_USER_PROCESS_PARAMETERS *k32_host_params;
extern jmp_buf fake_exit_jmp;
extern void *k32_host_image_base;
extern DWORD k32_host_pid;
extern _Thread_local DWORD k32_host_tid, k32_host_last_error;
extern _Thread_local PVOID k32_host_tls[64];
extern _Thread_local PVOID *k32_host_tls_expansion;
extern _Thread_local PVOID k32_host_fls;

void fake_reset(void);
void fake_wset(WCHAR *dst, const char *ascii);
int  fake_add_file(const char *nt_path, int is_dir, const void *data, size_t size);
int  fake_file_exists(const char *nt_path);
size_t fake_file_size(const char *nt_path);
#endif
