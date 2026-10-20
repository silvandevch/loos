cmake_minimum_required(VERSION 3.25)
project(LoOS LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

option(LOOS_BUILD_WIN32 "Build the Win32 subsystem libraries and their host tests" ON)
option(LOOS_BUILD_APPS "Build the LoOS desktop applications (windows-msvc preset only)" OFF)

include(${CMAKE_CURRENT_LIST_DIR}/cmake/LoosWarnings.cmake)

include(CTest)

# ---- filesystems ----
add_subdirectory(src/libc)
add_subdirectory(src/net)
add_subdirectory(src/pkg)
add_subdirectory(src/fs/btrfs)
add_subdirectory(src/fs/diskfmt)
add_subdirectory(src/kernel)
add_subdirectory(src/io)
add_subdirectory(src/security)

# ---- Win32 subsystem ----
if(LOOS_BUILD_WIN32)
    add_subdirectory(src/win32/apisets)
    add_subdirectory(src/win32/pe)
    add_subdirectory(src/win32/compat)
    add_subdirectory(src/win32/ldr)
endif()

add_subdirectory(src/shell)     # after the Win32 block: the compat console links loos_compat when it exists

# ---- tests ----
if(BUILD_TESTING)
    add_executable(kernel_core_test tests/kernel/core_test.c)
    target_link_libraries(kernel_core_test PRIVATE loos_kernel_core)
    target_compile_options(kernel_core_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME kernel.core_test COMMAND kernel_core_test)

    add_executable(kernel_mm_test tests/kernel/mm_test.c)
    target_link_libraries(kernel_mm_test PRIVATE loos_kernel_mm)
    target_compile_options(kernel_mm_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME kernel.mm_test COMMAND kernel_mm_test)

    add_executable(crypto_block_test tests/kernel/crypto_block_test.c)
    target_link_libraries(crypto_block_test PRIVATE loos_block loos_crypto)
    target_compile_options(crypto_block_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME security.crypto_block_test COMMAND crypto_block_test)

    add_executable(kdf_test tests/kernel/kdf_test.c)
    target_link_libraries(kdf_test PRIVATE loos_crypto)
    target_compile_options(kdf_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME security.kdf_test COMMAND kdf_test)

    add_executable(test_btrfs tests/fs/btrfs/test_btrfs.c)
    target_link_libraries(test_btrfs PRIVATE loos_btrfs)
    target_compile_options(test_btrfs PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME btrfs_fixture
             COMMAND ${CMAKE_COMMAND} -E env python3 ${CMAKE_SOURCE_DIR}/tests/fs/btrfs/make_fixture.py
                      ${CMAKE_CURRENT_BINARY_DIR}/btrfs-fixture.img)
    add_test(NAME btrfs_reader
             COMMAND test_btrfs ${CMAKE_CURRENT_BINARY_DIR}/btrfs-fixture.img --fixture)
    set_tests_properties(btrfs_reader PROPERTIES DEPENDS btrfs_fixture)
    add_test(NAME btrfs_real_reader
             COMMAND ${CMAKE_COMMAND} -E env python3
                     ${CMAKE_SOURCE_DIR}/tests/fs/btrfs/run_real_test.py
                     $<TARGET_FILE:test_btrfs>)
    set_tests_properties(btrfs_real_reader PROPERTIES SKIP_RETURN_CODE 77)

    if(LOOS_BUILD_WIN32)
        add_subdirectory(tests/win32)
    endif()
endif()

# ---- applications (PE targets) ----
if(WIN32 AND LOOS_BUILD_APPS)
    add_subdirectory(apps)
endif()

add_subdirectory(tests/shell)

# privacy gate: no direct network APIs, network libraries or external URLs anywhere in src/ or apps/
find_program(LOOS_PYTHON3 NAMES python3)
if(LOOS_PYTHON3 AND BUILD_TESTING)
    add_test(NAME privacy.lint COMMAND ${LOOS_PYTHON3} ${CMAKE_SOURCE_DIR}/tests/privacy/privacy_lint.py --root ${CMAKE_SOURCE_DIR})
endif()
add_subdirectory(tests/net)
add_subdirectory(tests/pkg)

# Rust crates (kernel components written in Rust): host unit tests via cargo
find_program(LOOS_CARGO NAMES cargo)
if(LOOS_CARGO AND BUILD_TESTING)
    add_test(NAME rust.workspace COMMAND ${LOOS_CARGO} test --manifest-path ${CMAKE_SOURCE_DIR}/crates/Cargo.toml --target-dir ${CMAKE_BINARY_DIR}/crates-target)
endif()

# assembly sources must at least assemble for the bare-metal target
find_program(LOOS_CLANG NAMES clang)
if(LOOS_CLANG AND BUILD_TESTING)
    foreach(asm isr switch syscall ap_tramp)
        add_test(NAME asm.${asm} COMMAND ${LOOS_CLANG} --target=x86_64-unknown-none -c ${CMAKE_SOURCE_DIR}/crates/khal/src/${asm}.S -o ${CMAKE_BINARY_DIR}/asm_${asm}.o)
    endforeach()
endif()

find_program(LOOS_QEMU NAMES qemu-system-x86_64)
if(LOOS_QEMU AND LOOS_CLANG AND BUILD_TESTING)
endif()

# ---- boot loader ----
if(BUILD_TESTING AND LOOS_CLANG)
    add_executable(loader_test tests/boot/loader_test.c boot/common/loader.c boot/common/mmap.c boot/common/elf.c boot/common/paging.c)
    target_compile_definitions(loader_test PRIVATE LOADER_HOST_TEST=1)
    target_compile_options(loader_test PRIVATE -Wall -Wextra -g)
    add_test(NAME boot.loader_core COMMAND sh -c "${CMAKE_SOURCE_DIR}/boot/build-kernel.sh ${CMAKE_BINARY_DIR}/boot >/dev/null && $<TARGET_FILE:loader_test> ${CMAKE_BINARY_DIR}/boot/kernel.elf")
    set_tests_properties(boot.loader_core PROPERTIES RESOURCE_LOCK boot-build SKIP_RETURN_CODE 77)
    if(LOOS_QEMU)
        add_test(NAME boot.bios COMMAND ${CMAKE_SOURCE_DIR}/boot/bios/boot_test.sh ${CMAKE_BINARY_DIR}/boot)
        add_test(NAME boot.uefi COMMAND ${CMAKE_SOURCE_DIR}/boot/uefi/boot_test.sh ${CMAKE_BINARY_DIR}/boot)
        add_test(NAME boot.install COMMAND ${CMAKE_SOURCE_DIR}/boot/bios/install_test.sh ${CMAKE_BINARY_DIR}/boot)
        add_test(NAME boot.install_uefi COMMAND ${CMAKE_SOURCE_DIR}/boot/uefi/install_test.sh ${CMAKE_BINARY_DIR}/boot)
        add_test(NAME boot.install_ui COMMAND python3 ${CMAKE_SOURCE_DIR}/boot/bios/ui_install_test.py ${CMAKE_BINARY_DIR}/boot)
        set_tests_properties(boot.bios boot.uefi boot.install boot.install_uefi boot.install_ui PROPERTIES RESOURCE_LOCK boot-build SKIP_RETURN_CODE 77)
    endif()
endif()

# ---- host snapshot tool + resolution test ----
add_executable(shell_snap tools/shell/shell_snap.c)
target_link_libraries(shell_snap PRIVATE loos_shell)
target_compile_options(shell_snap PRIVATE ${LOOS_C_WARNINGS})
add_executable(installer_snap tools/shell/installer_snap.c)
target_link_libraries(installer_snap PRIVATE loos_shell)
target_compile_options(installer_snap PRIVATE ${LOOS_C_WARNINGS})
add_executable(btrfs_mkfs tools/btrfs_mkfs.c)
target_link_libraries(btrfs_mkfs PRIVATE loos_btrfs)
target_compile_options(btrfs_mkfs PRIVATE ${LOOS_C_WARNINGS})

# btrfs formatter: reader round trip, then the host's own checker as an independent oracle
add_executable(test_mkfs tests/fs/btrfs/test_mkfs.c)
target_link_libraries(test_mkfs PRIVATE loos_btrfs)
target_compile_options(test_mkfs PRIVATE ${LOOS_C_WARNINGS})
add_test(NAME btrfs_mkfs COMMAND test_mkfs)
find_program(LOOS_BTRFS_CHECK NAMES btrfs)
if(LOOS_BTRFS_CHECK)
    add_test(NAME btrfs_mkfs_check COMMAND sh -c "for s in 128 300 4000; do rm -f ${CMAKE_BINARY_DIR}/mkfs.img && $<TARGET_FILE:btrfs_mkfs> ${CMAKE_BINARY_DIR}/mkfs.img $s LoOS && ${LOOS_BTRFS_CHECK} check --readonly ${CMAKE_BINARY_DIR}/mkfs.img > /dev/null || exit 1; done")
endif()
add_executable(diskfmt_tool tools/diskfmt_tool.c)
target_link_libraries(diskfmt_tool PRIVATE loos_diskfmt loos_btrfs)
target_compile_options(diskfmt_tool PRIVATE ${LOOS_C_WARNINGS})
add_test(NAME fs.diskfmt COMMAND ${CMAKE_SOURCE_DIR}/tests/fs/diskfmt_test.sh $<TARGET_FILE:diskfmt_tool> ${CMAKE_BINARY_DIR}/diskfmt)
set_tests_properties(fs.diskfmt PROPERTIES SKIP_RETURN_CODE 77)
