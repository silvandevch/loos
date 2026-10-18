cmake_minimum_required(VERSION 3.25)
project(LoOS LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

option(LOOS_BUILD_WIN32 "Build the Win32 subsystem libraries and their host tests" ON)
option(LOOS_BUILD_APPS "Build the LoOS desktop applications (windows-msvc preset only)" OFF)

include(CTest)

# ---- filesystems ----
add_subdirectory(src/fs/btrfs)

# ---- Win32 subsystem ----
if(LOOS_BUILD_WIN32)
    add_subdirectory(src/win32/apisets)
    add_subdirectory(src/win32/pe)
    add_subdirectory(src/win32/compat)
    add_subdirectory(src/win32/ldr)
endif()

# ---- tests ----
if(BUILD_TESTING)
    add_executable(test_btrfs tests/fs/btrfs/test_btrfs.c)
    target_link_libraries(test_btrfs PRIVATE loos_btrfs)
    target_compile_options(test_btrfs PRIVATE -Wall -Wextra -Werror -pedantic)
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
