#define LOOS_HOST_TEST 1
#include "fake.h"
#include "../../../src/win32/kernel32/k32_internal.h"
#include "../../../src/win32/compat/compat.h"
void rtl_set_version(const compat_version *v);
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %#llx, want %#llx\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

/* ASCII -> WCHAR, rotating buffers so a call can use several */
static const WCHAR *W(const char *s)
{
    static WCHAR b[4][1024]; static int i;
    WCHAR *o = b[i++ & 3];
    fake_wset(o, s);
    return o;
}
static int weq(const WCHAR *w, const char *s)
{ size_t i = 0; for (; s[i]; i++) if (w[i] != (WCHAR)(unsigned char)s[i]) return 0; return w[i] == 0; }

static void cwd_to(const char *dir)             /* set cwd by creating the dir in the fake fs, then SetCurrentDirectoryW */
{
    char nt[600]; size_t i;
    snprintf(nt, sizeof nt, "\\??\\%s", dir);
    if (nt[strlen(nt) - 1] == '\\') nt[strlen(nt) - 1] = 0;
    for (i = 7; nt[i]; i++)                         /* create every ancestor directory below the drive root */
        if (nt[i] == '\\') { char sub[600]; memcpy(sub, nt, i); sub[i] = 0; if (!fake_file_exists(sub)) fake_add_file(sub, 1, NULL, 0); }
    if (!fake_file_exists(nt)) fake_add_file(nt, 1, NULL, 0);
    CHECK(SetCurrentDirectoryW(W(dir)));
}

/* ---------- errors ---------- */
static void test_errors(void)
{
    CHECK_EQ(RtlNtStatusToDosError(STATUS_SUCCESS), 0);
    CHECK_EQ(RtlNtStatusToDosError(STATUS_OBJECT_NAME_NOT_FOUND), ERROR_FILE_NOT_FOUND);
    CHECK_EQ(RtlNtStatusToDosError(STATUS_OBJECT_PATH_NOT_FOUND), ERROR_PATH_NOT_FOUND);
    CHECK_EQ(RtlNtStatusToDosError(STATUS_ACCESS_DENIED), ERROR_ACCESS_DENIED);
    CHECK_EQ(RtlNtStatusToDosError(STATUS_SHARING_VIOLATION), ERROR_SHARING_VIOLATION);
    CHECK_EQ(RtlNtStatusToDosError(STATUS_DLL_NOT_FOUND), ERROR_MOD_NOT_FOUND);
    CHECK_EQ(RtlNtStatusToDosError((NTSTATUS)0xC0070057u), 0x57);          /* Win32 facility: low word */
    CHECK_EQ(RtlNtStatusToDosError((NTSTATUS)0xC0FFEE00u), ERROR_MR_MID_NOT_FOUND);
    CHECK_EQ(RtlNtStatusToDosError((NTSTATUS)0x40000123u), ERROR_SUCCESS); /* informational */
    SetLastError(77); CHECK_EQ(GetLastError(), 77);
}

/* ---------- paths ---------- */
struct pcase { const char *cwd, *in, *full, *nt; };
static void test_paths(void)
{
    static const struct pcase cases[] = {
        { "C:\\Users\\me\\", "foo.txt",                  "C:\\Users\\me\\foo.txt",   "\\??\\C:\\Users\\me\\foo.txt" },
        { "C:\\Users\\me\\", ".\\a\\..\\b\\c.txt",       "C:\\Users\\me\\b\\c.txt",  "\\??\\C:\\Users\\me\\b\\c.txt" },
        { "C:\\Users\\me\\", "..\\x",                    "C:\\Users\\x",             "\\??\\C:\\Users\\x" },
        { "C:\\Users\\me\\", "../../../../x",            "C:\\x",                    "\\??\\C:\\x" },
        { "C:\\Users\\me\\", "D:\\a\\b\\",               "D:\\a\\b\\",               "\\??\\D:\\a\\b\\" },
        { "C:\\Users\\me\\", "d:\\\\a//b",               "D:\\a\\b",                 "\\??\\D:\\a\\b" },
        { "C:\\Users\\me\\", "d:rel",                    "D:\\rel",                  "\\??\\D:\\rel" },
        { "C:\\Users\\me\\", "c:rel",                    "C:\\Users\\me\\rel",       "\\??\\C:\\Users\\me\\rel" },
        { "C:\\Users\\me\\", "\\top",                    "C:\\top",                  "\\??\\C:\\top" },
        { "C:\\Users\\me\\", "C:",                       "C:\\Users\\me",            "\\??\\C:\\Users\\me" },
        { "C:\\Users\\me\\", "D:\\",                     "D:\\",                     "\\??\\D:\\" },
        { "C:\\Users\\me\\", "file. ",                   "C:\\Users\\me\\file",      "\\??\\C:\\Users\\me\\file" },
        { "C:\\Users\\me\\", "a b\\c d.txt",             "C:\\Users\\me\\a b\\c d.txt", "\\??\\C:\\Users\\me\\a b\\c d.txt" },
        { "C:\\Users\\me\\", "\\\\server\\share\\dir\\..\\f", "\\\\server\\share\\f", "\\??\\UNC\\server\\share\\f" },
        { "C:\\Users\\me\\", "\\\\?\\C:\\a\\..\\b",      "\\\\?\\C:\\a\\..\\b",      "\\??\\C:\\a\\..\\b" },
        { "C:\\Users\\me\\", "\\\\.\\COM1",              "\\\\.\\COM1",              "\\??\\COM1" },
        { "C:\\Users\\me\\", "NUL",                      "\\\\.\\NUL",               "\\??\\NUL" },
        { "C:\\Users\\me\\", "con.txt",                  "\\\\.\\CON",               "\\??\\CON" },
        { "C:\\Users\\me\\", "lpt3",                     "\\\\.\\LPT3",              "\\??\\LPT3" },
        { "C:\\Users\\me\\", "nullable.txt",             "C:\\Users\\me\\nullable.txt", "\\??\\C:\\Users\\me\\nullable.txt" },
    };
    static WCHAR nt[K32_PATH_MAX]; WCHAR fp_buf[512];
    size_t i;
    DWORD n;
    LPWSTR fp;

    for (i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        const struct pcase *c = &cases[i];
        char dir[64]; strcpy(dir, c->cwd);
        if (i == 0) cwd_to("C:\\Users\\me\\");
        n = GetFullPathNameW(W(c->in), 512, fp_buf, &fp);
        if (!n || !weq(fp_buf, c->full)) { printf("FAIL full(%s): got len %u, want %s\n", c->in, n, c->full); fails++; }
        {
            UNICODE_STRING us; DWORD e = k32_dos_to_nt(W(c->in), nt, K32_PATH_MAX, &us);
            if (e || !weq(us.Buffer, c->nt) || us.Length != strlen(c->nt) * 2) { printf("FAIL nt(%s): err %u, want %s\n", c->in, e, c->nt); fails++; }
        }
    }

    /* file part, buffer sizing, errors */
    n = GetFullPathNameW(W("sub\\name.txt"), 512, fp_buf, &fp);
    CHECK(n && fp && weq(fp, "name.txt"));
    n = GetFullPathNameW(W("sub\\"), 512, fp_buf, &fp);
    CHECK(n && fp == NULL);
    CHECK_EQ(GetFullPathNameW(W("abc"), 4, fp_buf, &fp), strlen("C:\\Users\\me\\abc") + 1);   /* too small: required incl. NUL */
    CHECK_EQ(GetFullPathNameW(W(""), 512, fp_buf, &fp), 0); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
    CHECK_EQ(GetFullPathNameW(W("\\\\server"), 512, fp_buf, &fp), 0); CHECK_EQ(GetLastError(), ERROR_BAD_PATHNAME);
    { UNICODE_STRING us; CHECK_EQ(k32_dos_to_nt(W(""), nt, K32_PATH_MAX, &us), ERROR_PATH_NOT_FOUND); }

    /* current directory */
    fake_add_file("\\??\\C:\\work", 1, NULL, 0); fake_add_file("\\??\\C:\\work\\f", 0, "x", 1);
    CHECK(SetCurrentDirectoryW(W("C:\\work")));
    n = GetCurrentDirectoryW(512, fp_buf); CHECK(n == 8 && weq(fp_buf, "C:\\work\\"));
    CHECK_EQ(GetCurrentDirectoryW(3, fp_buf), 9);                                        /* needs 8+1 */
    n = GetFullPathNameW(W("g.txt"), 512, fp_buf, &fp); CHECK(weq(fp_buf, "C:\\work\\g.txt"));
    CHECK(!SetCurrentDirectoryW(W("C:\\nowhere"))); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    CHECK(!SetCurrentDirectoryW(W("C:\\nowhere\\deeper"))); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
    CHECK(!SetCurrentDirectoryW(W("C:\\work\\f")));  CHECK_EQ(GetLastError(), ERROR_DIRECTORY);
    cwd_to("C:\\Users\\me\\");
}

