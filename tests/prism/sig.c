/* Signals through Prism: sigaction handler with Darwin numbering, signal(), sigprocmask, qsort. */
int printf(const char *, ...);
int raise(int);
typedef void (*handler_t)(int);
handler_t signal(int, handler_t);
struct sigaction { handler_t sa_handler; unsigned sa_mask; int sa_flags; };
int sigaction(int, const struct sigaction *, struct sigaction *);
int sigprocmask(int, const unsigned *, unsigned *);
void qsort(void *, unsigned long, unsigned long, int (*)(const void *, const void *));

static volatile int got_usr1, got_term;
static void on_usr1(int s) { got_usr1 = s; }
static void on_term(int s) { got_term = s; }
static int cmp(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

int main(void) {
    struct sigaction sa = {on_usr1, 0, 0};
    sigaction(30, &sa, 0); /* SIGUSR1 is 30 on Darwin */
    raise(30);
    signal(15, on_term);
    raise(15);
    unsigned set = 1u << (30 - 1), old = 0;
    sigprocmask(1, &set, 0); /* block SIGUSR1 */
    sigprocmask(1, 0, &old);
    int v[] = {5, 3, 9, 1, 7};
    qsort(v, 5, sizeof v[0], cmp);
    printf("sig: usr1=%d term=%d blocked=%d sorted=%d%d%d%d%d\n", got_usr1, got_term, (old >> 29) & 1, v[0], v[1], v[2], v[3], v[4]);
    return 0;
}
