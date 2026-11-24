# LoOS Prism — macOS compatibility

Prism runs Mach-O programs on LoOS. A macOS process is an ordinary process of the Linux
subsystem (`crates/loos-lin`), so it uses the same address spaces, file descriptors, VFS,
scheduler, signals and windows. The code is in `crates/loos-prism`.

| Piece | Where | Status |
|---|---|---|
| Mach-O parsing: thin x86_64/arm64, universal (fat/fat64), segments, sections, `LC_MAIN`/`LC_UNIXTHREAD`, dylibs, rpaths | `macho.rs` | working |
| Fixups: `LC_DYLD_INFO` rebase/bind/lazy-bind opcodes, `LC_DYLD_CHAINED_FIXUPS` (`PTR_64`, `PTR_64_OFFSET`), export trie with re-exports | `macho.rs` | working, including arm64e (`PTR_ARM64E`, `USERLAND`, `USERLAND24`) |
| Loader: bundled dylibs via `@executable_path`/`@loader_path`/`@rpath`, initializers (`__mod_init_func`, `__init_offsets`), TLV templates | `loader.rs` | working |
| libSystem in Rust: stdio/printf family, malloc, string/memory, POSIX file I/O, env, time, atexit, Mach port names | `libsys.rs` | 136 functions; anything else stops the program with "X is not implemented by Prism's libSystem" |
| Darwin syscalls (BSD class 2, Mach traps, `thread_set_tsd_base`), errno/flags/`stat64` translation, carry-flag convention | `darwin.rs` | common calls; Mach IPC returns `MACH_SEND_INVALID_DEST` |
| Apple Silicon: AArch64 interpreter (integer, loads/stores, LSE atomics, exclusives, scalar FP, common SIMD), arm64e pointer authentication | `arm64.rs` | runs Apple's own arm64e tools |
| `.app` bundles (`Info.plist` XML, `CFBundleExecutable`) | `bundle.rs` | working (binary plists not yet) |

## How it fits in

- The kernel's Linux boot path (`crates/kffi/src/lin.rs`) hands any Mach-O file or `.app` given
  as `loos.lin=` to `Prism::spawn`.
- Imports from `/usr/lib/*` and `/System/Library/*` bind to 16-byte stubs at `0x7000_0000_0000`
  (`mov r10, rcx; mov eax, 0x4C50_0000+i; syscall; ret`). Intel code runs natively in ring 3 and
  traps into `Prism::syscall`. The ARM64 interpreter treats a branch into that range as the same
  call.
- Process start goes through the `__prism_start` stub: it runs initializers, then calls
  `main(argc, argv, envp, apple)`. `main` returns to `__prism_main_ret`, which runs the atexit
  handlers and exits.

## Objective-C

Prism's runtime includes an Objective-C runtime (`libc/objc.c`, `objc_msg.c`, `nsobject.m`,
`block.c`). At startup it reads every image's `__objc_*` sections (found through
`_dyld_image_count`/`_dyld_get_image_header`), uniques selectors, realizes classes eagerly
(sliding ivars past a grown superclass), attaches categories, makes relative method lists
absolute, and calls `+load`. `objc_msgSend` and the super/stret/fpret variants save the
argument registers, look the method up (per-class cache, `+initialize`, `+resolve*Method:`,
`forwardingTargetForSelector:`), and tail-jump to it. Reference counts, weak references and
associated objects live in side tables; autorelease pools are per thread. The runtime tables,
side tables and atomic properties are behind recursive locks, and `@synchronized` is a real
per-object recursive lock (exercised uncontended until Prism has threads). Tagged pointers use
macOS's layouts on both architectures (`_objc_registerTaggedPointerClass`). `NSObject`, `Protocol`
and the block classes are Objective-C; `_NSConcrete*Block` become copies of those classes, so
blocks answer messages. `dispatch_once` and the blocks runtime (`_Block_copy`, `__block`
variables) are there too.

Exceptions use the Itanium unwinder in `libc/unwind.c` (compact unwind and DWARF CFI, LSDA
scanning): `@try`/`@catch`/`@finally`, rethrow, and cleanups during unwinding. The runtime itself
is built with `-funwind-tables` so the unwinder can walk out of `objc_exception_throw`; an
uncaught exception prints its class and aborts.

`dlopen` maps the new image (and its dependencies) into the running process, applies fixups,
binds new libSystem imports to fresh stubs, runs initializers and the Objective-C
map/`+load` for it. `dlsym`, `dladdr`, `dlerror` and `dlopen_preflight` work; `dlclose` never
unloads.

## CoreFoundation