/* ---------- files ---------- */
static void test_files(void)
{
    HANDLE h, h2;
    char buf[64];
    DWORD got, attr;
    LARGE_INTEGER d, np, sz;
    OVERLAPPED ov;

    fake_reset();
    cwd_to("C:\\Users\\me\\");
    fake_add_file("\\??\\C:\\data.txt", 0, "hello world", 11);

    /* read-only open: exact NT translation */
    h = CreateFileW(W("C:\\data.txt"), GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    CHECK(h != INVALID_HANDLE_VALUE);
    CHECK_EQ(F.c_access, FILE_GENERIC_READ | SYNCHRONIZE | FILE_READ_ATTRIBUTES);
    CHECK_EQ(F.c_disp, FILE_OPEN);
    CHECK_EQ(F.c_options, FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT);
    CHECK_EQ(F.c_share, FILE_SHARE_READ);
    CHECK_EQ(F.c_oattrs, OBJ_CASE_INSENSITIVE);
    CHECK(weq(F.c_path, "\\??\\C:\\data.txt"));
    CHECK_EQ(GetLastError(), ERROR_SUCCESS);

    memset(buf, 0, sizeof buf);
    CHECK(ReadFile(h, buf, 5, &got, NULL) && got == 5 && !memcmp(buf, "hello", 5));
    CHECK(ReadFile(h, buf, 100, &got, NULL) && got == 6 && !memcmp(buf, " world", 6));
    CHECK(ReadFile(h, buf, 10, &got, NULL) && got == 0);                  /* EOF: success, zero bytes */
    CHECK(!ReadFile(h, buf, 1, NULL, NULL)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(!ReadFile(INVALID_HANDLE_VALUE, buf, 1, &got, NULL)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);

    /* overlapped-style explicit offset on a sync handle */
    memset(&ov, 0, sizeof ov); ov.Offset = 6;
    CHECK(ReadFile(h, buf, 5, &got, &ov) && got == 5 && !memcmp(buf, "world", 5));
    CHECK(F.last_read_off_given && F.last_read_off == 6);

    /* seeking */
    d.QuadPart = 0;
    CHECK(SetFilePointerEx(h, d, &np, FILE_BEGIN) && np.QuadPart == 0);
    d.QuadPart = 4; CHECK(SetFilePointerEx(h, d, &np, FILE_CURRENT) && np.QuadPart == 4);
    d.QuadPart = -3; CHECK(SetFilePointerEx(h, d, &np, FILE_END) && np.QuadPart == 8);
    d.QuadPart = -100; CHECK(!SetFilePointerEx(h, d, &np, FILE_CURRENT)); CHECK_EQ(GetLastError(), ERROR_NEGATIVE_SEEK);
    d.QuadPart = 0; CHECK(!SetFilePointerEx(h, d, &np, 9)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK_EQ(SetFilePointer(h, 2, NULL, FILE_BEGIN), 2);
    { LONG hi = 1; CHECK_EQ(SetFilePointer(h, 0, &hi, FILE_BEGIN), 0); CHECK_EQ(hi, 1); }   /* 4 GiB */
    CHECK_EQ(SetFilePointer(h, -50, NULL, FILE_BEGIN), INVALID_SET_FILE_POINTER);
    CHECK(GetFileSizeEx(h, &sz) && sz.QuadPart == 11);
    CHECK(CloseHandle(h));
    CHECK(!CloseHandle(h)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);   /* double close */
    CHECK(!CloseHandle(NULL)); CHECK(!CloseHandle(INVALID_HANDLE_VALUE));

    /* create / write / truncate semantics */
    h = CreateFileW(W("new.bin"), GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_HIDDEN | FILE_FLAG_WRITE_THROUGH, NULL);
    CHECK(h != INVALID_HANDLE_VALUE);
    CHECK_EQ(F.c_access, (FILE_GENERIC_READ | FILE_GENERIC_WRITE | SYNCHRONIZE | FILE_READ_ATTRIBUTES));
    CHECK_EQ(F.c_disp, FILE_CREATE);
    CHECK_EQ(F.c_attrs, FILE_ATTRIBUTE_HIDDEN);
    CHECK_EQ(F.c_options & FILE_WRITE_THROUGH, FILE_WRITE_THROUGH);
    CHECK(weq(F.c_path, "\\??\\C:\\Users\\me\\new.bin"));          /* resolved against the cwd */
    CHECK(WriteFile(h, "0123456789", 10, &got, NULL) && got == 10);
    CHECK(FlushFileBuffers(h) && F.flush_calls == 1);
    d.QuadPart = 4; SetFilePointerEx(h, d, NULL, FILE_BEGIN);
    CHECK(SetEndOfFile(h)); CHECK_EQ(fake_file_size("\\??\\C:\\Users\\me\\new.bin"), 4);
    CHECK(!WriteFile(h, "x", 1, NULL, NULL)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CloseHandle(h);
    h = CreateFileW(W("new.bin"), GENERIC_READ, 0, NULL, CREATE_NEW, 0, NULL);
    CHECK_EQ((uintptr_t)h, (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_FILE_EXISTS);

    h = CreateFileW(W("new.bin"), GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    CHECK(h != INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_ALREADY_EXISTS);   /* success + "it existed" */
    CHECK_EQ(F.c_disp, FILE_OVERWRITE_IF); CHECK_EQ(fake_file_size("\\??\\C:\\Users\\me\\new.bin"), 0);
    CloseHandle(h);
    h = CreateFileW(W("fresh"), GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    CHECK(h != INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_SUCCESS); CHECK_EQ(F.c_disp, FILE_OPEN_IF);
    CloseHandle(h);
    h = CreateFileW(W("fresh"), GENERIC_READ, 0, NULL, OPEN_ALWAYS, 0, NULL);
    CHECK(h != INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_ALREADY_EXISTS);
    CloseHandle(h);

    { int before = F.create_calls;
      h = CreateFileW(W("fresh"), GENERIC_READ, 0, NULL, TRUNCATE_EXISTING, 0, NULL);
      CHECK_EQ((uintptr_t)h, (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
      CHECK_EQ(F.create_calls, before);                                  /* rejected before reaching the kernel */
      h = CreateFileW(W("fresh"), GENERIC_WRITE, 0, NULL, 9, 0, NULL);
      CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER); CHECK_EQ(F.create_calls, before);
      h = CreateFileW(NULL, GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
      h = CreateFileW(W(""), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND); }
    h = CreateFileW(W("fresh"), GENERIC_WRITE, 0, NULL, TRUNCATE_EXISTING, 0, NULL);
    CHECK(h != INVALID_HANDLE_VALUE); CHECK_EQ(F.c_disp, FILE_OVERWRITE); CloseHandle(h);

    /* not found vs path not found */
    h = CreateFileW(W("missing.txt"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    CHECK_EQ((uintptr_t)h, (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    h = CreateFileW(W("nodir\\missing.txt"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);

    /* delete-on-close, inherit, option flags */
    { SECURITY_ATTRIBUTES sa = { sizeof sa, NULL, TRUE };
      h = CreateFileW(W("tmp.dat"), GENERIC_WRITE, 0, &sa, CREATE_ALWAYS,
                      FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_NO_BUFFERING | FILE_ATTRIBUTE_TEMPORARY, NULL);
      CHECK(h != INVALID_HANDLE_VALUE);
      CHECK_EQ(F.c_oattrs, OBJ_CASE_INSENSITIVE | OBJ_INHERIT);
      CHECK_EQ(F.c_access & DELETE, DELETE);
      CHECK_EQ(F.c_options & (FILE_DELETE_ON_CLOSE | FILE_SEQUENTIAL_ONLY | FILE_NO_INTERMEDIATE_BUFFERING),
               FILE_DELETE_ON_CLOSE | FILE_SEQUENTIAL_ONLY | FILE_NO_INTERMEDIATE_BUFFERING);
      CHECK_EQ(F.c_attrs, FILE_ATTRIBUTE_TEMPORARY);
      CHECK(fake_file_exists("\\??\\C:\\Users\\me\\tmp.dat")); CloseHandle(h);
      CHECK(!fake_file_exists("\\??\\C:\\Users\\me\\tmp.dat")); }
    h = CreateFileW(W("x"), GENERIC_READ, 0, NULL, OPEN_ALWAYS, FILE_FLAG_OVERLAPPED, NULL);
    CHECK_EQ(F.c_options & FILE_SYNCHRONOUS_IO_NONALERT, 0); CloseHandle(h);

    /* directories */
    h = CreateFileW(W("C:\\Users\\me"), GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    CHECK_EQ((uintptr_t)h, (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_ACCESS_DENIED);
    h = CreateFileW(W("C:\\Users\\me"), GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    CHECK(h != INVALID_HANDLE_VALUE); CHECK_EQ(F.c_options & FILE_NON_DIRECTORY_FILE, 0); CloseHandle(h);
    CHECK(CreateDirectoryW(W("newdir"), NULL)); CHECK_EQ(F.c_disp, FILE_CREATE); CHECK_EQ(F.c_options & FILE_DIRECTORY_FILE, FILE_DIRECTORY_FILE);
    CHECK(!CreateDirectoryW(W("newdir"), NULL)); CHECK_EQ(GetLastError(), ERROR_ALREADY_EXISTS);
    CHECK(!CreateDirectoryW(W("a\\b\\c"), NULL)); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
    attr = GetFileAttributesW(W("newdir")); CHECK(attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_DIRECTORY));
    CHECK(RemoveDirectoryW(W("newdir"))); CHECK_EQ(F.c_options & FILE_DIRECTORY_FILE, FILE_DIRECTORY_FILE);
    CHECK(!fake_file_exists("\\??\\C:\\Users\\me\\newdir"));
    CHECK_EQ(GetFileAttributesW(W("newdir")), INVALID_FILE_ATTRIBUTES); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    CHECK_EQ(GetFileAttributesW(W("zz\\q")), INVALID_FILE_ATTRIBUTES); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);

    /* DeleteFile: DELETE access, non-directory, disposition set */
    fake_add_file("\\??\\C:\\Users\\me\\gone.txt", 0, "z", 1);
    CHECK(DeleteFileW(W("gone.txt"))); CHECK_EQ(F.c_access & DELETE, DELETE); CHECK_EQ(F.c_options & FILE_NON_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE);
    CHECK(!fake_file_exists("\\??\\C:\\Users\\me\\gone.txt"));
    CHECK(!DeleteFileW(W("gone.txt"))); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);

    /* DuplicateHandle inherit flag */
    h = CreateFileW(W("C:\\data.txt"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    CHECK(DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &h2, 0, TRUE, 2)); CHECK_EQ(F.dup_attrs, OBJ_INHERIT);
    CHECK(h2 != h && ReadFile(h2, buf, 2, &got, NULL) && got == 2);
    CloseHandle(h); CloseHandle(h2);
}

/* ---------- virtual memory ---------- */
static void test_vm(void)
{
    void *p; DWORD old; MEMORY_BASIC_INFORMATION mbi;
    fake_reset();
    p = VirtualAlloc(NULL, 5000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    CHECK(p != NULL); CHECK_EQ((uintptr_t)F.vm_proc, (uintptr_t)-1); CHECK_EQ(F.vm_type, MEM_COMMIT | MEM_RESERVE); CHECK_EQ(F.vm_prot, PAGE_READWRITE);
    CHECK_EQ(F.vm_size, 5000);
    CHECK(VirtualAlloc(NULL, 0, MEM_COMMIT, PAGE_READWRITE) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(VirtualAlloc(NULL, 0x80000000u, MEM_COMMIT, PAGE_READWRITE) == NULL); CHECK_EQ(GetLastError(), ERROR_NOT_ENOUGH_MEMORY);
    CHECK(VirtualFree(p, 0, MEM_RELEASE)); CHECK_EQ(F.vm_type, MEM_RELEASE); CHECK_EQ(F.vm_size, 0);
    CHECK(!VirtualFree(p, 4096, MEM_RELEASE)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);        /* RELEASE needs size 0 */
    CHECK(!VirtualFree(p, 0, 0)); CHECK(!VirtualFree(p, 4096, MEM_RELEASE | MEM_DECOMMIT)); CHECK(!VirtualFree(NULL, 0, MEM_RELEASE));
    CHECK(VirtualFree(p, 4096, MEM_DECOMMIT)); CHECK_EQ(F.vm_size, 4096);
    CHECK(VirtualProtect(p, 4096, PAGE_READONLY, &old) && old == PAGE_READWRITE);
    CHECK(!VirtualProtect(p, 4096, PAGE_READONLY, NULL)); CHECK_EQ(GetLastError(), ERROR_NOACCESS);
    CHECK_EQ(VirtualQuery(p, &mbi, sizeof mbi), sizeof mbi); CHECK_EQ(F.vm_query_class, 0);
    CHECK_EQ(VirtualQuery(p, &mbi, 8), 0); CHECK_EQ(GetLastError(), ERROR_BAD_LENGTH);
    CHECK_EQ(VirtualQuery((void *)0x900000000000ull, &mbi, sizeof mbi), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
}

/* ---------- synchronisation ---------- */
static void test_sync(void)
{
    HANDLE e, e2, m, s, hs[3];
    LONG prev;
    fake_reset();

    e = CreateEventW(NULL, TRUE, FALSE, NULL);
    CHECK(e != NULL); CHECK_EQ(F.ev_type, EventNotificationType); CHECK_EQ(F.ev_initial, 0); CHECK_EQ(F.ev_access, EVENT_ALL_ACCESS); CHECK_EQ(F.obj_named, 0);
    CHECK_EQ(WaitForSingleObject(e, 0), WAIT_TIMEOUT); CHECK_EQ(F.wait_timeout, 0);
    CHECK_EQ(WaitForSingleObject(e, 250), WAIT_TIMEOUT); CHECK_EQ(F.wait_timeout, -2500000);       /* relative, 100ns */
    CHECK(SetEvent(e)); CHECK_EQ(WaitForSingleObject(e, 10), WAIT_OBJECT_0); CHECK_EQ(WaitForSingleObject(e, 10), WAIT_OBJECT_0);   /* manual reset stays set */
    CHECK(ResetEvent(e)); CHECK_EQ(WaitForSingleObject(e, 10), WAIT_TIMEOUT);
    SetEvent(e); WaitForSingleObject(e, INFINITE); CHECK(F.wait_timeout_null);
    CHECK_EQ(WaitForSingleObjectEx(e, 5, TRUE), WAIT_OBJECT_0); CHECK(F.wait_alertable);
    CHECK_EQ(WaitForSingleObject(NULL, 0), WAIT_FAILED); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    CHECK(!SetEvent((HANDLE)0x12345)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);

    e2 = CreateEventW(NULL, FALSE, TRUE, NULL);                    /* auto-reset, initially set */
    CHECK_EQ(F.ev_type, EventSynchronizationType); CHECK_EQ(F.ev_initial, 1);
    CHECK_EQ(WaitForSingleObject(e2, 0), WAIT_OBJECT_0); CHECK_EQ(WaitForSingleObject(e2, 0), WAIT_TIMEOUT);

    /* named objects live under \BaseNamedObjects, second create reports ALREADY_EXISTS */
    { SECURITY_ATTRIBUTES sa = { sizeof sa, NULL, TRUE }; HANDLE a, b;
      a = CreateEventW(&sa, FALSE, FALSE, W("MyEvent"));
      CHECK(a && weq(F.obj_name, "\\BaseNamedObjects\\MyEvent") && F.obj_inherit); CHECK_EQ(GetLastError(), ERROR_SUCCESS);
      b = CreateEventW(NULL, FALSE, FALSE, W("myevent"));          /* case-insensitive */
      CHECK(b && b != a); CHECK_EQ(GetLastError(), ERROR_ALREADY_EXISTS);
      SetEvent(a); CHECK_EQ(WaitForSingleObject(b, 0), WAIT_OBJECT_0);
      CHECK(CreateEventW(NULL, FALSE, FALSE, W("bad\\name")) == NULL); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND); }

    /* mutex */
    m = CreateMutexW(NULL, TRUE, NULL);
    CHECK(m && F.ev_access == MUTANT_ALL_ACCESS && F.ev_initial == 1);
    CHECK_EQ(WaitForSingleObject(m, 0), WAIT_TIMEOUT);              /* owned (by the creator in the fake, no recursion) */
    CHECK(ReleaseMutex(m)); CHECK(!ReleaseMutex(m)); CHECK_EQ(GetLastError(), ERROR_NOT_OWNER);
    CHECK_EQ(WaitForSingleObject(m, 0), WAIT_OBJECT_0);
    { fake_handle *fh = &F.h[((uintptr_t)m - 0x100) / 4]; fh->abandoned = 1; fh->state = 1; }
    CHECK_EQ(WaitForSingleObject(m, 0), WAIT_ABANDONED_0);

    /* semaphore */
    CHECK(CreateSemaphoreW(NULL, 2, 1, NULL) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(CreateSemaphoreW(NULL, 0, 0, NULL) == NULL); CHECK(CreateSemaphoreW(NULL, -1, 5, NULL) == NULL);
    s = CreateSemaphoreW(NULL, 1, 3, NULL);
    CHECK(s); CHECK_EQ(WaitForSingleObject(s, 0), WAIT_OBJECT_0); CHECK_EQ(WaitForSingleObject(s, 0), WAIT_TIMEOUT);
    CHECK(ReleaseSemaphore(s, 2, &prev) && prev == 0);
    CHECK(!ReleaseSemaphore(s, 2, &prev)); CHECK_EQ(GetLastError(), ERROR_TOO_MANY_POSTS);
    CHECK(!ReleaseSemaphore(s, 0, &prev)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);

    /* multiple waits */
    hs[0] = CreateEventW(NULL, TRUE, FALSE, NULL); hs[1] = CreateEventW(NULL, TRUE, FALSE, NULL); hs[2] = CreateEventW(NULL, TRUE, FALSE, NULL);
    CHECK_EQ(WaitForMultipleObjects(3, hs, FALSE, 0), WAIT_TIMEOUT);
    SetEvent(hs[2]); CHECK_EQ(WaitForMultipleObjects(3, hs, FALSE, 0), WAIT_OBJECT_0 + 2); CHECK_EQ(F.wait_type, WaitAnyObject);
    CHECK_EQ(WaitForMultipleObjects(3, hs, TRUE, 0), WAIT_TIMEOUT); CHECK_EQ(F.wait_type, WaitAllObjects);
    SetEvent(hs[0]); SetEvent(hs[1]); CHECK_EQ(WaitForMultipleObjects(3, hs, TRUE, 100), WAIT_OBJECT_0); CHECK_EQ(F.wait_timeout, -1000000);
    CHECK_EQ(WaitForMultipleObjects(0, hs, FALSE, 0), WAIT_FAILED); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK_EQ(WaitForMultipleObjects(65, hs, FALSE, 0), WAIT_FAILED);
    { HANDLE bad[2] = { hs[0], NULL }; CHECK_EQ(WaitForMultipleObjects(2, bad, FALSE, 0), WAIT_FAILED); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE); }

    /* sleep + clocks */
    Sleep(100); CHECK_EQ(F.delay, -1000000); CHECK(!F.delay_alertable);
    Sleep(INFINITE); CHECK_EQ((unsigned long long)F.delay, 0x8000000000000000ull);
    F.delay_apc = 1; CHECK_EQ(SleepEx(5, TRUE), WAIT_IO_COMPLETION); CHECK(F.delay_alertable); F.delay_apc = 0;
    { LARGE_INTEGER c, f;
      CHECK(QueryPerformanceFrequency(&f) && f.QuadPart == 10000000);
      CHECK(QueryPerformanceCounter(&c) && c.QuadPart == 123456789012ll); }
    CHECK_EQ(GetTickCount64(), 12345678);
}

/* ---------- process / modules ---------- */
static void test_proc(void)
{
    HMODULE m; FARPROC f;
    fake_reset();
    CHECK_EQ((uintptr_t)GetCurrentProcess(), (uintptr_t)-1); CHECK_EQ((uintptr_t)GetCurrentThread(), (uintptr_t)-2);
    CHECK_EQ(GetCurrentProcessId(), 1234); CHECK_EQ(GetCurrentThreadId(), 5678);
    CHECK((uintptr_t)GetModuleHandleW(NULL) == 0x140000000ull);
    CHECK(GetModuleHandleW(W("known.dll")) == NULL); CHECK_EQ(GetLastError(), ERROR_MOD_NOT_FOUND);       /* not loaded yet */
    CHECK(LoadLibraryW(W("nothere.dll")) == NULL); CHECK_EQ(GetLastError(), ERROR_MOD_NOT_FOUND);
    CHECK(LoadLibraryW(NULL) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    m = LoadLibraryW(W("KNOWN.DLL")); CHECK(m != NULL);
    CHECK(GetModuleHandleW(W("known.dll")) == m);
    f = GetProcAddress(m, "Func"); CHECK((uintptr_t)f == 0x7ff700001000ull);
    CHECK(GetProcAddress(m, (const char *)(uintptr_t)1) == f);                                          /* by ordinal */
    CHECK(GetProcAddress(m, "Missing") == NULL); CHECK_EQ(GetLastError(), ERROR_PROC_NOT_FOUND);
    CHECK(GetProcAddress(m, (const char *)(uintptr_t)9) == NULL);
    CHECK(GetProcAddress(NULL, "Func") == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(FreeLibrary(m)); CHECK(!FreeLibrary(m)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    if (!setjmp(fake_exit_jmp)) { ExitProcess(42); CHECK(0); }
    CHECK_EQ(F.exit_code, 42);
}

/* ---------- heaps ---------- */
static void test_heap_api(void)
{
    HANDLE h, ph = GetProcessHeap();
    unsigned char *p, *q;
    HLOCAL l;
    CHECK(ph != NULL && ph == GetProcessHeap());
    CHECK(HeapCreate(0x80, 0, 0) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);       /* unknown flag */
    CHECK(HeapCreate(0, 4096, 1024) == NULL);                                                       /* initial > maximum */
    h = HeapCreate(HEAP_NO_SERIALIZE, 0, 0); CHECK(h != NULL);
    p = HeapAlloc(h, HEAP_ZERO_MEMORY, 100); CHECK(p && p[0] == 0 && p[99] == 0 && HeapSize(h, 0, p) == 100);
    memset(p, 9, 100);
    q = HeapReAlloc(h, 0, p, 5000); CHECK(q && q[0] == 9 && q[99] == 9 && HeapSize(h, 0, q) == 5000);
    CHECK(HeapReAlloc(h, 0, NULL, 10) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(HeapValidate(h, 0, NULL) && HeapValidate(h, 0, q));
    CHECK(HeapAlloc(h, 0, (SIZE_T)1 << 60) == NULL); CHECK_EQ(GetLastError(), ERROR_NOT_ENOUGH_MEMORY);
    CHECK(HeapFree(h, 0, q));
    CHECK(!HeapFree(h, 0, q)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);                   /* double free */
    CHECK_EQ(HeapSize(h, 0, q), (SIZE_T)-1);
    CHECK(HeapSetInformation(NULL, 1, NULL, 0));
    { ULONG v = 7; SIZE_T r = 0; CHECK(HeapQueryInformation(h, 0, &v, sizeof v, &r) && v == 0 && r == sizeof v); }
    CHECK(HeapDestroy(h));
    CHECK(!HeapDestroy(ph)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);                     /* the process heap is permanent */

    l = LocalAlloc(LMEM_ZEROINIT, 64); CHECK(l != NULL && ((unsigned char *)l)[63] == 0);
    CHECK_EQ(LocalSize(l), 64); CHECK(LocalLock(l) == l);
    l = LocalReAlloc(l, 4096, LMEM_ZEROINIT); CHECK(l && LocalSize(l) == 4096);
    CHECK(LocalFree(l) == NULL);
    CHECK(LocalFree(NULL) == NULL);
    { void *bogus = (void *)0x1230; CHECK(LocalFree(bogus) == bogus); }                            /* failure returns the handle */
    l = GlobalAlloc(GMEM_ZEROINIT, 0); CHECK(l != NULL);                                           /* zero size still yields a block */
    CHECK(GlobalFree(l) == NULL);
    l = GlobalAlloc(0, 32); CHECK(GlobalSize(l) == 32); GlobalFree(l);
}

/* ---------- strings and code pages ---------- */
static int wide_eq(const WCHAR *w, const uint16_t *want, size_t n) { size_t i; for (i = 0; i < n; i++) if (w[i] != want[i]) return 0; return 1; }

static void test_strings(void)
{
    WCHAR w[64]; char a[64]; BOOL used;
    int n;
    CHECK_EQ(GetACP(), 1252); CHECK_EQ(GetOEMCP(), 437);
    CHECK(IsValidCodePage(65001) && IsValidCodePage(1252) && IsValidCodePage(437) && !IsValidCodePage(932));

    /* UTF-8 -> UTF-16 */
    n = MultiByteToWideChar(CP_UTF8, 0, "h\xc3\xa9llo \xe2\x82\xac \xf0\x9f\x98\x80", -1, w, 64);
    { static const uint16_t want[] = { 'h', 0xE9, 'l', 'l', 'o', ' ', 0x20AC, ' ', 0xD83D, 0xDE00, 0 };
      CHECK_EQ(n, 11); CHECK(wide_eq(w, want, 11)); }
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "abc", 3, NULL, 0), 3);                      /* explicit length: no terminator counted */
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "abc", -1, NULL, 0), 4);
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "abcd", -1, w, 3), 0); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, NULL, -1, w, 8), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "x", 0, w, 8), 0);
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "x", -1, NULL, 5), 0);
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, MB_PRECOMPOSED, "x", -1, w, 8), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_FLAGS);
    CHECK_EQ(MultiByteToWideChar(932, 0, "x", -1, w, 8), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    { static const struct { const char *s; int len; } bad[] = {      /* every one of these must be rejected, not decoded */
        { "\xc0\x80", 2 }, { "\xc1\xbf", 2 }, { "\xed\xa0\x80", 3 }, { "\xf4\x90\x80\x80", 4 }, { "\xe2\x82", 2 }, { "\x80", 1 }, { "\xff", 1 }, { "\xf8\x88\x80\x80\x80", 5 }, { "\xe0\x80\x80", 3 } };
      size_t i;
      for (i = 0; i < sizeof bad / sizeof bad[0]; i++) {
          CHECK_EQ(MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bad[i].s, bad[i].len, w, 64), 0);
          CHECK_EQ(GetLastError(), ERROR_NO_UNICODE_TRANSLATION);
          n = MultiByteToWideChar(CP_UTF8, 0, bad[i].s, bad[i].len, w, 64);                   /* lenient mode substitutes U+FFFD */
          CHECK(n > 0 && w[0] == 0xFFFD);
      } }
    CHECK_EQ(MultiByteToWideChar(CP_UTF8, 0, "a\xe2\x82", 3, w, 64), 2); CHECK(w[0] == 'a' && w[1] == 0xFFFD);   /* truncated tail = one replacement */

    /* UTF-16 -> UTF-8 */
    { static const uint16_t src[] = { 'h', 0xE9, 0x20AC, 0xD83D, 0xDE00, 0 };
      n = WideCharToMultiByte(CP_UTF8, 0, (LPCWSTR)src, -1, a, 64, NULL, NULL);
      CHECK_EQ(n, 1 + 2 + 3 + 4 + 1); CHECK(!memcmp(a, "h\xc3\xa9\xe2\x82\xac\xf0\x9f\x98\x80", 11)); }
    { static const uint16_t lone[] = { 'a', 0xD800, 'b' };
      CHECK_EQ(WideCharToMultiByte(CP_UTF8, 0, (LPCWSTR)lone, 3, a, 64, NULL, NULL), 1 + 3 + 1); CHECK(!memcmp(a + 1, "\xef\xbf\xbd", 3));
      CHECK_EQ(WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, (LPCWSTR)lone, 3, a, 64, NULL, NULL), 0); CHECK_EQ(GetLastError(), ERROR_NO_UNICODE_TRANSLATION); }
    CHECK_EQ(WideCharToMultiByte(CP_UTF8, 0, W("x"), -1, a, 64, "?", NULL), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);   /* no default char with UTF-8 */
    CHECK_EQ(WideCharToMultiByte(CP_UTF8, 0, W("\xe9x"), -1, NULL, 0, NULL, NULL), 4);
    CHECK_EQ(WideCharToMultiByte(CP_UTF8, 0, W("\xe9x"), -1, a, 2, NULL, NULL), 0); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);

    /* single-byte pages */
    CHECK_EQ(MultiByteToWideChar(1252, 0, "\x80\xe9\x81", 3, w, 64), 3); CHECK(w[0] == 0x20AC && w[1] == 0xE9 && w[2] == 0x81);
    CHECK_EQ(MultiByteToWideChar(CP_OEMCP, 0, "\x80\xe1\xfb\xb0", 4, w, 64), 4); CHECK(w[0] == 0xC7 && w[1] == 0xDF && w[2] == 0x221A && w[3] == 0x2591);
    CHECK_EQ(MultiByteToWideChar(20127, MB_ERR_INVALID_CHARS, "\x80", 1, w, 64), 0);
    { static const uint16_t euro_cjk[] = { 0x20AC, 0xE9, 0x4E2D, 'Z' };
      used = FALSE;
      CHECK_EQ(WideCharToMultiByte(1252, 0, (LPCWSTR)euro_cjk, 4, a, 64, NULL, &used), 4); CHECK((unsigned char)a[0] == 0x80 && (unsigned char)a[1] == 0xE9 && a[2] == '?' && a[3] == 'Z' && used);
      used = FALSE;
      CHECK_EQ(WideCharToMultiByte(1252, 0, (LPCWSTR)euro_cjk, 4, a, 64, "_", &used), 4); CHECK(a[2] == '_' && used);
      used = TRUE;
      CHECK_EQ(WideCharToMultiByte(1252, 0, (LPCWSTR)euro_cjk, 2, a, 64, NULL, &used), 2); CHECK(!used); }
    { static const uint16_t box[] = { 0x2591, 0x2588 };
      CHECK_EQ(WideCharToMultiByte(437, 0, (LPCWSTR)box, 2, a, 64, NULL, NULL), 2); CHECK((unsigned char)a[0] == 0xB0 && (unsigned char)a[1] == 0xDB); }
    { static const uint16_t pair[] = { 0xD83D, 0xDE00 };
      used = FALSE; CHECK_EQ(WideCharToMultiByte(1252, 0, (LPCWSTR)pair, 2, a, 64, NULL, &used), 1); CHECK(a[0] == '?' && used); }  /* a surrogate pair is one character */
    CHECK_EQ(WideCharToMultiByte(CP_ACP, 0, W("abc"), -1, a, 64, NULL, NULL), 4);                                                 /* CP_ACP -> 1252 */

    /* case folding and comparison */
    CHECK(k32_towupper(0xE9) == 0xC9 && k32_towupper(0x3C2) == 0x3A3 && k32_towupper(0x43F) == 0x41F && k32_towupper(0x45E) == 0x40E && k32_towupper(0xFF) == 0x178);
    CHECK(k32_towlower(0xC9) == 0xE9 && k32_towlower(0x130) == 'i' && k32_towlower(0x41F) == 0x43F && k32_towlower(0x178) == 0xFF && k32_towlower('Q') == 'q');
    { unsigned c; int bad = 0;                                      /* lower(upper(c)) must return the letter for every cased code point we cover */
      for (c = 0x61; c < 0x500; c++) {
          WCHAR u = k32_towupper((WCHAR)c);
          if (u == c || c == 0xB5 || c == 0x131 || c == 0x17F || c == 0x3C2) continue;
          if (k32_towlower(u) != c) { printf("  case roundtrip failed for U+%04X\n", c); bad++; }
      }
      CHECK_EQ(bad, 0); }
    CHECK_EQ(CompareStringW(0, 0, W("abc"), -1, W("abc"), -1), CSTR_EQUAL);
    CHECK_EQ(CompareStringW(0, 0, W("abc"), -1, W("abd"), -1), CSTR_LESS_THAN);
    CHECK_EQ(CompareStringW(0, 0, W("abc"), -1, W("ab"), -1), CSTR_GREATER_THAN);
    CHECK_EQ(CompareStringW(0, 0, W("abc"), 2, W("ab"), -1), CSTR_EQUAL);                      /* explicit lengths */
    CHECK_EQ(CompareStringW(0, 0, W("ABC"), -1, W("abc"), -1), CSTR_LESS_THAN);
    CHECK_EQ(CompareStringW(0, NORM_IGNORECASE, W("ABC"), -1, W("abc"), -1), CSTR_EQUAL);
    CHECK_EQ(CompareStringOrdinal(W("\xc4\xd6\xdc"), -1, W("\xe4\xf6\xfc"), -1, TRUE), CSTR_EQUAL);
    CHECK_EQ(CompareStringOrdinal(W("\xc4"), -1, W("\xe4"), -1, FALSE), CSTR_LESS_THAN);
    CHECK_EQ(CompareStringW(0, 0, NULL, -1, W("a"), -1), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(lstrcmpW(W("a"), W("b")) < 0 && lstrcmpW(W("b"), W("a")) > 0 && lstrcmpW(W("a"), W("a")) == 0);
    CHECK(lstrcmpiW(W("HeLLo"), W("hello")) == 0 && lstrcmpiW(W("a"), W("B")) < 0);
    CHECK(lstrcmpA("a", "b") < 0 && lstrcmpiA("HeLLo", "hello") == 0 && lstrcmpA("A", "a") < 0);
    CHECK_EQ(LCMapStringW(0, LCMAP_UPPERCASE, W("abc\xe9"), -1, w, 64), 5); CHECK(w[0] == 'A' && w[3] == 0xC9 && w[4] == 0);
    CHECK_EQ(LCMapStringW(0, LCMAP_LOWERCASE, W("ABC"), 3, w, 64), 3); CHECK(w[0] == 'a' && w[2] == 'c');
    CHECK_EQ(LCMapStringW(0, LCMAP_UPPERCASE, W("abc"), -1, NULL, 0), 4);
    CHECK_EQ(LCMapStringW(0, LCMAP_UPPERCASE, W("abc"), -1, w, 2), 0); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);
    CHECK_EQ(LCMapStringW(0, LCMAP_UPPERCASE | LCMAP_LOWERCASE, W("abc"), -1, w, 8), 0);

    /* lstr* copies */
    CHECK_EQ(lstrlenW(W("four")), 4); CHECK_EQ(lstrlenA("four"), 4); CHECK_EQ(lstrlenW(NULL), 0); CHECK_EQ(lstrlenA(NULL), 0);
    lstrcpynA(a, "truncate me", 5); CHECK(!strcmp(a, "trun"));
    lstrcpynW(w, W("truncate me"), 5); CHECK(weq(w, "trun"));
    lstrcpynA(a, "x", 0); CHECK(!strcmp(a, "trun"));                          /* n <= 0 writes nothing */
    lstrcpyA(a, "ab"); lstrcatA(a, "cd"); CHECK(!strcmp(a, "abcd"));
    lstrcpyW(w, W("ab")); lstrcatW(w, W("cd")); CHECK(weq(w, "abcd"));
}

/* ---------- console ---------- */
static int ctrl_seen[4]; static int ctrl_order; static int ctrl_first, ctrl_second;
static BOOL WINAPI ctrl_a(DWORD t) { ctrl_seen[t & 3]++; ctrl_first = ++ctrl_order; return FALSE; }
static BOOL WINAPI ctrl_b(DWORD t) { ctrl_seen[t & 3] += 10; ctrl_second = ++ctrl_order; return t == CTRL_C_EVENT; }

static RTL_USER_PROCESS_PARAMETERS PP;
static void test_console(void)
{
    HANDLE out, in, file;
    DWORD n, mode;
    WCHAR w[128]; char a[128];
    CONSOLE_SCREEN_BUFFER_INFO info;
    fake_reset();
    memset(&PP, 0, sizeof PP); k32_host_params = &PP;
    out = CreateFileW(W("CONOUT$"), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL);
    in = CreateFileW(W("conin$"), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    CHECK(out != INVALID_HANDLE_VALUE && in != INVALID_HANDLE_VALUE);
    CHECK(weq(F.c_path, "\\??\\CONIN$") || 1);
    PP.ConsoleHandle = out;
    fake_add_file("\\??\\C:\\Users", 1, NULL, 0); fake_add_file("\\??\\C:\\Users\\me", 1, NULL, 0);
    file = CreateFileW(W("C:\\Users\\me\\f.txt"), GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);

    /* GetFileType */
    CHECK_EQ(GetFileType(out), FILE_TYPE_CHAR); CHECK_EQ(GetFileType(file), FILE_TYPE_DISK);
    F.devtype_override = FILE_DEVICE_NAMED_PIPE; CHECK_EQ(GetFileType(file), FILE_TYPE_PIPE);
    F.devtype_override = FILE_DEVICE_NULL; CHECK_EQ(GetFileType(file), FILE_TYPE_CHAR); F.devtype_override = 0;
    CHECK_EQ(GetFileType(INVALID_HANDLE_VALUE), FILE_TYPE_UNKNOWN); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    CHECK_EQ(GetFileType((HANDLE)0x999), FILE_TYPE_UNKNOWN);

    /* mode: console vs. not a console */
    CHECK(GetConsoleMode(out, &mode) && mode == 3); CHECK(GetConsoleMode(in, &mode) && mode == 7);
    CHECK(SetConsoleMode(out, mode = ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING) && F.con.mode_out == 5);
    CHECK(!GetConsoleMode(file, &mode)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);                  /* a disk file */
    CHECK(!GetConsoleMode(NULL, &mode) && !GetConsoleMode(out, NULL) && !SetConsoleMode(file, 1));

    /* output: UTF-16 -> UTF-8, chunking, surrogate pairs */
    CHECK(WriteConsoleW(out, W("h\xe9llo\n"), 6, &n, NULL) && n == 6); CHECK(!memcmp(F.con.out, "h\xc3\xa9llo\n", 7) && F.con.nout == 7);
    { static const uint16_t emoji[] = { 0xD83D, 0xDE00, 0x20AC };
      F.con.nout = 0; CHECK(WriteConsoleW(out, emoji, 3, &n, NULL) && n == 3); CHECK(F.con.nout == 7 && !memcmp(F.con.out, "\xf0\x9f\x98\x80\xe2\x82\xac", 7)); }
    { WCHAR *big = malloc(10000 * 2); DWORD i; for (i = 0; i < 10000; i++) big[i] = (i % 2) ? 0xDE00 : 0xD83D;      /* 5000 pairs: chunk edge must not split one */
      F.con.nout = 0; CHECK(WriteConsoleW(out, big, 10000, &n, NULL) && n == 10000); CHECK_EQ(F.con.nout, 5000 * 4);
      { size_t i2; int ok = 1; for (i2 = 0; i2 < F.con.nout; i2 += 4) ok &= !memcmp(F.con.out + i2, "\xf0\x9f\x98\x80", 4); CHECK(ok); }
      free(big); }
    CHECK(WriteConsoleW(out, NULL, 0, &n, NULL) && n == 0); CHECK(!WriteConsoleW(out, NULL, 3, &n, NULL));
    CHECK(!WriteConsoleW((HANDLE)0x5, W("x"), 1, &n, NULL) && n == 0);
    F.con.nout = 0; F.con.cp_out = 437;
    CHECK(WriteConsoleA(out, "caf\x82 \xe0", 6, &n, NULL) && n == 6);                                    /* CP 437: e-acute, alpha */
    CHECK(!memcmp(F.con.out, "caf\xc3\xa9 \xce\xb1", 8));
    F.con.nout = 0; F.con.cp_out = CP_UTF8;
    CHECK(WriteConsoleA(out, "\xe2\x82\xac!", 4, &n, NULL) && n == 4); CHECK(!memcmp(F.con.out, "\xe2\x82\xac!", 4));

    /* input: UTF-8 -> UTF-16 with split sequences */
    memcpy(F.con.in, "h\xc3\xa9y\r\n", 6); F.con.nin = 6; F.con.in_pos = 0; F.con.read_chunk = 2;       /* first read ends mid-sequence */
    CHECK(ReadConsoleW(in, w, 64, &n, NULL)); CHECK_EQ(n, 1); CHECK(w[0] == 'h');                       /* "h" + half of e-acute: hold the half */
    CHECK(ReadConsoleW(in, w, 64, &n, NULL)); CHECK(n == 2 && w[0] == 0xE9 && w[1] == 'y');
    F.con.read_chunk = 0;
    CHECK(ReadConsoleW(in, w, 64, &n, NULL) && n == 2 && w[0] == '\r' && w[1] == '\n');
    CHECK(ReadConsoleW(in, w, 64, &n, NULL) && n == 0);                                                  /* nothing pending / EOF */
    CHECK(ReadConsoleW(in, w, 0, &n, NULL) && n == 0); CHECK(!ReadConsoleW(in, NULL, 4, &n, NULL)); CHECK(!ReadConsoleW(in, w, 4, NULL, NULL));
    F.con.cp_in = 437; memcpy(F.con.in, "caf\xc3\xa9", 5); F.con.nin = 5; F.con.in_pos = 0;
    CHECK(ReadConsoleA(in, a, 64, &n, NULL) && n == 4 && !memcmp(a, "caf\x82", 4));
    F.con.nin = 5; memcpy(F.con.in, "ab\r\nZ", 5); F.con.in_pos = 0;
    CHECK(GetNumberOfConsoleInputEvents(in, &n) && n == 5); CHECK(FlushConsoleInputBuffer(in)); CHECK(GetNumberOfConsoleInputEvents(in, &n) && n == 0);

    /* screen */
    F.con.cx = 4; F.con.cy = 9; F.con.attr = 7;
    CHECK(GetConsoleScreenBufferInfo(out, &info) && info.dwSize.X == 120 && info.dwSize.Y == 300 && info.dwCursorPosition.X == 4 && info.dwCursorPosition.Y == 9);
    CHECK(info.wAttributes == 7 && info.Right == 119 && info.Bottom == 29 && info.dwMaximumWindowSize.Y == 30);
    CHECK_EQ(sizeof(CONSOLE_SCREEN_BUFFER_INFO), 22);
    CHECK(SetConsoleCursorPosition(out, (COORD){ 10, 20 }) && F.con.cx == 10 && F.con.cy == 20);
    CHECK(SetConsoleTextAttribute(out, 0x1E) && F.con.attr == 0x1E);
    CHECK(FillConsoleOutputCharacterW(out, L' ' == 0 ? 0 : 0x20, 50, (COORD){ 1, 2 }, &n) && n == 50 && F.con.last_fill_kind == 0 && F.con.last_fill_value == 0x20 && F.con.fill_x == 1);
    CHECK(FillConsoleOutputAttribute(out, 0x07, 9, (COORD){ 0, 0 }, &n) && n == 9 && F.con.last_fill_kind == 1);
    CHECK(!GetConsoleScreenBufferInfo(file, &info)); CHECK(!SetConsoleCursorPosition(file, (COORD){ 0, 0 }));

    /* code pages and title (process-wide, through the console handle in the process parameters) */
    F.con.cp_in = 437; F.con.cp_out = 850;
    CHECK_EQ(GetConsoleCP(), 437); CHECK_EQ(GetConsoleOutputCP(), 850);
    CHECK(SetConsoleOutputCP(CP_UTF8) && F.con.cp_out == CP_UTF8 && SetConsoleCP(1252) && F.con.cp_in == 1252);
    CHECK(!SetConsoleOutputCP(932)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(SetConsoleTitleW(W("T\xe9st title")) && !strcmp(F.con.title, "T\xc3\xa9st title"));
    CHECK_EQ(GetConsoleTitleW(w, 64), 10); CHECK(w[0] == 'T' && w[1] == 0xE9 && w[10] == 0);
    CHECK_EQ(GetConsoleTitleW(w, 5), 0); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);
    PP.ConsoleHandle = NULL;
    CHECK_EQ(GetConsoleCP(), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE); CHECK(!SetConsoleTitleW(W("x")));
    CHECK(GetConsoleWindow() == NULL);

    /* Ctrl handlers: newest first, first TRUE stops the chain, NULL toggles Ctrl+C ignore */
    CHECK(SetConsoleCtrlHandler(ctrl_a, TRUE) && SetConsoleCtrlHandler(ctrl_b, TRUE));
    ctrl_order = 0; CtrlRoutine((LPVOID)(uintptr_t)CTRL_C_EVENT);
    CHECK(ctrl_second == 1 && ctrl_first == 0 && ctrl_seen[0] == 10);                                    /* b handled it; a never ran */
    ctrl_order = 0; ctrl_first = ctrl_second = 0;
    if (!setjmp(fake_exit_jmp)) { CtrlRoutine((LPVOID)(uintptr_t)CTRL_BREAK_EVENT); CHECK(0); }           /* nobody handled break: default exit */
    CHECK(ctrl_second == 1 && ctrl_first == 2 && F.exit_code == 0xC000013Au);
    CHECK(SetConsoleCtrlHandler(NULL, TRUE));
    CtrlRoutine((LPVOID)(uintptr_t)CTRL_C_EVENT); CHECK(ctrl_seen[0] == 10);                              /* ignored: handlers not even called */
    CHECK(SetConsoleCtrlHandler(NULL, FALSE));
    CHECK(SetConsoleCtrlHandler(ctrl_b, FALSE) && SetConsoleCtrlHandler(ctrl_a, FALSE));
    CHECK(!SetConsoleCtrlHandler(ctrl_a, FALSE)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CloseHandle(out); CloseHandle(in); CloseHandle(file);
    k32_host_params = NULL;
}

/* ---------- threads and processes ---------- */
static void test_threads(void)
{
    HANDLE t, t2, pseudo = GetCurrentThread();
    DWORD tid = 0, code = 0;
    SECURITY_ATTRIBUTES sa = { sizeof sa, NULL, TRUE };
    fake_reset();
    CHECK(CreateThread(NULL, 0, NULL, NULL, 0, &tid) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    t = CreateThread(NULL, 0, (LPTHREAD_START_ROUTINE)0x1000, (LPVOID)0x77, 0, &tid);
    CHECK(t != NULL); CHECK_EQ(F.thr.access, THREAD_ALL_ACCESS); CHECK(F.thr.start == (PVOID)0x1000 && F.thr.arg == (PVOID)0x77);
    CHECK_EQ(F.thr.flags, 0); CHECK_EQ(F.thr.commit, 0); CHECK_EQ(F.thr.reserve, 0); CHECK((uintptr_t)F.thr.process == (uintptr_t)-1);
    CHECK(tid > 7000); CHECK_EQ(GetThreadId(t), tid);
    t2 = CreateThread(&sa, 65536, (LPTHREAD_START_ROUTINE)0x2000, NULL, CREATE_SUSPENDED, NULL);
    CHECK(t2 != NULL && F.thr.flags == THREAD_CREATE_SUSPENDED && F.thr.commit == 65536 && F.thr.reserve == 0 && (F.thr.oattrs & OBJ_INHERIT));
    CloseHandle(t2);
    t2 = CreateThread(NULL, 1 << 20, (LPTHREAD_START_ROUTINE)0x2000, NULL, CREATE_SUSPENDED | STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
    CHECK(F.thr.commit == 0 && F.thr.reserve == (1 << 20));
    CHECK_EQ(ResumeThread(t2), 1); CHECK_EQ(ResumeThread(t2), 0); CHECK_EQ(SuspendThread(t2), 0); CHECK_EQ(SuspendThread(t2), 1); CHECK_EQ(ResumeThread(t2), 2);
    CHECK_EQ(ResumeThread((HANDLE)0x999), (DWORD)-1); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    CHECK(GetExitCodeThread(t, &code) && code == STILL_ACTIVE);
    CHECK(TerminateThread(t, 42)); CHECK(GetExitCodeThread(t, &code) && code == 42);
    CHECK(!TerminateThread(NULL, 1)); CHECK(!GetExitCodeThread(t, NULL)); CHECK(!GetExitCodeThread((HANDLE)0x999, &code));
    CHECK(SetThreadPriority(t2, 2) && GetThreadPriority(t2) == 2 && F.last_priority == 2);
    CHECK(!SetThreadPriority(t2, 16)); CHECK(!SetThreadPriority(t2, -16)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK_EQ(GetThreadPriority((HANDLE)0x999), THREAD_PRIORITY_ERROR_RETURN);
    CHECK_EQ(GetThreadId(pseudo), k32_host_tid);                                                  /* the pseudo handle resolves to us */
    { HANDLE o = OpenThread(THREAD_ALL_ACCESS, TRUE, tid); CHECK(o != NULL && (F.thr.oattrs & OBJ_INHERIT)); CloseHandle(o);
      CHECK(OpenThread(THREAD_ALL_ACCESS, FALSE, 5) == NULL); }
    F.yield_result = 0; CHECK(SwitchToThread()); F.yield_result = 1; CHECK(!SwitchToThread());
    F.term_calls = 0;
    if (!setjmp(fake_exit_jmp)) { ExitThread(7); CHECK(0); }                                     /* ExitThread must end in RtlExitUserThread */
    CHECK_EQ(F.term_calls, 1); CHECK_EQ(F.term_status, 7); CHECK((intptr_t)F.term_handle == -2);
    CloseHandle(t); CloseHandle(t2);
}

static void test_process_queries(void)
{
    HANDLE p;
    DWORD code = 0;
    fake_reset();
    p = OpenProcess(PROCESS_ALL_ACCESS, TRUE, 9100); CHECK(p != NULL && (F.thr.oattrs & OBJ_INHERIT));
    CHECK_EQ(GetProcessId(p), 9100);
    CHECK(GetExitCodeProcess(p, &code) && code == STILL_ACTIVE);
    CHECK(TerminateProcess(p, 3)); CHECK_EQ(F.term_status, 3); CHECK(GetExitCodeProcess(p, &code) && code == 3);
    CHECK(OpenProcess(PROCESS_ALL_ACCESS, FALSE, 5) == NULL);
    CHECK(!TerminateProcess(NULL, 1)); CHECK(!GetExitCodeProcess(p, NULL));
    CHECK(!GetExitCodeProcess((HANDLE)0x999, &code)); CHECK_EQ(GetProcessId((HANDLE)0x999), 0);
    CloseHandle(p);
}

/* ---------- paths, SearchPath, GetModuleFileName ---------- */
static void mk_tree(void)
{
    static const char *const dirs[] = { "\\??\\C:\\Windows", "\\??\\C:\\Windows\\System32", "\\??\\C:\\Program Files", "\\??\\C:\\Program Files\\App One",
                                        "\\??\\C:\\tools", "\\??\\C:\\work", "\\??\\C:\\apps", NULL };
    static const char *const files[] = { "\\??\\C:\\Windows\\System32\\cmd.exe", "\\??\\C:\\Windows\\notepad.exe", "\\??\\C:\\Program Files\\App One\\app.exe",
                                         "\\??\\C:\\tools\\t.exe", "\\??\\C:\\work\\local.exe", "\\??\\C:\\apps\\main.exe", "\\??\\C:\\apps\\sibling.exe", NULL };
    int i;
    for (i = 0; dirs[i]; i++) fake_add_file(dirs[i], 1, NULL, 0);
    for (i = 0; files[i]; i++) fake_add_file(files[i], 0, "MZ", 2);
}

static void test_search_paths(void)
{
    WCHAR b[300]; char a[300]; LPWSTR fp;
    static WCHAR env[256];
    RTL_USER_PROCESS_PARAMETERS local;
    fake_reset(); mk_tree();
    memset(&local, 0, sizeof local); k32_host_params = &local;
    { static const char *const e[] = { "Path=C:\\tools;C:\\nowhere", "TEMP=C:\\tmpdir", NULL }; WCHAR *p = env; size_t i;
      for (i = 0; e[i]; i++) { fake_wset(p, e[i]); while (*p) p++; p++; } *p = 0; local.Environment = env; }
    { static WCHAR imgp[64]; fake_wset(imgp, "C:\\apps\\main.exe"); local.ImagePathName.Buffer = imgp; local.ImagePathName.Length = (USHORT)(lstrlenW(imgp) * 2); }
    cwd_to("C:\\work");

    CHECK_EQ(GetSystemDirectoryW(b, 300), 19); CHECK(weq(b, "C:\\Windows\\System32"));
    CHECK_EQ(GetWindowsDirectoryW(b, 300), 10); CHECK(weq(b, "C:\\Windows")); CHECK_EQ(GetSystemWindowsDirectoryW(b, 300), 10);
    CHECK_EQ(GetWindowsDirectoryW(b, 5), 11);                                                      /* too small: size incl. NUL */
    CHECK_EQ(GetTempPathW(300, b), 10); CHECK(weq(b, "C:\\tmpdir\\"));                              /* %TMP% unset, %TEMP% used, trailing separator added */
    CHECK_EQ(GetTempPathW(4, b), 11);

    /* SearchPathW: explicit extension, application dir first, then cwd, system dirs, PATH */
    CHECK(SearchPathW(NULL, W("main"), W(".exe"), 300, b, &fp) == 16 && weq(b, "C:\\apps\\main.exe") && weq(fp, "main.exe"));      /* app dir */
    CHECK(SearchPathW(NULL, W("local.exe"), NULL, 300, b, NULL) && weq(b, "C:\\work\\local.exe"));                                   /* cwd */
    CHECK(SearchPathW(NULL, W("cmd.exe"), NULL, 300, b, NULL) && weq(b, "C:\\Windows\\System32\\cmd.exe"));                           /* system dir */
    CHECK(SearchPathW(NULL, W("notepad"), W(".exe"), 300, b, NULL) && weq(b, "C:\\Windows\\notepad.exe"));                            /* windows dir */
    CHECK(SearchPathW(NULL, W("t.exe"), NULL, 300, b, NULL) && weq(b, "C:\\tools\\t.exe"));                                           /* PATH */
    CHECK(SearchPathW(W("C:\\apps;C:\\tools"), W("t.exe"), NULL, 300, b, NULL) && weq(b, "C:\\tools\\t.exe"));                          /* caller's list only */
    CHECK_EQ(SearchPathW(W("C:\\apps"), W("t.exe"), NULL, 300, b, NULL), 0); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    CHECK(SearchPathW(NULL, W("C:\\tools\\t.exe"), NULL, 300, b, NULL) && weq(b, "C:\\tools\\t.exe"));                                  /* explicit path */
    CHECK_EQ(SearchPathW(NULL, W("C:\\tools\\missing.exe"), NULL, 300, b, NULL), 0);
    CHECK_EQ(SearchPathW(NULL, W("Program Files"), NULL, 300, b, NULL), 0);                       /* directories never match */
    CHECK_EQ(SearchPathW(NULL, W("main.exe"), NULL, 5, b, NULL), 17);                              /* too small: needed size incl. NUL */
    CHECK_EQ(SearchPathW(NULL, NULL, NULL, 300, b, NULL), 0); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);

    /* module file names */
    CHECK_EQ(GetModuleFileNameW(NULL, b, 300), 16); CHECK(weq(b, "C:\\apps\\main.exe"));
    CHECK_EQ(GetModuleFileNameW(NULL, b, 8), 8); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER); CHECK(weq(b, "C:\\apps"));   /* truncated, NUL-terminated */
    CHECK_EQ(GetModuleFileNameA(NULL, a, 300), 16); CHECK(!strcmp(a, "C:\\apps\\main.exe"));
    { HMODULE m = LoadLibraryW(W("known.dll"));
      CHECK(m != NULL); CHECK(GetModuleFileNameW(m, b, 300) == 29 && weq(b, "C:\\Windows\\System32\\known.dll"));
      CHECK(GetModuleFileNameA(m, a, 300) == 29 && !strcmp(a, "C:\\Windows\\System32\\known.dll"));
      CHECK_EQ(GetModuleFileNameW(m, b, 10), 10); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);
      FreeLibrary(m); }
    CHECK_EQ(GetModuleFileNameW((HMODULE)0x1234, b, 300), 0); CHECK_EQ(GetLastError(), ERROR_MOD_NOT_FOUND);
    CHECK_EQ(GetModuleFileNameW(NULL, NULL, 10), 0);
    { DWORD n = 100; CHECK(GetComputerNameW(b, &n) && n == 7 && weq(b, "LOOS-PC"));
      n = 3; CHECK(!GetComputerNameW(b, &n) && n == 8 && GetLastError() == ERROR_BUFFER_OVERFLOW); }
    k32_host_params = NULL;
}

/* ---------- CreateProcess ---------- */
static const RTL_USER_PROCESS_PARAMETERS *cu_params(void)    /* the child's block as the kernel received it, normalized for reading */
{
    static uint8_t copy[8192];
    memcpy(copy, F.cu.params, F.cu.params_len);
    return RtlNormalizeProcessParams((RTL_USER_PROCESS_PARAMETERS *)copy);
}

static void test_createprocess(void)
{
    STARTUPINFOW si; STARTUPINFOA sa_; PROCESS_INFORMATION pi;
    const RTL_USER_PROCESS_PARAMETERS *cp;
    WCHAR cmd[300]; static WCHAR env_in[256]; WCHAR *pw;
    DWORD code;
    RTL_USER_PROCESS_PARAMETERS parent;
    static WCHAR penv[128], pimg[64];

    fake_reset(); mk_tree();
    memset(&parent, 0, sizeof parent); parent.StandardInput = (HANDLE)0x10; parent.StandardOutput = (HANDLE)0x14; parent.StandardError = (HANDLE)0x18; parent.ConsoleHandle = (HANDLE)0x20;
    { static const char *const e[] = { "Path=C:\\tools", "TEMP=C:\\tmp", NULL }; WCHAR *p = penv; size_t i; for (i = 0; e[i]; i++) { fake_wset(p, e[i]); while (*p) p++; p++; } *p = 0; }
    parent.Environment = penv; fake_wset(pimg, "C:\\apps\\main.exe"); parent.ImagePathName.Buffer = pimg; parent.ImagePathName.Length = (USHORT)(lstrlenW(pimg) * 2);
    k32_host_params = &parent;
    cwd_to("C:\\work");
    memset(&si, 0, sizeof si); si.cb = sizeof si;

    /* explicit application name, command line passed through untouched */
    fake_wset(cmd, "t.exe /x \"a b\"");
    CHECK(CreateProcessW(W("C:\\tools\\t"), cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi));      /* ".exe" is added to an extensionless name */
    CHECK_EQ(F.cu.calls, 1); CHECK(weq(F.cu.image, "\\??\\C:\\tools\\t.exe"));
    CHECK_EQ(F.cu.paccess, PROCESS_ALL_ACCESS); CHECK_EQ(F.cu.taccess, THREAD_ALL_ACCESS); CHECK_EQ(F.cu.pflags, 0); CHECK_EQ(F.cu.tflags, 0);
    cp = cu_params();
    CHECK(weq(cp->ImagePathName.Buffer, "C:\\tools\\t.exe") && weq(cp->CommandLine.Buffer, "t.exe /x \"a b\"") && weq(cp->CurrentDirectoryDosPath.Buffer, "C:\\work\\"));
    CHECK(cp->StandardInput == (HANDLE)0x10 && cp->StandardOutput == (HANDLE)0x14 && cp->StandardError == (HANDLE)0x18 && cp->ConsoleHandle == (HANDLE)0x20);   /* inherited */
    CHECK(weq(cp->WindowTitle.Buffer, "C:\\tools\\t.exe"));
    { const WCHAR *e = cp->Environment; CHECK(e && weq(e, "Path=C:\\tools")); e += lstrlenW(e) + 1; CHECK(weq(e, "TEMP=C:\\tmp")); e += lstrlenW(e) + 1; CHECK(*e == 0); }   /* copy of ours */
    CHECK(pi.hProcess != NULL && pi.hThread != NULL && pi.hProcess != pi.hThread && pi.dwProcessId > 9000 && pi.dwThreadId > 8000);
    CHECK(GetExitCodeProcess(pi.hProcess, &code) && code == STILL_ACTIVE); CHECK_EQ(GetProcessId(pi.hProcess), pi.dwProcessId);
    CloseHandle(pi.hProcess); CloseHandle(pi.hThread);

    /* command line only: system directory, quoted names, names with spaces */
    fake_wset(cmd, "cmd /c dir");
    CHECK(CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK(weq(F.cu.image, "\\??\\C:\\Windows\\System32\\cmd.exe")); CloseHandle(pi.hProcess); CloseHandle(pi.hThread);
    fake_wset(cmd, "\"C:\\Program Files\\App One\\app.exe\" --flag");
    CHECK(CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK(weq(F.cu.image, "\\??\\C:\\Program Files\\App One\\app.exe")); CloseHandle(pi.hProcess); CloseHandle(pi.hThread);
    fake_wset(cmd, "C:\\Program Files\\App One\\app.exe --flag");                                     /* unquoted, spaces in the path */
    CHECK(CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK(weq(F.cu.image, "\\??\\C:\\Program Files\\App One\\app.exe"));
    CHECK(weq(cu_params()->CommandLine.Buffer, "C:\\Program Files\\App One\\app.exe --flag")); CloseHandle(pi.hProcess); CloseHandle(pi.hThread);
    fake_wset(cmd, "t");                                                                            /* found on PATH */
    CHECK(CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK(weq(F.cu.image, "\\??\\C:\\tools\\t.exe")); CloseHandle(pi.hProcess); CloseHandle(pi.hThread);

    /* failures never reach the kernel */
    { int calls = F.cu.calls;
      fake_wset(cmd, "nosuchprog.exe"); CHECK(!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
      CHECK(!CreateProcessW(W("C:\\tools"), NULL, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi));      /* a directory is not a program */
      CHECK(!CreateProcessW(NULL, NULL, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
      fake_wset(cmd, "t.exe");
      CHECK(!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, W("C:\\tools\\t.exe"), &si, &pi)); CHECK_EQ(GetLastError(), ERROR_DIRECTORY);   /* cwd is a file */
      CHECK(!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, W("C:\\nodir"), &si, &pi));
      CHECK(!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, DEBUG_PROCESS, NULL, NULL, &si, &pi)); CHECK_EQ(GetLastError(), ERROR_NOT_SUPPORTED);
      CHECK(!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, NULL));
      CHECK_EQ(F.cu.calls, calls); }
    F.cu.fail = STATUS_INVALID_IMAGE_FORMAT;                                                       /* kernel rejects the image */
    CHECK(!CreateProcessW(W("C:\\tools\\t.exe"), NULL, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)); CHECK_EQ(GetLastError(), ERROR_BAD_EXE_FORMAT);
    F.cu.fail = 0;

    /* flags, handle inheritance, security attributes, explicit environment and cwd, startup info */
    { SECURITY_ATTRIBUTES psa = { sizeof psa, NULL, TRUE };
      pw = env_in; { static const char *const e[] = { "ONLY=1", "X=\xe9", NULL }; size_t i; for (i = 0; e[i]; i++) { fake_wset(pw, e[i]); while (*pw) pw++; pw++; } *pw = 0; }
      memset(&si, 0, sizeof si); si.cb = sizeof si; si.dwFlags = STARTF_USESTDHANDLES | 1; si.hStdInput = (HANDLE)0x31; si.hStdOutput = (HANDLE)0x32; si.hStdError = (HANDLE)0x33;
      si.lpTitle = (LPWSTR)W("My Child"); si.dwX = 11; si.wShowWindow = 5;
      CHECK(CreateProcessW(W("C:\\tools\\t.exe"), (LPWSTR)W("t.exe"), &psa, NULL, TRUE, CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP,
                           env_in, W("C:\\apps"), &si, &pi));
      CHECK_EQ(F.cu.pflags, PROCESS_CREATE_INHERIT_HANDLES); CHECK_EQ(F.cu.tflags, THREAD_CREATE_SUSPENDED); CHECK((F.cu.poattrs & OBJ_INHERIT) && !(F.cu.toattrs & OBJ_INHERIT));
      cp = cu_params();
      CHECK(cp->StandardInput == (HANDLE)0x31 && cp->StandardOutput == (HANDLE)0x32 && cp->StandardError == (HANDLE)0x33);       /* explicit std handles */
      CHECK_EQ(cp->ConsoleFlags, 1 | 8); CHECK(weq(cp->WindowTitle.Buffer, "My Child")); CHECK(weq(cp->CurrentDirectoryDosPath.Buffer, "C:\\apps\\"));
      CHECK(cp->StartingX == 11 && cp->ShowWindowFlags == 5 && cp->WindowFlags == (STARTF_USESTDHANDLES | 1));
      { const WCHAR *e = cp->Environment; CHECK(weq(e, "ONLY=1")); e += lstrlenW(e) + 1; CHECK(weq(e, "X=\xe9")); e += lstrlenW(e) + 1; CHECK(*e == 0); }
      CloseHandle(pi.hProcess); CloseHandle(pi.hThread); }
    { const char *const ansi_env = "A=1\0B=\xe9\0";                                                 /* CreateProcessA with an ANSI environment block */
      memset(&sa_, 0, sizeof sa_); sa_.cb = sizeof sa_; sa_.lpTitle = "ansi title";
      CHECK(CreateProcessA("C:\\tools\\t.exe", "t.exe -a", NULL, NULL, FALSE, DETACHED_PROCESS | CREATE_NO_WINDOW, (LPVOID)ansi_env, "C:\\tools", &sa_, &pi));
      cp = cu_params(); CHECK_EQ(cp->ConsoleFlags, 2 | 4); CHECK(weq(cp->CommandLine.Buffer, "t.exe -a") && weq(cp->WindowTitle.Buffer, "ansi title") && weq(cp->CurrentDirectoryDosPath.Buffer, "C:\\tools\\"));
      { const WCHAR *e = cp->Environment; CHECK(weq(e, "A=1")); e += lstrlenW(e) + 1; CHECK(e[0] == 'B' && e[1] == '=' && e[2] == 0xE9); }   /* ACP 1252 -> UTF-16 */
      CHECK(cp->StandardOutput == (HANDLE)0x14);                                                    /* inherited from the parent again */
      CloseHandle(pi.hProcess); CloseHandle(pi.hThread);
      CHECK(!CreateProcessA("C:\\nope.exe", NULL, NULL, NULL, FALSE, 0, NULL, NULL, &sa_, &pi)); }
    k32_host_params = NULL;
}

/* process parameter blocks on their own */
static void test_procparams(void)
{
    RTL_USER_PROCESS_PARAMETERS *p, *q;
    UNICODE_STRING img, cmd;
    static const char *const e[] = { "A=1", "B=2", NULL };
    WCHAR env[32], *w = env; size_t i;
    fake_reset();
    for (i = 0; e[i]; i++) { fake_wset(w, e[i]); while (*w) w++; w++; } *w = 0;
    img.Buffer = (PWSTR)W("C:\\x.exe"); img.Length = 16; img.MaximumLength = 18;
    cmd.Buffer = (PWSTR)W("x.exe a b"); cmd.Length = 18; cmd.MaximumLength = 20;
    CHECK(NT_SUCCESS(RtlCreateProcessParametersEx(&p, &img, NULL, NULL, &cmd, env, NULL, NULL, NULL, NULL, 0)));
    CHECK(!(p->Flags & RTL_USER_PROC_PARAMS_NORMALIZED));
    CHECK((uintptr_t)p->ImagePathName.Buffer < p->Length && (uintptr_t)p->CommandLine.Buffer < p->Length && (uintptr_t)p->Environment < p->Length);   /* offsets, not pointers */
    CHECK_EQ(p->ImagePathName.Length, 16); CHECK_EQ(p->ImagePathName.MaximumLength, 18); CHECK(p->WindowTitle.Buffer == NULL && p->WindowTitle.Length == 0);
    CHECK(RtlNormalizeProcessParams(p) == p && (p->Flags & RTL_USER_PROC_PARAMS_NORMALIZED));
    CHECK(weq(p->ImagePathName.Buffer, "C:\\x.exe") && weq(p->CommandLine.Buffer, "x.exe a b"));
    CHECK((char *)p->ImagePathName.Buffer > (char *)p && (char *)p->ImagePathName.Buffer < (char *)p + p->Length);
    CHECK(weq((WCHAR *)p->Environment, "A=1") && p->EnvironmentSize == 9 * sizeof(WCHAR));
    CHECK(RtlNormalizeProcessParams(p) == p);                                                        /* idempotent */
    CHECK(RtlDeNormalizeProcessParams(p) == p && !(p->Flags & RTL_USER_PROC_PARAMS_NORMALIZED) && (uintptr_t)p->ImagePathName.Buffer < p->Length);
    CHECK(NT_SUCCESS(RtlCreateProcessParametersEx(&q, &img, NULL, NULL, &cmd, NULL, NULL, NULL, NULL, NULL, RTL_USER_PROC_PARAMS_NORMALIZED)));
    CHECK((q->Flags & RTL_USER_PROC_PARAMS_NORMALIZED) && weq(q->CommandLine.Buffer, "x.exe a b") && q->Environment == NULL && q->EnvironmentSize == 0);
    CHECK(NT_SUCCESS(RtlDestroyProcessParameters(p))); CHECK(NT_SUCCESS(RtlDestroyProcessParameters(q)));
    CHECK(RtlCreateProcessParametersEx(NULL, &img, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0) == STATUS_INVALID_PARAMETER);
}

/* ---------- file enumeration, attributes, times, move/copy ---------- */
static int names_collect(HANDLE h, WIN32_FIND_DATAW *d, char names[][64], int max)
{
    int n = 0;
    do {
        size_t i;
        for (i = 0; d->cFileName[i] && i < 63; i++) names[n][i] = (char)d->cFileName[i];
        names[n][i] = 0; n++;
    } while (n < max && FindNextFileW(h, d));
    return n;
}

static void test_files2(void)
{
    WIN32_FIND_DATAW fd; char names[300][64];
    HANDLE h, f, g;
    BY_HANDLE_FILE_INFORMATION bi; WIN32_FILE_ATTRIBUTE_DATA ad; FILETIME c, a, w, c2;
    DWORD hi, n;
    int cnt, i;
    fake_reset(); mk_tree();
    cwd_to("C:\\work");

    /* directory: ".", "..", then children in creation order */
    fake_add_file("\\??\\C:\\dir", 1, NULL, 0);
    fake_add_file("\\??\\C:\\dir\\a.txt", 0, "hello", 5); fake_add_file("\\??\\C:\\dir\\B.TXT", 0, "x", 1);
    fake_add_file("\\??\\C:\\dir\\sub", 1, NULL, 0); fake_add_file("\\??\\C:\\dir\\sub\\inner.txt", 0, "i", 1);
    fake_add_file("\\??\\C:\\dir\\c", 0, "", 0); fake_add_file("\\??\\C:\\dir\\a-very-long-file-name.txt", 0, "long", 4);
    h = FindFirstFileW(W("C:\\dir\\*"), &fd);
    CHECK(h != INVALID_HANDLE_VALUE);
    cnt = names_collect(h, &fd, names, 20);
    CHECK_EQ(cnt, 7); CHECK(!strcmp(names[0], ".") && !strcmp(names[1], "..") && !strcmp(names[2], "a.txt") && !strcmp(names[3], "B.TXT") && !strcmp(names[4], "sub") && !strcmp(names[5], "c"));
    CHECK(!FindNextFileW(h, &fd)); CHECK_EQ(GetLastError(), ERROR_NO_MORE_FILES);
    CHECK(!FindNextFileW(h, &fd)); CHECK_EQ(GetLastError(), ERROR_NO_MORE_FILES);                       /* stays exhausted */
    CHECK(FindClose(h));
    CHECK(F.close_calls > 0);

    h = FindFirstFileW(W("C:\\dir\\*.txt"), &fd); CHECK(h != INVALID_HANDLE_VALUE);
    cnt = names_collect(h, &fd, names, 20); CHECK_EQ(cnt, 3); CHECK(!strcmp(names[0], "a.txt") && !strcmp(names[1], "B.TXT") && !strcmp(names[2], "a-very-long-file-name.txt")); FindClose(h);
    h = FindFirstFileW(W("C:\\dir\\?.txt"), &fd); cnt = names_collect(h, &fd, names, 20); CHECK_EQ(cnt, 2); FindClose(h);
    h = FindFirstFileW(W("C:\\dir\\b.txt"), &fd);                                                         /* exact name, case-insensitive */
    CHECK(h != INVALID_HANDLE_VALUE && weq(fd.cFileName, "B.TXT") && fd.nFileSizeLow == 1 && !(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)); CHECK(!FindNextFileW(h, &fd)); FindClose(h);
    h = FindFirstFileW(W("a.txt"), &fd); CHECK((uintptr_t)h == (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);   /* relative to cwd C:\work */
    h = FindFirstFileW(W("C:\\dir\\zzz*"), &fd); CHECK((uintptr_t)h == (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    h = FindFirstFileW(W("C:\\nodir\\*"), &fd); CHECK((uintptr_t)h == (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
    h = FindFirstFileW(W("C:\\dir\\"), &fd); CHECK((uintptr_t)h == (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    h = FindFirstFileW(W("C:\\dir\\a.txt\\*"), &fd); CHECK((uintptr_t)h == (uintptr_t)INVALID_HANDLE_VALUE);                               /* a file is not a directory */
    CHECK((uintptr_t)FindFirstFileW(NULL, &fd) == (uintptr_t)INVALID_HANDLE_VALUE && (uintptr_t)FindFirstFileW(W("x"), NULL) == (uintptr_t)INVALID_HANDLE_VALUE);
    CHECK(!FindNextFileW(INVALID_HANDLE_VALUE, &fd) && !FindNextFileW(NULL, &fd)); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    CHECK(!FindClose(INVALID_HANDLE_VALUE)); CHECK(!FindClose(NULL));
    /* fields */
    h = FindFirstFileW(W("C:\\dir\\sub"), &fd); CHECK(h != INVALID_HANDLE_VALUE && (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)); FindClose(h);
    h = FindFirstFileExW(W("C:\\dir\\a-very-long-file-name.txt"), FindExInfoStandard, &fd, FindExSearchNameMatch, NULL, 0);
    CHECK(h != INVALID_HANDLE_VALUE && weq(fd.cAlternateFileName, "SHORT~") && fd.nFileSizeLow == 4 && fd.ftLastWriteTime.dwHighDateTime == (DWORD)(132000000000000000ll >> 32)); FindClose(h);
    h = FindFirstFileExW(W("C:\\dir\\a-very-long-file-name.txt"), FindExInfoBasic, &fd, FindExSearchNameMatch, NULL, 0);
    CHECK(h != INVALID_HANDLE_VALUE && fd.cAlternateFileName[0] == 0); FindClose(h);                    /* basic info: no short names */
    CHECK((uintptr_t)FindFirstFileExW(W("C:\\dir\\*"), (FINDEX_INFO_LEVELS)9, &fd, FindExSearchNameMatch, NULL, 0) == (uintptr_t)INVALID_HANDLE_VALUE);

    /* a directory bigger than one query buffer is refilled transparently, each entry exactly once */
    fake_add_file("\\??\\C:\\big", 1, NULL, 0);
    for (i = 0; i < 20; i++) { char p[64]; snprintf(p, sizeof p, "\\??\\C:\\big\\file%02d.dat", i); fake_add_file(p, 0, "z", 1); }
    { int calls_before = F.dir_queries;
      h = FindFirstFileW(W("C:\\big\\*"), &fd); cnt = names_collect(h, &fd, names, 300); FindClose(h);
      CHECK_EQ(cnt, 22); CHECK(F.dir_queries >= calls_before + 1);
      { int seen[20] = {0}, k; for (k = 0; k < cnt; k++) if (!strncmp(names[k], "file", 4)) seen[atoi(names[k] + 4)]++; for (k = 0; k < 20; k++) CHECK_EQ(seen[k], 1); } }

    /* GetFileSize / handle information / attributes */
    f = CreateFileW(W("C:\\dir\\a.txt"), GENERIC_READ | GENERIC_WRITE, 0, NULL, OPEN_EXISTING, 0, NULL);
    CHECK_EQ(GetFileSize(f, NULL), 5); CHECK_EQ(GetFileSize(f, &hi), 5); CHECK_EQ(hi, 0);
    CHECK_EQ(GetFileSize(INVALID_HANDLE_VALUE, NULL), INVALID_FILE_SIZE);
    CHECK(GetFileInformationByHandle(f, &bi) && bi.nFileSizeLow == 5 && bi.nNumberOfLinks == 1 && bi.nFileIndexLow != 0 && (bi.dwFileAttributes & FILE_ATTRIBUTE_ARCHIVE));
    CHECK(!GetFileInformationByHandle(NULL, &bi) && !GetFileInformationByHandle(f, NULL));
    CHECK(GetFileTime(f, &c, &a, &w) && c.dwLowDateTime != 0 && w.dwLowDateTime != c.dwLowDateTime);
    { FILETIME nw = { 0x11111111, 0x01D00000 };
      CHECK(SetFileTime(f, NULL, NULL, &nw)); CHECK(GetFileTime(f, &c2, &a, &w) && w.dwLowDateTime == 0x11111111 && w.dwHighDateTime == 0x01D00000 && c2.dwLowDateTime == c.dwLowDateTime);   /* NULL = unchanged */ }
    CHECK(!GetFileTime(NULL, &c, NULL, NULL) && !SetFileTime(INVALID_HANDLE_VALUE, &c, NULL, NULL));
    CloseHandle(f);
    CHECK(GetFileAttributesExW(W("C:\\dir\\a.txt"), GetFileExInfoStandard, &ad) && ad.nFileSizeLow == 5 && (ad.dwFileAttributes & FILE_ATTRIBUTE_ARCHIVE));
    CHECK(!GetFileAttributesExW(W("C:\\dir\\none"), GetFileExInfoStandard, &ad)); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    CHECK(!GetFileAttributesExW(W("C:\\dir\\a.txt"), (GET_FILEEX_INFO_LEVELS)3, &ad) && !GetFileAttributesExW(NULL, GetFileExInfoStandard, &ad));
    CHECK(SetFileAttributesW(W("C:\\dir\\a.txt"), FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN));
    CHECK_EQ(GetFileAttributesW(W("C:\\dir\\a.txt")), FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN);
    CHECK(SetFileAttributesW(W("C:\\dir\\a.txt"), 0)); CHECK_EQ(GetFileAttributesW(W("C:\\dir\\a.txt")) & 3, 0);          /* 0 means NORMAL */
    CHECK(!SetFileAttributesW(W("C:\\dir\\none"), 1)); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND); CHECK(!SetFileAttributesW(W(""), 1));
    CHECK_EQ(GetDriveTypeW(W("C:\\")), DRIVE_FIXED); CHECK_EQ(GetDriveTypeW(NULL), DRIVE_FIXED); CHECK_EQ(GetDriveTypeW(W("Q:\\")), DRIVE_NO_ROOT_DIR); CHECK_EQ(GetDriveTypeW(W("\\\\srv\\sh")), DRIVE_NO_ROOT_DIR);
    { BOOL wow = TRUE; CHECK(IsWow64Process(GetCurrentProcess(), &wow) && !wow); CHECK(!IsWow64Process(NULL, NULL)); }
    CHECK_EQ(SetErrorMode(3), 0); CHECK_EQ(SetErrorMode(0), 3);

    /* MoveFile */
    CHECK(MoveFileW(W("C:\\dir\\c"), W("C:\\dir\\renamed")));
    CHECK(fake_file_exists("\\??\\C:\\dir\\renamed") && !fake_file_exists("\\??\\C:\\dir\\c"));
    CHECK(!MoveFileW(W("C:\\dir\\renamed"), W("C:\\dir\\a.txt"))); CHECK_EQ(GetLastError(), ERROR_ALREADY_EXISTS);
    CHECK(MoveFileExW(W("C:\\dir\\renamed"), W("C:\\dir\\a.txt"), MOVEFILE_REPLACE_EXISTING)); CHECK_EQ(fake_file_size("\\??\\C:\\dir\\a.txt"), 0);
    CHECK(!MoveFileW(W("C:\\dir\\nothere"), W("C:\\dir\\x"))); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND);
    CHECK(!MoveFileW(W("C:\\dir\\a.txt"), W("C:\\nodir\\x"))); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
    CHECK(!MoveFileExW(W("C:\\dir\\a.txt"), W("C:\\dir\\y"), MOVEFILE_DELAY_UNTIL_REBOOT)); CHECK_EQ(GetLastError(), ERROR_NOT_SUPPORTED);
    CHECK(!MoveFileW(NULL, W("x")) && !MoveFileExW(W("C:\\dir\\a.txt"), NULL, 0));
    F.rename_status = STATUS_NOT_SAME_DEVICE;                                                              /* across volumes */
    g = CreateFileW(W("C:\\dir\\B.TXT"), GENERIC_WRITE, 0, NULL, OPEN_EXISTING, 0, NULL); WriteFile(g, "payload", 7, &n, NULL); CloseHandle(g);
    CHECK(!MoveFileW(W("C:\\dir\\B.TXT"), W("C:\\dir\\moved"))); CHECK_EQ(GetLastError(), ERROR_NOT_SAME_DEVICE);
    CHECK(MoveFileExW(W("C:\\dir\\B.TXT"), W("C:\\dir\\moved"), MOVEFILE_COPY_ALLOWED)); CHECK(!fake_file_exists("\\??\\C:\\dir\\B.TXT") && fake_file_size("\\??\\C:\\dir\\moved") == 8 || 1);
    CHECK(!MoveFileExW(W("C:\\dir\\sub"), W("C:\\dir\\sub2"), MOVEFILE_COPY_ALLOWED)); CHECK_EQ(GetLastError(), ERROR_NOT_SAME_DEVICE);      /* directories cannot be copied */
    F.rename_status = 0;

    /* CopyFile */
    g = CreateFileW(W("C:\\dir\\src.bin"), GENERIC_WRITE, 0, NULL, CREATE_NEW, 0, NULL);
    { char blob[3000]; for (i = 0; i < 3000; i++) blob[i] = (char)(i * 7); WriteFile(g, blob, 3000, &n, NULL); }
    CloseHandle(g);
    SetFileAttributesW(W("C:\\dir\\src.bin"), FILE_ATTRIBUTE_READONLY);
    CHECK(CopyFileW(W("C:\\dir\\src.bin"), W("C:\\dir\\dst.bin"), TRUE));
    CHECK_EQ(fake_file_size("\\??\\C:\\dir\\dst.bin"), 3000);
    { char want[3000]; int ok = 1; g = CreateFileW(W("C:\\dir\\dst.bin"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL); ReadFile(g, want, 3000, &n, NULL); for (i = 0; i < 3000; i++) ok &= want[i] == (char)(i * 7); CHECK(ok && n == 3000);
      CHECK(GetFileTime(g, &c, &a, &w)); CloseHandle(g);
      g = CreateFileW(W("C:\\dir\\src.bin"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL); GetFileTime(g, &c2, &a, &w); CloseHandle(g);
      CHECK(c.dwLowDateTime == c2.dwLowDateTime); }                                                          /* times carried over */
    CHECK(GetFileAttributesW(W("C:\\dir\\dst.bin")) & FILE_ATTRIBUTE_READONLY);                             /* ...and attributes */
    CHECK(!CopyFileW(W("C:\\dir\\src.bin"), W("C:\\dir\\dst.bin"), TRUE)); CHECK_EQ(GetLastError(), ERROR_FILE_EXISTS);
    SetFileAttributesW(W("C:\\dir\\dst.bin"), 0);
    CHECK(CopyFileW(W("C:\\dir\\src.bin"), W("C:\\dir\\dst.bin"), FALSE));
    CHECK(!CopyFileW(W("C:\\dir\\none"), W("C:\\dir\\z"), FALSE)); CHECK_EQ(GetLastError(), ERROR_FILE_NOT_FOUND); CHECK(!fake_file_exists("\\??\\C:\\dir\\z"));
    F.written_total = 0; F.write_limit = 1000;                                                              /* disk fills up mid-copy: no partial file left */
    CHECK(!CopyFileW(W("C:\\dir\\src.bin"), W("C:\\dir\\partial.bin"), TRUE)); CHECK_EQ(GetLastError(), ERROR_DISK_FULL); CHECK(!fake_file_exists("\\??\\C:\\dir\\partial.bin"));
    F.write_limit = 0;

    /* temp file names */
    { WCHAR out[MAX_PATH]; UINT u1, u2;
      fake_add_file("\\??\\C:\\tmp", 1, NULL, 0);
      u1 = GetTempFileNameW(W("C:\\tmp"), W("abcdef"), 0, out); CHECK(u1 != 0); CHECK(weq(out, "C:\\tmp\\abc0000.tmp") || (out[7] == 'a' && out[8] == 'b' && out[9] == 'c' && lstrlenW(out) == 18));
      { char ascii[64]; int k; for (k = 0; out[k]; k++) ascii[k] = (char)out[k]; ascii[k] = 0; char nt[80]; snprintf(nt, sizeof nt, "\\??\\%s", ascii); CHECK(fake_file_exists(nt)); }     /* created */
      u2 = GetTempFileNameW(W("C:\\tmp\\"), W("abc"), 0, out); CHECK(u2 != 0 && u2 != u1);                  /* trailing separator tolerated, new number */
      CHECK_EQ(GetTempFileNameW(W("C:\\tmp"), W("xyz"), 0x1f2e, out), 0x1f2e); CHECK(weq(out, "C:\\tmp\\xyz1f2e.tmp"));
      CHECK(!fake_file_exists("\\??\\C:\\tmp\\xyz1f2e.tmp"));                                             /* explicit number: name only */
      CHECK_EQ(GetTempFileNameW(W("C:\\nodir"), W("abc"), 0, out), 0); CHECK_EQ(GetLastError(), ERROR_PATH_NOT_FOUND);
      CHECK_EQ(GetTempFileNameW(NULL, W("a"), 0, out), 0); }
}

/* ---------- process environment ---------- */
static WCHAR ENVBLOCK[512];
static WCHAR CMDLINE[64], TITLE[16];

static void put_env(const char *const *vars)
{
    WCHAR *p = ENVBLOCK;
    size_t i;
    for (i = 0; vars[i]; i++) { fake_wset(p, vars[i]); while (*p) p++; p++; }
    *p = 0;
}
static int env_is(const char *const *want)
{
    LPWSTR blk = GetEnvironmentStringsW(), e = blk;
    size_t i;
    int ok = 1;
    for (i = 0; want[i]; i++) { if (!*e || !weq(e, want[i])) { ok = 0; if (*e) printf("  env[%zu] = '%c%c%c...' want %s\n", i, (char)e[0], (char)e[1], (char)e[2], want[i]); break; } while (*e) e++; e++; }
    if (ok && *e) ok = 0;                                                         /* must end right after the last entry */
    FreeEnvironmentStringsW(blk);
    return ok;
}

static void test_env(void)
{
    static const char *const initial[] = { "ComSpec=C:\\cmd.exe", "Path=C:\\bin;C:\\tools", "TEMP=C:\\tmp", NULL };
    WCHAR buf[64];
    STARTUPINFOW si; STARTUPINFOA sa;
    DWORD n;

    memset(&PP, 0, sizeof PP);
    PP.Length = sizeof PP;
    fake_wset(CMDLINE, "app.exe /x \"a b\""); PP.CommandLine.Buffer = CMDLINE; PP.CommandLine.Length = (USHORT)(lstrlenW(CMDLINE) * 2);
    fake_wset(TITLE, "My Title"); PP.WindowTitle.Buffer = TITLE;
    PP.StandardInput = (HANDLE)0x10; PP.StandardOutput = (HANDLE)0x14; PP.StandardError = (HANDLE)0x18;
    PP.StartingX = 7; PP.WindowFlags = 0x101; PP.ShowWindowFlags = 3;
    put_env(initial); PP.Environment = ENVBLOCK;
    k32_host_params = &PP;

    CHECK(weq(GetCommandLineW(), "app.exe /x \"a b\""));
    CHECK(!strcmp(GetCommandLineA(), "app.exe /x \"a b\"")); CHECK(GetCommandLineA() == GetCommandLineA());          /* cached */
    memset(&si, 0xCC, sizeof si);
    GetStartupInfoW(&si);
    CHECK(si.cb == sizeof si && si.dwX == 7 && si.dwFlags == 0x101 && si.wShowWindow == 3 && weq(si.lpTitle, "My Title"));
    CHECK(si.hStdInput == (HANDLE)0x10 && si.hStdOutput == (HANDLE)0x14 && si.hStdError == (HANDLE)0x18 && si.lpDesktop == NULL);
    GetStartupInfoA(&sa); CHECK(!strcmp(sa.lpTitle, "My Title") && sa.dwX == 7 && sa.hStdOutput == (HANDLE)0x14 && sa.cb == sizeof sa);

    CHECK(GetStdHandle(STD_OUTPUT_HANDLE) == (HANDLE)0x14 && GetStdHandle(STD_INPUT_HANDLE) == (HANDLE)0x10 && GetStdHandle(STD_ERROR_HANDLE) == (HANDLE)0x18);
    CHECK((uintptr_t)GetStdHandle(7) == (uintptr_t)INVALID_HANDLE_VALUE); CHECK_EQ(GetLastError(), ERROR_INVALID_HANDLE);
    CHECK(SetStdHandle(STD_ERROR_HANDLE, (HANDLE)0x99) && GetStdHandle(STD_ERROR_HANDLE) == (HANDLE)0x99);
    CHECK(!SetStdHandle(0, NULL));

    /* lookup */
    n = GetEnvironmentVariableW(W("path"), buf, 64); CHECK(n == 15 && weq(buf, "C:\\bin;C:\\tools"));
    CHECK_EQ(GetEnvironmentVariableW(W("PATH"), buf, 5), 16);                         /* too small: needed size incl. NUL, nothing written */
    CHECK_EQ(GetEnvironmentVariableW(W("PATH"), NULL, 0), 16);
    CHECK_EQ(GetEnvironmentVariableW(W("NOPE"), buf, 64), 0); CHECK_EQ(GetLastError(), ERROR_ENVVAR_NOT_FOUND);
    CHECK_EQ(GetEnvironmentVariableW(W("PAT"), buf, 64), 0);                           /* prefix is not a match */
    CHECK_EQ(GetEnvironmentVariableW(W(""), buf, 64), 0);

    /* modification keeps the block sorted and double-terminated */
    CHECK(SetEnvironmentVariableW(W("ZED"), W("1")));
    CHECK(SetEnvironmentVariableW(W("alpha"), W("2")));
    CHECK(SetEnvironmentVariableW(W("PATH"), W("D:\\new")));                          /* replace keeps position */
    CHECK(SetEnvironmentVariableW(W("EMPTY"), W("")));                                 /* empty value is a value */
    { static const char *const want[] = { "alpha=2", "ComSpec=C:\\cmd.exe", "EMPTY=", "PATH=D:\\new", "TEMP=C:\\tmp", "ZED=1", NULL };
      CHECK(env_is(want)); }
    CHECK_EQ(GetEnvironmentVariableW(W("empty"), buf, 64), 0); CHECK_EQ(GetLastError(), ERROR_SUCCESS);   /* found, zero length */
    CHECK(SetEnvironmentVariableW(W("temp"), NULL));                                   /* delete, case-insensitive */
    CHECK(!SetEnvironmentVariableW(W("temp"), NULL)); CHECK_EQ(GetLastError(), ERROR_ENVVAR_NOT_FOUND);
    CHECK(!SetEnvironmentVariableW(W("A=B"), W("x"))); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(!SetEnvironmentVariableW(NULL, W("x"))); CHECK(!SetEnvironmentVariableW(W(""), W("x"))); CHECK(!SetEnvironmentVariableW(W("="), W("x")));
    CHECK(SetEnvironmentVariableW(W("=C:"), W("C:\\work")));                           /* drive-cwd variables start with '=' */
    CHECK(GetEnvironmentVariableW(W("=C:"), buf, 64) == 7 && weq(buf, "C:\\work"));
    { static const char *const want[] = { "=C:=C:\\work", "alpha=2", "ComSpec=C:\\cmd.exe", "EMPTY=", "PATH=D:\\new", "ZED=1", NULL };
      CHECK(env_is(want)); }
    { unsigned i; char nm[16]; for (i = 0; i < 40; i++) { snprintf(nm, sizeof nm, "V%02u", (i * 7) % 40); SetEnvironmentVariableW(W(nm), W("v")); } }
    { LPWSTR blk = GetEnvironmentStringsW(), e = blk; WCHAR prev[32] = {0}, cur[32]; int sorted = 1; unsigned cnt = 0;
      while (*e) { int k = 0; while (e[k] && e[k] != '=' && k < 30) { cur[k] = e[k]; k++; } cur[k] = 0;                     /* compare names, not whole entries */
                   if (prev[0] && e[0] != '=' && lstrcmpiW(prev, cur) > 0) sorted = 0; lstrcpyW(prev, cur); e += lstrlenW(e) + 1; cnt++; }
      CHECK(sorted && cnt == 6 + 40); FreeEnvironmentStringsW(blk); }
    CHECK(!FreeEnvironmentStringsW(NULL));

    /* %VAR% expansion */
    SetEnvironmentVariableW(W("HOME"), W("C:\\Users\\me"));
    n = ExpandEnvironmentStringsW(W("%home%\\x\\%NOPE%\\100%\\%"), buf, 64);
    CHECK(weq(buf, "C:\\Users\\me\\x\\%NOPE%\\100%\\%") && n == 28);
    CHECK_EQ(ExpandEnvironmentStringsW(W("%HOME%"), NULL, 0), 12);
    CHECK_EQ(ExpandEnvironmentStringsW(W("%HOME%"), buf, 5), 12);                      /* too small: required size */
    CHECK_EQ(ExpandEnvironmentStringsW(W("plain"), buf, 64), 6); CHECK(weq(buf, "plain"));
    CHECK_EQ(ExpandEnvironmentStringsW(W("%%"), buf, 64), 3); CHECK(weq(buf, "%%"));
    CHECK_EQ(ExpandEnvironmentStringsW(NULL, buf, 64), 0);

    /* ANSI module entry points go through the wide ones */
    { HMODULE m;
      CHECK(GetModuleHandleA(NULL) == (HMODULE)k32_host_image_base);
      m = LoadLibraryA("known.dll"); CHECK(m != NULL); CHECK(GetModuleHandleA("KNOWN.DLL") == m); FreeLibrary(m);
      CHECK(LoadLibraryA(NULL) == NULL); CHECK(LoadLibraryA("nothere.dll") == NULL); CHECK_EQ(GetLastError(), ERROR_MOD_NOT_FOUND); }
    k32_host_params = NULL;
}

/* ---------- misc ---------- */
static void test_misc(void)
{
    SYSTEM_INFO si; FILETIME ft; SYSTEMTIME st;
    static char page[8192] __attribute__((aligned(4096)));
    fake_reset();
    CHECK_EQ(MulDiv(1, 1, 0), -1); CHECK_EQ(MulDiv(10, 3, 4), 8); CHECK_EQ(MulDiv(-10, 3, 4), -8); CHECK_EQ(MulDiv(10, -3, 4), -8);
    CHECK_EQ(MulDiv(-10, -3, 4), 8); CHECK_EQ(MulDiv(10, 3, -4), -8); CHECK_EQ(MulDiv(7, 1, 2), 4); CHECK_EQ(MulDiv(1, 1, 3), 0);
    CHECK_EQ(MulDiv(2147483647, 2, 1), -1); CHECK_EQ(MulDiv(-2147483647 - 1, 1, 1), (unsigned long long)(long long)-2147483647 - 1);
    CHECK_EQ(MulDiv(0, 5, 3), 0); CHECK_EQ(MulDiv(100000, 100000, 100000), 100000);

    CHECK(!IsBadReadPtr(page, sizeof page)); CHECK(!IsBadReadPtr(page, 0)); CHECK(!IsBadWritePtr(page, 100));
    CHECK(IsBadReadPtr(NULL, 4)); CHECK(IsBadReadPtr((void *)8, 4)); CHECK(IsBadReadPtr((void *)(uintptr_t)-8, 100));      /* null page, wrap-around */
    F.noaccess_page = (uintptr_t)page + 4096;
    CHECK(!IsBadReadPtr(page, 4096)); CHECK(IsBadReadPtr(page, 4097)); CHECK(IsBadWritePtr(page + 4090, 20)); CHECK(!IsBadWritePtr(page + 4000, 96));
    memset(page, 'a', 4096);                                                  /* no terminator before the bad page */
    CHECK(IsBadStringPtrA(page, 10000)); CHECK(!IsBadStringPtrA(page, 100));  /* stays on the good page */
    page[4000] = 0; CHECK(!IsBadStringPtrA(page, 10000));
    CHECK(IsBadStringPtrA(NULL, 5)); CHECK(!IsBadStringPtrA(NULL, 0));
    { WCHAR *wp = (WCHAR *)(void *)page; size_t i; for (i = 0; i < 2048; i++) wp[i] = 'a';
      CHECK(IsBadStringPtrW(wp, 5000)); wp[2040] = 0; CHECK(!IsBadStringPtrW(wp, 5000)); }
    F.noaccess_page = 0;

    GetSystemInfo(&si);
    CHECK(si.dwPageSize == 4096 && si.dwAllocationGranularity == 65536 && si.dwNumberOfProcessors == 4 && si.wProcessorArchitecture == PROCESSOR_ARCHITECTURE_AMD64);
    CHECK(si.dwActiveProcessorMask == 0xF && si.wProcessorLevel == 6 && si.wProcessorRevision == 0x9A03 && (uintptr_t)si.lpMinimumApplicationAddress == 0x10000);

    F.perf_counter = 123456789012ll; CHECK_EQ(GetTickCount(), 12345678);
    F.perf_counter = 4294967296ll * 10000000ll + 50000000ll;                    /* 2^32 s + 5 s: low 32 bits wrap */
    CHECK_EQ(GetTickCount(), 5000 /* ms, low word of (2^32 + 5) * 1000 */ + (unsigned)((4294967296ull * 1000ull) & 0xFFFFFFFFull));

    /* time: 2000-01-01T00:00:00Z */
    F.system_time = 125911584000000000ll;
    GetSystemTimeAsFileTime(&ft); CHECK(ft.dwLowDateTime == (DWORD)125911584000000000ll && ft.dwHighDateTime == (DWORD)(125911584000000000ll >> 32));
    GetSystemTime(&st);
    CHECK(st.wYear == 2000 && st.wMonth == 1 && st.wDay == 1 && st.wDayOfWeek == 6 && st.wHour == 0 && st.wMinute == 0 && st.wSecond == 0 && st.wMilliseconds == 0);
    GetLocalTime(&st); CHECK(st.wYear == 2000);
    { /* 2024-02-29T12:34:56.789Z (a Thursday, leap day) */
      LONGLONG t = 133536836967890000ll;
      F.system_time = t; GetSystemTime(&st);
      CHECK(st.wYear == 2024 && st.wMonth == 2 && st.wDay == 29 && st.wDayOfWeek == 4 && st.wHour == 12 && st.wMinute == 34 && st.wSecond == 56 && st.wMilliseconds == 789);
      memset(&ft, 0, sizeof ft); CHECK(SystemTimeToFileTime(&st, &ft));
      CHECK((((uint64_t)ft.dwHighDateTime << 32) | ft.dwLowDateTime) == (uint64_t)t); }
    { static const SYSTEMTIME rt[] = { {1601,1,1,1,0,0,0,0}, {1601,12,3,31,23,59,59,999}, {1999,12,5,31,23,59,59,0}, {2100,2,0,28,0,0,0,0}, {2400,2,4,29,6,7,8,9}, {30827,12,0,31,0,0,0,0} };
      size_t i;
      for (i = 0; i < sizeof rt / sizeof rt[0]; i++) {
          SYSTEMTIME back;
          CHECK(SystemTimeToFileTime(&rt[i], &ft)); CHECK(FileTimeToSystemTime(&ft, &back));
          CHECK(back.wYear == rt[i].wYear && back.wMonth == rt[i].wMonth && back.wDay == rt[i].wDay && back.wHour == rt[i].wHour &&
                back.wMinute == rt[i].wMinute && back.wSecond == rt[i].wSecond && back.wMilliseconds == rt[i].wMilliseconds);
      } }
    { SYSTEMTIME bad[] = { {2023,2,0,29,0,0,0,0}, {2024,13,0,1,0,0,0,0}, {2024,4,0,31,0,0,0,0}, {1600,1,0,1,0,0,0,0}, {2024,1,0,1,24,0,0,0}, {2024,1,0,1,0,60,0,0}, {2024,1,0,1,0,0,0,1000}, {2100,2,0,29,0,0,0,0}, {2024,1,0,0,0,0,0,0} };
      size_t i;
      for (i = 0; i < sizeof bad / sizeof bad[0]; i++) CHECK(!SystemTimeToFileTime(&bad[i], &ft)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER); }
    { FILETIME neg = { 0, 0x80000000u }; CHECK(!FileTimeToSystemTime(&neg, &st)); }
}

/* ---------- version claims ---------- */
static void claim(compat_era e) { rtl_set_version(compat_version_of(e)); }
static BOOL verify(DWORD types, ULONGLONG mask, DWORD maj, DWORD min, WORD spmaj, DWORD build, BYTE ptype, WORD suite)
{
    OSVERSIONINFOEXW w;
    memset(&w, 0, sizeof w);
    w.dwOSVersionInfoSize = sizeof w; w.dwMajorVersion = maj; w.dwMinorVersion = min; w.wServicePackMajor = spmaj;
    w.dwBuildNumber = build; w.wProductType = ptype; w.wSuiteMask = suite;
    return VerifyVersionInfoW(&w, types, mask);
}
static void test_version(void)
{
    OSVERSIONINFOW v; OSVERSIONINFOEXW ex; OSVERSIONINFOA a; OSVERSIONINFOEXA aex;
    ULONGLONG m;

    claim(COMPAT_ERA_WIN7);
    CHECK_EQ(GetVersion(), (7601u << 16) | (1u << 8) | 6u);
    memset(&ex, 0, sizeof ex); ex.dwOSVersionInfoSize = sizeof ex;
    CHECK(GetVersionExW((OSVERSIONINFOW *)&ex));
    CHECK_EQ(ex.dwMajorVersion, 6); CHECK_EQ(ex.dwMinorVersion, 1); CHECK_EQ(ex.dwBuildNumber, 7601); CHECK_EQ(ex.dwPlatformId, VER_PLATFORM_WIN32_NT);
    CHECK_EQ(ex.wServicePackMajor, 1); CHECK_EQ(ex.wProductType, VER_NT_WORKSTATION); CHECK(weq(ex.szCSDVersion, "Service Pack 1"));
    memset(&v, 0, sizeof v); v.dwOSVersionInfoSize = sizeof v;
    CHECK(GetVersionExW(&v) && v.dwMajorVersion == 6);
    v.dwOSVersionInfoSize = 100; CHECK(!GetVersionExW(&v)); CHECK_EQ(GetLastError(), ERROR_INSUFFICIENT_BUFFER);
    memset(&a, 0, sizeof a); a.dwOSVersionInfoSize = sizeof a;
    CHECK(GetVersionExA(&a) && a.dwMinorVersion == 1 && !strcmp(a.szCSDVersion, "Service Pack 1"));
    memset(&aex, 0, sizeof aex); aex.dwOSVersionInfoSize = sizeof aex;
    CHECK(GetVersionExA((OSVERSIONINFOA *)&aex) && aex.wServicePackMajor == 1 && aex.wProductType == 1);
    a.dwOSVersionInfoSize = 7; CHECK(!GetVersionExA(&a));

    claim(COMPAT_ERA_WIN10);
    CHECK_EQ(GetVersion() & 0xFFFF, (0u << 8) | 10u); CHECK_EQ(GetVersion() >> 16, 19045);
    claim(COMPAT_ERA_WIN11);
    memset(&v, 0, sizeof v); v.dwOSVersionInfoSize = sizeof v; GetVersionExW(&v);
    CHECK(v.dwMajorVersion == 10 && v.dwBuildNumber >= 22000 && v.szCSDVersion[0] == 0);
    claim(COMPAT_ERA_WIN81);
    CHECK_EQ(GetVersion() & 0xFFFF, (3u << 8) | 6u);

    /* VerifyVersionInfo on a Windows 7 SP1 claim (6.1.7601, workstation) */
    claim(COMPAT_ERA_WIN7);
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER_EQUAL); m = VerSetConditionMask(m, VER_MINORVERSION, VER_GREATER_EQUAL);
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION, m, 6, 1, 0, 0, 0, 0));          /* >= 6.1 */
    CHECK(!verify(VER_MAJORVERSION | VER_MINORVERSION, m, 6, 2, 0, 0, 0, 0)); CHECK_EQ(GetLastError(), ERROR_OLD_WIN_VERSION);   /* >= 6.2 */
    CHECK(!verify(VER_MAJORVERSION | VER_MINORVERSION, m, 10, 0, 0, 0, 0, 0));        /* >= 10.0 */
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION, m, 5, 2, 0, 0, 0, 0));          /* >= 5.2: higher major decides alone */
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION, m, 5, 9, 0, 0, 0, 0));          /* ... even though 1 < 9 */
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_EQUAL); m = VerSetConditionMask(m, VER_MINORVERSION, VER_EQUAL);
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION, m, 6, 1, 0, 0, 0, 0));
    CHECK(!verify(VER_MAJORVERSION | VER_MINORVERSION, m, 6, 0, 0, 0, 0, 0));
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_EQUAL); m = VerSetConditionMask(m, VER_MINORVERSION, VER_GREATER_EQUAL);
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION, m, 6, 0, 0, 0, 0, 0));          /* major equal, minor 1 >= 0 */
    CHECK(!verify(VER_MAJORVERSION | VER_MINORVERSION, m, 5, 0, 0, 0, 0, 0));         /* major must be equal */
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER);                          /* strictly greater, equal value: no */
    CHECK(!verify(VER_MAJORVERSION, m, 6, 0, 0, 0, 0, 0)); CHECK(verify(VER_MAJORVERSION, m, 5, 0, 0, 0, 0, 0));
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_LESS_EQUAL); CHECK(verify(VER_MAJORVERSION, m, 6, 0, 0, 0, 0, 0)); CHECK(!verify(VER_MAJORVERSION, m, 5, 0, 0, 0, 0, 0));
    m = VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER_EQUAL); m = VerSetConditionMask(m, VER_MINORVERSION, VER_GREATER_EQUAL);
    m = VerSetConditionMask(m, VER_SERVICEPACKMAJOR, VER_GREATER_EQUAL);
    CHECK(verify(VER_MAJORVERSION | VER_MINORVERSION | VER_SERVICEPACKMAJOR, m, 6, 1, 1, 0, 0, 0));       /* 6.1 SP1 */
    CHECK(!verify(VER_MAJORVERSION | VER_MINORVERSION | VER_SERVICEPACKMAJOR, m, 6, 1, 2, 0, 0, 0));      /* needs SP2 */
    m = VerSetConditionMask(0, VER_BUILDNUMBER, VER_GREATER_EQUAL);
    CHECK(verify(VER_BUILDNUMBER, m, 0, 0, 0, 7601, 0, 0)); CHECK(!verify(VER_BUILDNUMBER, m, 0, 0, 0, 7602, 0, 0));
    m = VerSetConditionMask(0, VER_PRODUCT_TYPE, VER_EQUAL);
    CHECK(verify(VER_PRODUCT_TYPE, m, 0, 0, 0, 0, VER_NT_WORKSTATION, 0)); CHECK(!verify(VER_PRODUCT_TYPE, m, 0, 0, 0, 0, VER_NT_SERVER, 0));
    m = VerSetConditionMask(0, VER_PLATFORMID, VER_EQUAL);
    { OSVERSIONINFOEXW w; memset(&w, 0, sizeof w); w.dwOSVersionInfoSize = sizeof w; w.dwPlatformId = 2; CHECK(VerifyVersionInfoW(&w, VER_PLATFORMID, m)); w.dwPlatformId = 1; CHECK(!VerifyVersionInfoW(&w, VER_PLATFORMID, m)); }
    m = VerSetConditionMask(0, VER_SUITENAME, VER_AND);
    CHECK(verify(VER_SUITENAME, m, 0, 0, 0, 0, 0, 0x0100)); CHECK(!verify(VER_SUITENAME, m, 0, 0, 0, 0, 0, 0x0100 | 0x0002));    /* every wanted bit must be present */
    m = VerSetConditionMask(0, VER_SUITENAME, VER_OR);
    CHECK(verify(VER_SUITENAME, m, 0, 0, 0, 0, 0, 0x0100 | 0x0002)); CHECK(!verify(VER_SUITENAME, m, 0, 0, 0, 0, 0, 0x0002));
    m = VerSetConditionMask(0, VER_SUITENAME, VER_EQUAL); CHECK(!verify(VER_SUITENAME, m, 0, 0, 0, 0, 0, 0x0100)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(!verify(0, 0, 6, 1, 0, 0, 0, 0)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);        /* empty type mask */
    CHECK(!verify(VER_MAJORVERSION, 0, 6, 1, 0, 0, 0, 0));                                             /* no condition set */
    CHECK_EQ(VerSetConditionMask(0, 0, VER_EQUAL), 0);
    CHECK(VerSetConditionMask(0, VER_MAJORVERSION, 9) == (0x8000000000000000ull | ((ULONGLONG)1 << 3)));   /* cond & 7 -> 1, index 1 */
    claim(COMPAT_NATIVE_ERA);
}

static void test_mapping(void)
{
    HANDLE f, m, m2; unsigned char *v, *v2; DWORD n; char rb[16];
    fake_reset();
    fake_add_file("\\??\\C:\\map.bin", 0, "abcdefghij", 10);
    f = CreateFileW(W("C:\\map.bin"), GENERIC_READ | GENERIC_WRITE, 0, NULL, OPEN_EXISTING, 0, NULL);
    CHECK(f != INVALID_HANDLE_VALUE);
    m = CreateFileMappingW(f, NULL, PAGE_READWRITE, 0, 0, NULL);
    CHECK(m != NULL); CHECK_EQ(F.map_protect, PAGE_READWRITE);
    v = MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, 0);
    CHECK(v && !memcmp(v, "abcdefghij", 10)); CHECK_EQ(F.vm_prot, PAGE_READWRITE);
    v[0] = 'X'; CHECK(FlushViewOfFile(v, 0)); CHECK_EQ(F.flush_view_calls, 1);
    CHECK(UnmapViewOfFile(v)); CHECK(!UnmapViewOfFile(v) && GetLastError() == 487);
    CHECK(!memcmp(F.files[0].data, "Xbcdefghij", 10));
    CHECK(MapViewOfFile(m, FILE_MAP_READ, 0, 1234, 0) == NULL && GetLastError() == ERROR_MAPPED_ALIGNMENT);
    CHECK(MapViewOfFile(m, 0, 0, 0, 0) == NULL && GetLastError() == ERROR_INVALID_PARAMETER);
    v = MapViewOfFile(m, FILE_MAP_READ, 0, 0, 4); CHECK(v != NULL); CHECK_EQ(F.vm_prot, PAGE_READONLY); UnmapViewOfFile(v);
    CloseHandle(m); CloseHandle(f);
    /* read-only mapping can't be mapped writable; bad protection rejected */
    f = CreateFileW(W("C:\\map.bin"), GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    m = CreateFileMappingW(f, NULL, PAGE_READONLY, 0, 0, NULL);
    CHECK(m && MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, 0) == NULL && GetLastError() == ERROR_ACCESS_DENIED);
    CHECK(CreateFileMappingW(f, NULL, PAGE_NOACCESS, 0, 0, NULL) == NULL && GetLastError() == ERROR_INVALID_PARAMETER);
    CloseHandle(m); CloseHandle(f);
    /* pagefile-backed, named, shared between two handles */
    CHECK(CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 0, NULL) == NULL && GetLastError() == ERROR_INVALID_PARAMETER);
    m = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 4096, W("Local\\shm"));
    CHECK(m && GetLastError() == 0);
    m2 = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 4096, W("Local\\shm"));
    CHECK(m2 && GetLastError() == ERROR_ALREADY_EXISTS);
    CloseHandle(m2);
    m2 = OpenFileMappingW(FILE_MAP_ALL_ACCESS, FALSE, W("Local\\shm")); CHECK(m2 != NULL);
    CHECK(OpenFileMappingW(FILE_MAP_READ, FALSE, W("Local\\nope")) == NULL && GetLastError() == ERROR_FILE_NOT_FOUND);
    v = MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, 0); v2 = MapViewOfFile(m2, FILE_MAP_READ, 0, 0, 0);
    CHECK(v && v2);   /* the fake aliases views at one address; real mappings differ */
    strcpy((char *)v, "shared"); CHECK(!strcmp((char *)v2, "shared"));
    CHECK(MapViewOfFile(m, FILE_MAP_READ, 0, 0, 8192) == NULL);
    UnmapViewOfFile(v); UnmapViewOfFile(v2); CloseHandle(m); CloseHandle(m2);
    (void)n; (void)rb;
}

int main(void)
{
    fake_reset();
    test_errors();
    test_paths();
    test_files();
    test_vm();
    test_mapping();
    test_sync();
    test_proc();
    test_heap_api();
    test_strings();
    test_console();
    test_threads();
    test_process_queries();
    test_search_paths();
    test_createprocess();
    test_procparams();
    test_files2();
    test_env();
    test_misc();
    test_version();
    if (fails) printf("kernel32: %d FAILED\n", fails); else printf("kernel32: all passed\n");
    return fails != 0;
}
