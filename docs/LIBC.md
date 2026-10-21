# LoOS C standard library (`src/libc`)

A from-scratch C11 hosted-environment library for native and Linux-ABI LoOS programs. No telemetry, no
network, no code borrowed (algorithm credits in `docs/PROVENANCE.md`).

## Layout

| Path | Contents |
|---|---|
| `include/` | public headers: assert ctype errno inttypes locale math setjmp signal stdio stdlib string strings time unistd wchar wctype (stddef/stdarg/stdint/limits/float/stdbool come from the compiler's freestanding headers) |
| `src/os_linux.c` | **the only file that issues system calls** (Linux x86-64 ABI, as exposed by LoOS Linux-compat). Retarget the library by replacing this file. |
| `arch/x86_64.S` | `_start`, `setjmp`/`longjmp` family |
| `src/malloc.c` | boundary-tag allocator, 16-byte-class fast bins (<=512 B, ~35M malloc/free per second), immediate coalescing, ≥256 KiB requests get their own mapping, `aligned_alloc`/`posix_memalign` |
| `src/string.c`, `ctype.c`, `wchar.c` | `<string.h>` (+ BSD/POSIX extras), C-locale ctype, UTF-8 multibyte/wide support, `setlocale`, signals |
| `src/stdio.c` | buffered `FILE` (fd and memory backed), `fopen`…`fseek`, `getline`, `tmpfile`, `fmemopen`, scanf engine |
| `src/format.c` | the printf engine (all C11 conversions incl. `%a`, `%n`, `%lc/%ls`) |
| `src/strtod.c` | correctly rounded decimal/hex → double/float (big-integer exact; Clinger fast path) |
| `src/math.c` | libm: x87 extended-precision kernels for transcendentals, bit-exact floor/ceil/fmod/frexp/ldexp |
| `src/time.c` | `gmtime`/`mktime`/`strftime`/clocks/sleep; local time = UTC or `TZ=UTC±hh[:mm]` |
| `src/posix.c` | `open/stat/fstat/mkdir/getcwd/pipe/dup/fork/execve/execvp/waitpid`, `opendir/readdir` (getdents64), `sysconf`, `gettimeofday`; `system()` is a real `fork`+`/bin/sh -c` |
| `src/complex.c`, `rt_complex.c`, `fenv.c` | `<complex.h>` (+ `__muldc3`/`__divdc3` runtime with Annex G inf/NaN recovery), `<fenv.h>` (x87+MXCSR) |
| `src/stdlib.c` | integer parsing, `qsort` (introsort), `bsearch`, env, `atexit`/`exit`, `rand` |

## Guarantees

- `printf` family prints floating point from the exact binary expansion with round-half-even, so output
  matches a correctly rounding libc digit for digit. `strtod`/`strtof` are correctly rounded for any input
  length including subnormals and ties. Both were fuzz-compared against CPython (12k parse + 4k print cases,
  0 mismatches).
- `errno` is per-process (single-threaded libc); `malloc` is not thread-safe yet.
- `long double` math and `strtold` have double precision; `%Lf` prints the value converted to double.

## Known gaps (tracked)

Threads/locking (`threads.h`, pthreads), sockets, `poll/select`, real locale/zone databases, `wprintf`/`wscanf`, `long double` math.

## Build / test

`ninja libc_test && ctest -R libc` — the test links `-nostdlib -static` against `loos_c`, so no host libc symbol
can leak in. 5179 checks cover string/ctype/malloc stress+perf/stdlib/strtod/printf/scanf/files/math/time/POSIX/complex/fenv.