CF objects are Objective-C objects, so `CFStringRef` and `NSString *` (and the other pairs) are
interchangeable. The value logic is Rust (`src/cf/`: string encoding, comparison, search and
case mapping; number conversion, comparison and hashing; byte/pointer buffers and the
open-addressing hash table behind dictionaries). A thin Objective-C layer (`libc/cfstring.m`,
`libc/cfbase.m`) makes these class clusters, each with one concrete `__NSCF*` class:
`NSString`/`NSMutableString`, `NSNumber` (plus `CFBoolean`), `NSData`/`NSMutableData`,
`NSArray`/`NSMutableArray`, `NSDictionary`/`NSMutableDictionary` and `NSNull`, with the CF
functions on them, `CFRetain`/`CFRelease`/`CFEqual`/`CFHash`/`CFGetTypeID`, string formatting
with `%@`, and `NSLog`. `@"..."` constants get their class the way blocks do:
`__CFConstantStringClassReference` becomes a copy of `__NSCFConstantString`. The compiler's
constant literals (`NSConstantIntegerNumber`, `NSConstantDoubleNumber`, `NSConstantArray`,
`NSConstantDictionary`, `__kCFBooleanTrue`) are classes of their own.

Beyond the values:

- `NSSet`/`CFSet` share the dictionary's hash table; `NSDate`/`CFDate` hold a `CFAbsoluteTime`.
- `CFRunLoop`/`NSRunLoop` (`libc/cfrunloop.m`): timers (`CFRunLoopTimer`, `NSTimer`,
  `performSelector:afterDelay:`), version-0 sources, observers and `CFRunLoopPerformBlock`, each
  scheduled in a set of modes; a run in a mode services what is in that mode, or in the common
  modes when it is one of them. There is one run loop (one thread per process); it sleeps with
  `nanosleep` until the next timer.
- `CFURL`/`NSURL` and `NSString`'s path methods (`libc/cfurl.m`, logic in `src/cf/url.rs`: RFC 3986
  parsing and resolution, percent escapes, path components).
- `CFBundle`/`NSBundle`: the main bundle is the `.app` around the executable (or its directory),
  with `Info.plist`, resources (also in `en.lproj`/`Base.lproj`) and `Localizable.strings`.
- Property lists (`libc/cfplist.m`, `src/cf/plist.rs`): reading and writing XML (keys sorted, as
  CF does), binary and OpenStep/strings text. The text format has only strings, data, arrays and
  dictionaries, so numbers, booleans and dates are written as text. (macOS refuses to write it.) `CFPropertyList*`,
  `NSPropertyListSerialization`, and the `contentsOfFile`/`writeToFile` methods of NSData,
  NSString, NSArray and NSDictionary.
- `NSException` (`libc/nsexception.m`): range errors, nil arguments, unrecognized selectors and
  mutation during enumeration raise the exceptions macOS raises, catchable with `@catch`;
  `objc_msgSend` has unwind info so exceptions pass through it. An uncaught one prints macOS's
  `*** Terminating app due to uncaught exception ...` line (after an `NSSetUncaughtExceptionHandler`
  handler) and aborts.

- Foundation services: `NSFileManager` (`libc/nsfile.m`, with `NSDirectoryEnumerator` and the
  search-path functions), `NSProcessInfo` and `NSUUID` (`libc/nsprocess.m`), `NSUserDefaults`
  and `CFPreferences` (argument, application, global and registration domains; each change is
  written at once to `~/Library/Preferences/<domain>.plist` as on macOS), `NSNotificationCenter`
  (observers held weakly, blocks, `NSNotificationQueue`) and `CFNotificationCenter`
  (`libc/nsnotification.m`), and `NSError`.
- Keyed archiving (`libc/nsarchive.m`): `NSKeyedArchiver`/`NSKeyedUnarchiver` in Apple's format
  (a binary plist of `$objects` linked by UIDs), secure coding with allowed classes, shared and
  cyclic references, and coding for the built-in classes.

Not yet: `NSCharacterSet`, `NSScanner`, `NSFormatter`s, `NSAttributedString`, `NSOperationQueue`
and threads (Prism runs one thread per process).

## Processes

`fork` gives the child a copy of the parent's Prism state. An `execve` of a Mach-O, by any
process (Linux or macOS), comes back from the Linux layer as `Sys::ExecForeign`; Prism loads the
program and calls `LinSystem::exec_image`. An `execve` of an ELF program by a Prism process leaves
Prism. `posix_spawn` forks; the child carries out the file actions and the exec without running
program code. All of this works across architectures (an Intel program can start an arm64 one).

## arm64e

Apple's own binaries ship as x86_64 + arm64e. Prism loads arm64e code without PAC keys:
signed pointers in chained fixups are written unsigned, `PAC*` leave a pointer unchanged,
`AUT*`/`XPAC*` are no-ops, and `BRAA`/`BLRAA`/`RETAA`/`LDRAA` branch or load without
checking. A program cannot tell the difference unless it inspects pointer bits itself.

## Roadmap to full macOS application support

| Stage | Scope | Status |
|---|---|---|
| 1 | libSystem coverage; pthreads, sockets, `kqueue` on epoll, `statfs`/`getmntinfo`, C++ exceptions, `getaddrinfo`/DNS (done); tool coverage | in progress |
| 2 | `mach_msg` with port rights, launchd bootstrap registry, libdispatch on threads, XPC | planned |
| 3 | NSCharacterSet, NSScanner, formatters, NSAttributedString, NSOperationQueue, NSThread, NSURLSession, JSON, binary plists | partial |
| 4 | Swift runtime; loading frameworks from a user-supplied dyld shared cache | planned |
| 5 | Window server shim on the win32k/compositor path; CoreGraphics, CoreText, QuartzCore; AppKit; SwiftUI | planned |
| 6 | Metal (compute and basic render passes) | planned |
| 7 | Electron apps via the Linux Electron runtime; arm64 JIT; `.dmg`/`.pkg` install flows | partial |

