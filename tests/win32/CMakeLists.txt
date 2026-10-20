# Host-side tests for the user-mode Win32 pieces (reference: tests/win32/run.sh, which adds ASan/UBSan).
# The kernel32 and ntdll tests call Windows-ABI code through ms_abi, so they need an x86-64 host.
set(W32 ${CMAKE_SOURCE_DIR}/src/win32)

find_package(Threads REQUIRED)

function(loos_win32_test name)
    cmake_parse_arguments(T "" "" "SOURCES;LIBS;DEFS;INCLUDES;ARGS" ${ARGN})
    add_executable(${name} ${T_SOURCES})
    target_include_directories(${name} PRIVATE ${T_INCLUDES})
    target_compile_definitions(${name} PRIVATE ${T_DEFS})
    target_link_libraries(${name} PRIVATE ${T_LIBS} Threads::Threads)
    set_target_properties(${name} PROPERTIES C_STANDARD 11 C_EXTENSIONS ON)
    add_test(NAME win32.${name} COMMAND ${name} ${T_ARGS})
endfunction()

# wine's builtin PE DLLs, when installed, give the PE parser real-world input
file(GLOB LOOS_WINE_PE_DIRS "/usr/lib64/wine*/wine/x86_64-windows" "/usr/lib64/wine*/wine/i386-windows")
set(LOOS_WINE_PE_IMAGES)
foreach(d IN LISTS LOOS_WINE_PE_DIRS)
    foreach(n kernel32 ntdll user32)
        if(EXISTS "${d}/${n}.dll")
            list(APPEND LOOS_WINE_PE_IMAGES "${d}/${n}.dll")
        endif()
    endforeach()
endforeach()
list(REMOVE_DUPLICATES LOOS_WINE_PE_IMAGES)

loos_win32_test(apiset_test SOURCES apisets/apiset_test.c LIBS loos_apiset)
loos_win32_test(compat_test SOURCES compat/compat_test.c LIBS loos_compat)
loos_win32_test(pe_test SOURCES pe/pe_test.c LIBS loos_pe ARGS ${LOOS_WINE_PE_IMAGES})
loos_win32_test(ldr_test SOURCES ldr/ldr_test.c common/pebuild.c LIBS loos_ldr loos_compat)

