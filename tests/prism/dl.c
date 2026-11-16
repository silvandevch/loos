/* dlopen, dlsym, dlerror and dladdr. */
int printf(const char *, ...);
void *dlopen(const char *, int);
void *dlsym(void *, const char *);
int dlclose(void *);
char *dlerror(void);
typedef struct { const char *dli_fname; void *dli_fbase; const char *dli_sname; void *dli_saddr; } Dl_info;
int dladdr(const void *, Dl_info *);
void *objc_getClass(const char *);
#define RTLD_DEFAULT ((void *)-2)

int main(void) {
    int before = objc_getClass("Plugin") != 0;
    void *h = dlopen("@executable_path/libplug.dylib", 2);
    int (*value)(void) = h ? dlsym(h, "plug_value") : 0;
    unsigned long (*len)(const char *) = dlsym(RTLD_DEFAULT, "strlen");
    long (*labs_)(long) = dlsym(RTLD_DEFAULT, "labs"); /* not imported by anything: bound on demand */
    void *again = dlopen("@executable_path/libplug.dylib", 2);
    Dl_info info = {0};
    int where = value ? dladdr((void *)value, &info) : 0;
    const char *base = where && info.dli_fname ? info.dli_fname : "?";
    for (const char *p = base; *p; p++) if (*p == '/') base = p + 1;
    int clean = dlerror() == 0;
    void *missing = dlopen("/nowhere/libnone.dylib", 2);
    char *err = dlerror();
    printf("dl: before=%d value=%d len=%lu labs=%ld same=%d in=%s clean=%d missing=%d err=%d nosym=%d self=%d close=%d\n", before, value ? value() : -1,
           len ? len("prism") : 0, labs_ ? labs_(-9) : 0, again == h, base, clean, missing != 0, err != 0 && dlerror() == 0,
           dlsym(h, "no_such_symbol") != 0, dlsym(dlopen(0, 2), "main") == (void *)main, dlclose(h));
    return 0;
}
