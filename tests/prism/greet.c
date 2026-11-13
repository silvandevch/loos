/* A dylib an app ships in Contents/Frameworks (loaded by Prism via @rpath). */
int printf(const char *, ...);
int greet_base = 100;
int greet_add(int a, int b) { return a + b + greet_base; }
void greet(const char *who) { printf("greet: hello %s from a bundled dylib\n", who); }