Known gaps today:

- `posix_spawn`: process groups and sessions are skipped; `POSIX_SPAWN_CLOEXEC_DEFAULT` is ignored.
- Sockets (`libc/socket.c`): AF_UNIX, AF_INET (IPv4) and loopback TCP/UDP on the Linux layer, with
  Darwin sockaddr, option and `MSG_*` translation, `inet_pton`/`ntop`. `kqueue`/`kevent`/`kevent64`
  (`libc/kqueue.c`) run on epoll: EVFILT_READ, WRITE, TIMER and USER. Name resolution
  (`libc/netdb.c`): `getaddrinfo`, `getnameinfo`, `gethostbyname`, `getservbyname`, from numeric
  addresses, `localhost`, `/etc/hosts`, `/etc/services`, then DNS over UDP (CNAME chains, A, AAAA)
  to the servers in `/etc/resolv.conf`; the Linux layer generates that file from the DHCP lease when
  none exists. The network is up only when the boot command line has `loos.net`. Not yet: mDNS,
  search domains, reverse DNS (PTR), EVFILT_VNODE/PROC/SIGNAL, control messages in `sendmsg`/`recvmsg`, SIGPIPE suppression.
- C++ (`libc/cxxabi.c`, the libc++abi part): exceptions (throw, catch by type, base class or
  pointer, rethrow, cleanups), `std::exception_ptr`, `__dynamic_cast`, function-local static
  guards, `std::terminate` and its handler, `operator new`/`delete`, the fundamental type_infos and
  `std::exception`/`bad_alloc`/`bad_cast`/`bad_typeid`. Apple's arm64 non-unique RTTI names are
  honoured. Not yet: the rest of libc++ (`std::string`, streams, containers' out-of-line parts),
  `__cxa_demangle`.
- `statfs`, `fstatfs`, `getfsstat`, `getmntinfo` (`libc/statfs.c`): one APFS-style root volume.
- Threads (x86_64 and arm64, one interpreter CPU per thread): real LoOS threads (`libc/pthread.c` over Prism's `bsdthread_create`), with
  futex-backed mutexes, condition variables, rwlocks, `os_unfair_lock`, keys, and per-thread errno
  and thread-locals. Timed waits sleep on the futex with
  the remaining time; detached threads' stacks are unmapped by a later `pthread_create`. Not yet:
  the shared scratch buffer used by some calls is not per-thread.
- The ARM64 interpreter has no JIT. QEMU TCG and Dynarmic are C/C++ and do not fit the no_std
  kernel crate.
- Out of scope: kernel and system extensions, Apple ID and iCloud, FairPlay DRM, App Store
  receipt validation.

## Tests

- `cargo +1.97.1 test -p loos-prism`: unit tests, plus real binaries built from
  `tests/prism/*.c` with clang and rustup's `ld64.lld`. The arm64 program runs end to end on the
  interpreter.
- `PRISM_MAC_BIN=<dir> cargo +1.97.1 test -p loos-prism`: Apple's arm64e `pwd`, `printenv`,
  `dirname`, `cksum`, `sum`, `xxd` and `getopt` (copied from macOS, not stored here) on the
  interpreter, with output compared against macOS.
- `boot/prism_macos_test.sh`: those tools plus `realpath`, `uname`, `head`, `base64`, `jot`,
  `mkdir` and `mktemp`, natively (x86_64 slice) on LoOS under QEMU, copied from a Mac over ssh
  (`PRISM_MAC=user@host`). 75 of the 683 tools in macOS 15's `/bin` and `/usr/bin` have every
  import implemented.
- `boot/prism_test.sh`: boots LoOS in QEMU and runs the Intel build natively, the arm64 build on
  the interpreter, a universal `.app`, an app with a bundled dylib, and `tests/prism/sig.c`
  (signals, qsort, realpath, mkstemp/mkdtemp, sysctlbyname, sscanf, setmode/getmode, wcwidth,
  strtod/strtof, mbrtowc, fnmatch, strptime, strtonum, syslog), `tests/prism/objc.m`
  (Objective-C with ARC: classes, categories, blocks, weak references, forwarding, exceptions,
  tagged pointers, `@synchronized`), `tests/prism/dl.c` with `plug.m` (dlopen), `tests/prism/cf.m`
  (CoreFoundation, toll-free bridging and literals), `tests/prism/found.m` as `Found.app`
  (exceptions, sets, dates, run loop and modes, URLs, property lists, the main bundle, files,
  process info, defaults, notifications, keyed archiving) and `tests/prism/spawn.c` (fork, exec, posix_spawn with a pipe, execvp) for
  both architectures. The boots run in parallel, `PRISM_JOBS` at a time (default 4), each in
  its own QEMU window (`QEMU_DISPLAY=none` for none); the logs are checked afterwards in order.
