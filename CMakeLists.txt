cmake_minimum_required(VERSION 3.25)
project(LoOS LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

option(LOOS_BUILD_WIN32 "Build the Win32 subsystem libraries and their host tests" ON)
option(LOOS_BUILD_APPS "Build the LoOS desktop applications (windows-msvc preset only)" OFF)

# The kernel preset uses CMAKE_SYSTEM_NAME=Generic and has no hosted libc.  Keep
# host-only modules, tests, boot harnesses, and snapshot tools out of that build;
# otherwise CMake tries to compile their stdio/POSIX dependencies for the
# bare-metal target.
set(LOOS_FREESTANDING_BUILD OFF)
if(CMAKE_SYSTEM_NAME STREQUAL "Generic")
    set(LOOS_FREESTANDING_BUILD ON)
endif()

include(${CMAKE_CURRENT_LIST_DIR}/cmake/LoosWarnings.cmake)

include(CTest)

# Rust crates and the remaining C modules call each other across static archives
# (Rust -> loos_shell, C -> loos_rust). Group them so the linker rescans both.
if(NOT LOOS_FREESTANDING_BUILD)
    add_link_options("LINKER:--start-group")
    set(CMAKE_C_STANDARD_LIBRARIES "${CMAKE_C_STANDARD_LIBRARIES} -Wl,--end-group")
endif()

# ---- Rust crates behind a C ABI (hosted builds) ----
if(NOT LOOS_FREESTANDING_BUILD AND NOT WIN32)
    add_subdirectory(src/rust)
endif()

# ---- filesystems ----
add_subdirectory(src/libc)
add_subdirectory(src/fs/btrfs)
add_subdirectory(src/fs/iso9660)
add_subdirectory(src/kernel)
add_subdirectory(src/io)
add_subdirectory(src/security)
if(NOT LOOS_FREESTANDING_BUILD AND NOT WIN32)
    add_subdirectory(src/net)
    add_subdirectory(src/pkg)
    add_subdirectory(src/fs/diskfmt)
endif()

# ---- Win32 subsystem ----
if(LOOS_BUILD_WIN32 AND NOT LOOS_FREESTANDING_BUILD)
    add_subdirectory(src/win32/apisets)
    add_subdirectory(src/win32/pe)
    add_subdirectory(src/win32/compat)
    add_subdirectory(src/win32/ldr)
endif()

# ---- tests ----
if(BUILD_TESTING AND NOT LOOS_FREESTANDING_BUILD)
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

    add_executable(iso9660_test tests/fs/iso9660_test.c)
    target_link_libraries(iso9660_test PRIVATE loos_iso9660)
    target_compile_options(iso9660_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME fs.iso9660 COMMAND iso9660_test)

    if(TARGET loos_rust)
        add_executable(rust_ffi_test tests/rust/ffi_test.c)
        target_link_libraries(rust_ffi_test PRIVATE loos_rust loos_kernel_core)
        target_compile_options(rust_ffi_test PRIVATE ${LOOS_C_WARNINGS})
        add_test(NAME rust.ffi COMMAND rust_ffi_test)

        add_executable(rust_ffi_btrfs_test tests/rust/ffi_btrfs_test.c)
        target_link_libraries(rust_ffi_btrfs_test PRIVATE loos_rust)
        target_compile_options(rust_ffi_btrfs_test PRIVATE ${LOOS_C_WARNINGS})
        add_test(NAME rust.ffi_btrfs COMMAND ${CMAKE_COMMAND} -E env python3 ${CMAKE_SOURCE_DIR}/tests/rust/run_btrfs_ffi.py $<TARGET_FILE:rust_ffi_btrfs_test>)
        set_tests_properties(rust.ffi_btrfs PROPERTIES SKIP_RETURN_CODE 77)
    endif()

    if(LOOS_BUILD_WIN32)
        add_subdirectory(tests/win32)
    endif()
endif()

# ---- applications (PE targets) ----
if(WIN32 AND LOOS_BUILD_APPS AND NOT LOOS_FREESTANDING_BUILD)
    add_subdirectory(apps)
endif()


# privacy gate: no direct network APIs, network libraries or external URLs anywhere in src/ or apps/
find_program(LOOS_PYTHON3 NAMES python3)
if(LOOS_PYTHON3 AND BUILD_TESTING)
    add_test(NAME privacy.lint COMMAND ${LOOS_PYTHON3} ${CMAKE_SOURCE_DIR}/tests/privacy/privacy_lint.py --root ${CMAKE_SOURCE_DIR})
endif()
if(NOT LOOS_FREESTANDING_BUILD)
    add_subdirectory(tests/net)
    add_subdirectory(tests/pkg)
endif()

# Rust crates (kernel components written in Rust): host unit tests via cargo
find_program(LOOS_CARGO NAMES cargo)
if(LOOS_CARGO AND BUILD_TESTING)
    add_test(NAME rust.workspace COMMAND ${CMAKE_COMMAND} -E chdir ${CMAKE_SOURCE_DIR} ${LOOS_CARGO} test --manifest-path ${CMAKE_SOURCE_DIR}/crates/Cargo.toml --target-dir ${CMAKE_BINARY_DIR}/crates-target --locked --offline)
endif()

# assembly sources must at least assemble for the bare-metal target
find_program(LOOS_CLANG NAMES clang)
if(LOOS_CLANG AND BUILD_TESTING)
    foreach(asm isr switch syscall ap_tramp)
        add_test(NAME asm.${asm} COMMAND ${LOOS_CLANG} --target=x86_64-unknown-none -c ${CMAKE_SOURCE_DIR}/crates/khal/src/${asm}.S -o ${CMAKE_BINARY_DIR}/asm_${asm}.o)
    endforeach()
endif()

# The boot sources already build a complete BIOS image through the checked-in
# script. Expose that path as a first-class host build target without pulling
# freestanding-only boot inputs into the normal CMake graph.
if(NOT LOOS_FREESTANDING_BUILD AND NOT WIN32 AND LOOS_CLANG)
    set(LOOS_BIOS_IMAGE "${CMAKE_BINARY_DIR}/boot/bios.img")
    add_custom_command(
        OUTPUT "${LOOS_BIOS_IMAGE}"
        COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/boot"
        COMMAND ${CMAKE_COMMAND} -E env
                "CC=${LOOS_CLANG}"
                bash "${CMAKE_SOURCE_DIR}/boot/bios/build.sh"
                "${CMAKE_BINARY_DIR}/boot"
                --cmdline "console=ttyS0 loos.test=1"
        DEPENDS
            "${CMAKE_SOURCE_DIR}/boot/bios/build.sh"
            "${CMAKE_SOURCE_DIR}/boot/build-kernel.sh"
        VERBATIM)
    add_custom_target(loos_boot_bios DEPENDS "${LOOS_BIOS_IMAGE}")
endif()

find_program(LOOS_QEMU NAMES qemu-system-x86_64)
if(LOOS_QEMU AND LOOS_CLANG AND BUILD_TESTING)
endif()

# ---- boot loader ----
if(BUILD_TESTING AND LOOS_CLANG AND NOT LOOS_FREESTANDING_BUILD)
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

# ---- host snapshot tools + resolution tests ----
if(NOT LOOS_FREESTANDING_BUILD AND NOT WIN32)
    add_executable(btrfs_mkfs tools/btrfs_mkfs.c)
    target_link_libraries(btrfs_mkfs PRIVATE loos_btrfs)
    target_compile_options(btrfs_mkfs PRIVATE ${LOOS_C_WARNINGS})
    add_executable(btrfs_check tools/btrfs_check.c)
    target_link_libraries(btrfs_check PRIVATE loos_btrfs_tools)
    target_compile_options(btrfs_check PRIVATE ${LOOS_C_WARNINGS})

    # btrfs formatter: reader round trip, then the host's own checker as an independent oracle
    add_executable(test_mkfs tests/fs/btrfs/test_mkfs.c)
    target_link_libraries(test_mkfs PRIVATE loos_btrfs)
    target_compile_options(test_mkfs PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME btrfs_mkfs COMMAND test_mkfs)

    add_executable(test_btrfs_integrity tests/fs/btrfs/test_integrity.c)
    target_link_libraries(test_btrfs_integrity PRIVATE loos_btrfs_tools)
    target_compile_options(test_btrfs_integrity PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME btrfs_integrity COMMAND test_btrfs_integrity)
    add_test(NAME btrfs_check_tool
             COMMAND sh ${CMAKE_SOURCE_DIR}/tests/fs/btrfs/test_tool.sh
                     $<TARGET_FILE:btrfs_check>
                     $<TARGET_FILE:btrfs_mkfs>
                     ${CMAKE_CURRENT_BINARY_DIR}/btrfs-tool-test)

    find_program(LOOS_BTRFS_CHECK NAMES btrfs)
    if(LOOS_BTRFS_CHECK)
        add_test(NAME btrfs_mkfs_check COMMAND sh -c "for s in 128 300 4000; do rm -f ${CMAKE_BINARY_DIR}/mkfs.img && $<TARGET_FILE:btrfs_mkfs> ${CMAKE_BINARY_DIR}/mkfs.img $s LoOS && ${LOOS_BTRFS_CHECK} check --readonly ${CMAKE_BINARY_DIR}/mkfs.img > /dev/null || exit 1; done")
    endif()
    add_executable(diskfmt_tool tools/diskfmt_tool.c)
    target_link_libraries(diskfmt_tool PRIVATE loos_diskfmt loos_btrfs)
    target_compile_options(diskfmt_tool PRIVATE ${LOOS_C_WARNINGS})
    add_executable(diskfmt_unit_test tests/fs/diskfmt_unit_test.c)
    target_link_libraries(diskfmt_unit_test PRIVATE loos_diskfmt)
    target_compile_options(diskfmt_unit_test PRIVATE ${LOOS_C_WARNINGS})
    add_test(NAME fs.diskfmt_unit COMMAND diskfmt_unit_test)
    add_test(NAME fs.diskfmt COMMAND ${CMAKE_SOURCE_DIR}/tests/fs/diskfmt_test.sh $<TARGET_FILE:diskfmt_tool> ${CMAKE_BINARY_DIR}/diskfmt)
    set_tests_properties(fs.diskfmt PROPERTIES SKIP_RETURN_CODE 77)
endif()
