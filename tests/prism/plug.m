/* A plug-in loaded with dlopen: an initializer, an Objective-C class with +load, and imports the
 * host program does not use (new libSystem stubs bound into the running process). */
int printf(const char *, ...);
int atoi(const char *);
char *strchr(const char *, int);

__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (instancetype)new;
@end

static int ready;
__attribute__((constructor)) static void plug_init(void) { ready += 100; }

@interface Plugin : NSObject
- (int)answer;
@end
@implementation Plugin
+ (void)load { ready += 10; printf("load: Plugin\n"); }
- (int)answer { return atoi(strchr("x=42", '=') + 1); }
@end

int plug_value(void) { return ready + [[Plugin new] answer]; }
