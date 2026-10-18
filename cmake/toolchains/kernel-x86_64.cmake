set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

find_program(LOOS_CLANG clang REQUIRED)
set(CMAKE_C_COMPILER "${LOOS_CLANG}" CACHE FILEPATH "Clang C compiler")
set(CMAKE_C_COMPILER_TARGET x86_64-unknown-none-elf)
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# Bare-metal target: never pick up host system headers (glibc's stdint.h pulls
# in bits/floatn.h and fails on x86_64-unknown-none). Use only clang's own
# freestanding headers plus the module's own -I paths.
execute_process(
  COMMAND "${LOOS_CLANG}" -print-resource-dir
  OUTPUT_VARIABLE LOOS_CLANG_RESOURCE_DIR
  OUTPUT_STRIP_TRAILING_WHITESPACE)

# Minimal freestanding libc headers (string.h/stdlib.h) for the kernel, which
# builds static libraries and does not link a libc.
get_filename_component(LOOS_FREESTANDING_INCLUDE
  "${CMAKE_CURRENT_LIST_DIR}/../freestanding/include" ABSOLUTE)

set(CMAKE_C_FLAGS_INIT
  "-ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -nostdinc -isystem ${LOOS_FREESTANDING_INCLUDE} -isystem ${LOOS_CLANG_RESOURCE_DIR}/include")
