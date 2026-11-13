/* Main program of the dylib test app: imports code and data from libgreet.dylib. */
int printf(const char *, ...);
extern int greet_base;
int greet_add(int, int);
void greet(const char *);
int main(void) {
    greet("prism");
    greet_base = 1000;
    printf("app: greet_add=%d\n", greet_add(2, 3));
    return 0;
}
