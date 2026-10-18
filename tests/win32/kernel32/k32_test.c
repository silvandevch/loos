#define LOOS_HOST_TEST 1
#include "fake.h"
#include "../../../src/win32/kernel32/k32_internal.h"
#include "../../../src/win32/compat/compat.h"
void rtl_set_version(const compat_version *v);
#include <stdio.h>
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

int main(void)
{
    fake_reset();
    test_errors();
    test_paths();
    test_files();
    test_vm();
    test_sync();
    test_proc();
    test_version();
    if (fails) printf("kernel32: %d FAILED\n", fails); else printf("kernel32: all passed\n");
    return fails != 0;
}
