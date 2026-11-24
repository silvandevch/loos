// C++ exceptions and RTTI through Prism's C++ ABI runtime. No SDK / libc++ headers: the std
// classes the runtime provides are declared here with their real layouts and mangled names.
extern "C" int printf(const char *, ...);
extern "C" void _exit(int);
namespace std {
class exception {
public:
    exception() noexcept;
    virtual ~exception() noexcept;
    virtual const char *what() const noexcept;
};
class bad_cast : public exception {
public:
    bad_cast() noexcept;
    ~bad_cast() noexcept override;
    const char *what() const noexcept override;
};
class exception_ptr {
    void *p;
public:
    exception_ptr() noexcept : p(0) {}
    exception_ptr(const exception_ptr &) noexcept;
    exception_ptr &operator=(const exception_ptr &) noexcept;
    ~exception_ptr() noexcept;
    explicit operator bool() const noexcept { return p != 0; }
};
exception_ptr current_exception() noexcept;
[[noreturn]] void rethrow_exception(exception_ptr);
typedef void (*terminate_handler)();
terminate_handler set_terminate(terminate_handler) noexcept;
int uncaught_exceptions() noexcept;
class type_info {
public:
    virtual ~type_info();
    const char *name() const noexcept { return n; }
private:
    const char *n;
};
}

static int dtors, raii, during;
struct Err : std::exception {
    int code;
    explicit Err(int c) : code(c) {}
    ~Err() noexcept override { dtors++; }
    const char *what() const noexcept override { return "err"; }
};
struct A { int a = 1; virtual ~A() {} };
struct B { int b = 2; virtual ~B() {} };
struct C : A, B { int c = 3; };
struct Guard { ~Guard() { raii++; during = std::uncaught_exceptions(); } };
struct Other { virtual ~Other() {} };

static int calls;
static int init() { return ++calls * 10; }
static int once() { static int v = init(); return v; }

__attribute__((noinline)) static void thrower(int k) {
    Guard g;
    if (k == 1) throw 42;
    if (k == 2) throw Err(7);
    if (k == 3) throw C();
    if (k == 4) throw new C();
    if (k == 5) throw "text";
}

static void on_terminate() {
    printf("cxx: terminate handler ran\n");
    _exit(0);
}

int main() {
    int i = 0, e = 0, mi = 0, ptr = 0, vptr = 0, rethrow = 0, nested = 0, str = 0, any = 0, ep = 0;
    try { thrower(1); } catch (long) { i = -1; } catch (int v) { i = v; }
    try { thrower(2); } catch (const std::exception &x) { e = (x.what()[0] == 'e') + 10 * static_cast<const Err &>(x).code; }
    try { thrower(3); } catch (B &b) { mi = b.b; }
    try { thrower(4); } catch (B *b) { ptr = b->b; delete static_cast<C *>(b); }
    try { thrower(4); } catch (void *p) { vptr = p != 0; delete static_cast<C *>(p); }
    try { thrower(5); } catch (const char *s) { str = s[0] == 't'; }
    try {
        try { thrower(2); } catch (Err &x) { x.code = 9; throw; }
    } catch (Err &x) { rethrow = x.code; }
    try { thrower(1); } catch (...) {
        try { thrower(2); } catch (Err &x) { nested = x.code; }
    }
    try { throw 3.5; } catch (int) { any = -1; } catch (...) { any = 1; }
    std::exception_ptr saved;
    try { thrower(2); } catch (...) { saved = std::current_exception(); }
    try { std::rethrow_exception(saved); } catch (Err &x) { ep = x.code; }

    A *base = new C();
    C *down = dynamic_cast<C *>(base);
    Other *none = dynamic_cast<Other *>(base);
    B *cross = dynamic_cast<B *>(base);
    int dyn = down != 0 && none == 0 && cross != 0 && cross->b == 2;
    delete base;
    int statics = once() + once() == 20 && calls == 1;
    printf("cxx: int=%d exc=%d mi=%d ptr=%d void=%d str=%d rethrow=%d nested=%d any=%d ep=%d dyn=%d static=%d raii=%d during=%d dtors_ok=%d\n",
           i, e, mi, ptr, vptr, str, rethrow, nested, any, ep, dyn, statics, raii, during, dtors == 3);
    std::set_terminate(on_terminate);
    throw 1; // uncaught
}
