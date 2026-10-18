# Host-side tests for the user-mode Win32 pieces (reference: tests/win32/run.sh, which adds ASan/UBSan).
# The kernel32 and ntdll tests call Windows-ABI code through ms_abi, so they need an x86-64 host.
set(W32 ${CMAKE_SOURCE_DIR}/src/win32)

function(loos_win32_test name)
    cmake_parse_arguments(T "" "" "SOURCES;LIBS;DEFS;INCLUDES;ARGS" ${ARGN})
    add_executable(${name} ${T_SOURCES})
    target_include_directories(${name} PRIVATE ${T_INCLUDES})
    target_compile_definitions(${name} PRIVATE ${T_DEFS})
    target_link_libraries(${name} PRIVATE ${T_LIBS})
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
    set(K32_SRC ${W32}/kernel32/path.c ${W32}/kernel32/file.c ${W32}/kernel32/mem.c ${W32}/kernel32/sync.c
                ${W32}/kernel32/proc.c ${W32}/kernel32/version.c ${W32}/ntdll/errmap.c ${W32}/ntdll/version.c)
    loos_win32_test(kernel32_test SOURCES kernel32/k32_test.c kernel32/nt_fake.c ${K32_SRC}
                    LIBS loos_compat DEFS LOOS_HOST_TEST INCLUDES ${W32}/include)
    loos_win32_test(ldr_api_test SOURCES ntdll/ldr_api_test.c kernel32/nt_fake.c common/pebuild.c ${W32}/ntdll/ldr_api.c
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
