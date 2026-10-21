# LoOS C standard library (C11 hosted environment). The OS backend (src/os_linux.c) targets the Linux x86-64
# syscall ABI that LoOS' Linux-compat layer provides; swap that one file to retarget the library.
# Built freestanding (-ffreestanding -fno-builtin -nostdinc for the C library headers) so it can never pick up
# the host's libc headers or symbols.
if(CMAKE_SYSTEM_NAME STREQUAL "Linux" AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
    enable_language(ASM)
    file(GLOB LOOS_LIBC_SRC CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/src/*.c)
    add_library(loos_c STATIC ${LOOS_LIBC_SRC} ${CMAKE_CURRENT_SOURCE_DIR}/arch/x86_64.S)
    target_include_directories(loos_c SYSTEM PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)
    target_include_directories(loos_c PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)
    target_compile_options(loos_c PRIVATE -std=gnu11 -ffreestanding -fno-builtin -fno-stack-protector -fno-pic -fno-pie
        -nostdlibinc -O2 -Wall -Wextra -Wno-unused-parameter -fno-strict-aliasing -mno-red-zone)
    set_target_properties(loos_c PROPERTIES POSITION_INDEPENDENT_CODE OFF)

    # Self-hosted test: a static, -nostdlib executable whose every libc symbol comes from loos_c.
    add_executable(libc_test ${CMAKE_SOURCE_DIR}/tests/libc/libc_test.c)
    target_link_libraries(libc_test PRIVATE loos_c)
    target_compile_options(libc_test PRIVATE -std=gnu11 -ffreestanding -fno-builtin -fno-stack-protector -fno-pic -fno-pie
        -nostdlibinc -O1 -g -Wall -Wno-unused-value)
    target_link_options(libc_test PRIVATE -nostdlib -static)
    add_test(NAME libc.libc_test COMMAND libc_test)
endif()

# Dependency-free subset (mem*/str*/ctype) for the bare-metal kernel: the kernel toolchain defines
# LOOS_FREESTANDING_INCLUDE, and its libraries link this instead of carrying private copies.
if(LOOS_FREESTANDING_INCLUDE)
    add_library(loos_kcrt STATIC src/string.c src/ctype.c)
    target_compile_definitions(loos_kcrt PRIVATE LOOS_LIBC_PURE=1)
    target_compile_options(loos_kcrt PRIVATE -fno-builtin -O2 -Wall -Wextra -Wno-unused-parameter)
endif()
