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
add_subdirectory(src/fs/btrfs)
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