if(CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
    set(K32_SRC ${W32}/kernel32/ansi.c ${W32}/kernel32/heap.c ${W32}/ntdll/heap.c ${W32}/ntdll/sync.c ${W32}/kernel32/locks.c ${W32}/kernel32/tls.c ${W32}/kernel32/strings.c ${W32}/kernel32/thread.c ${W32}/kernel32/files2.c ${W32}/kernel32/message.c ${W32}/kernel32/mapping.c ${W32}/kernel32/paths.c ${W32}/kernel32/createproc.c ${W32}/ntdll/procparams.c ${W32}/ntdll/rtl.c ${W32}/ntdll/delayload.c ${W32}/kernel32/console.c ${W32}/kernel32/env.c ${W32}/kernel32/misc.c ${W32}/kernel32/path.c ${W32}/kernel32/file.c ${W32}/kernel32/mem.c ${W32}/kernel32/sync.c
                ${W32}/kernel32/proc.c ${W32}/kernel32/version.c ${W32}/ntdll/errmap.c ${W32}/ntdll/version.c)
    loos_win32_test(kernel32_test SOURCES kernel32/k32_test.c kernel32/nt_fake.c ${K32_SRC}
                    LIBS loos_compat DEFS LOOS_HOST_TEST INCLUDES ${W32}/include)
    loos_win32_test(ansi_adapter_test SOURCES kernel32/ansi_test.c kernel32/nt_fake.c ${K32_SRC}
                    LIBS loos_compat DEFS LOOS_HOST_TEST INCLUDES ${W32}/include)
    loos_win32_test(sync_test SOURCES kernel32/sync_test.c kernel32/nt_fake.c ${W32}/kernel32/locks.c ${W32}/kernel32/tls.c ${W32}/kernel32/proc.c
                    ${W32}/ntdll/sync.c ${W32}/ntdll/heap.c ${W32}/ntdll/errmap.c DEFS LOOS_HOST_TEST INCLUDES ${W32}/include)
    loos_win32_test(heap_test SOURCES ntdll/heap_test.c kernel32/nt_fake.c ${W32}/ntdll/heap.c DEFS LOOS_HOST_TEST FAKE_NO_LDR INCLUDES ${W32}/include)
    loos_win32_test(ldr_api_test SOURCES ntdll/ldr_api_test.c kernel32/nt_fake.c common/pebuild.c ${W32}/ntdll/sync.c ${W32}/ntdll/heap.c ${W32}/ntdll/rtl.c ${W32}/ntdll/ldr_api.c
                    ${W32}/ntdll/version.c ${W32}/ntdll/errmap.c
                    LIBS loos_ldr loos_compat DEFS LOOS_HOST_TEST FAKE_NO_LDR INCLUDES ${W32}/include)
endif()

# Real linked system DLLs: needs clang for the Windows target and an LLD in link mode.
find_program(LOOS_LLD NAMES lld-link)
file(GLOB LOOS_RUST_LLD "$ENV{HOME}/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld")
if(CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64" AND (LOOS_LLD OR LOOS_RUST_LLD OR DEFINED ENV{LLD}))
    set(PE_OUT ${CMAKE_CURRENT_BINARY_DIR}/pe)
    add_custom_target(win32_pe_images
        COMMAND ${CMAKE_SOURCE_DIR}/scripts/win32/build-pe.sh ${PE_OUT}
        COMMENT "Building ntdll.dll and kernel32.dll (PE32+)" VERBATIM)
    loos_win32_test(artifacts_test SOURCES pe/artifacts_test.c LIBS loos_ldr ARGS ${PE_OUT})
    add_dependencies(artifacts_test win32_pe_images)
endif()

# Run a *real* PE32+ executable through the loader: build loos-echo.exe + its
# losshost.dll with MinGW, then map/bind/run the exe's entry point on the host.
find_program(LOOS_MINGW_CC NAMES x86_64-w64-mingw32-gcc)
if(LOOS_MINGW_CC)
    set(EXEC_OUT ${CMAKE_CURRENT_BINARY_DIR}/exec)
    add_custom_target(win32_exec_demo
        COMMAND ${CMAKE_COMMAND} -E env MINGW_CC=${LOOS_MINGW_CC}
                bash ${CMAKE_CURRENT_SOURCE_DIR}/exec/build-demo.sh ${EXEC_OUT}
        COMMENT "Building pe_exec demo PE images (losshost.dll, loos-echo.exe)" VERBATIM)
    loos_win32_test(pe_exec SOURCES exec/pe_exec.c LIBS loos_ldr loos_compat ARGS ${EXEC_OUT})
    add_dependencies(pe_exec win32_exec_demo)
endif()

# Run a *real, unmodified* Win32 app (GetStdHandle/WriteFile from kernel32.dll)
# on the LoOS compat layer: LoOS loader binds it to the LoOS kernel32 + Nt* path.
if(LOOS_MINGW_CC AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
    set(APP_OUT ${CMAKE_CURRENT_BINARY_DIR}/app)
    add_custom_target(win32_app_demo
        COMMAND ${CMAKE_COMMAND} -E env MINGW_CC=${LOOS_MINGW_CC}
                bash ${CMAKE_CURRENT_SOURCE_DIR}/app/build-app.sh ${APP_OUT}
        COMMENT "Building Win32 app-compat proof (compat-hello.exe, k32shim.dll)" VERBATIM)
    loos_win32_test(app_compat SOURCES app/app_compat.c kernel32/nt_fake.c ${K32_SRC}
                    LIBS loos_ldr loos_pe loos_apiset loos_compat DEFS LOOS_HOST_TEST
                    INCLUDES ${CMAKE_CURRENT_SOURCE_DIR}/kernel32 ${W32}/include ${W32}/ldr ${W32}/pe ${W32}/apisets
                    ARGS ${APP_OUT})
    add_dependencies(app_compat win32_app_demo)
endif()
