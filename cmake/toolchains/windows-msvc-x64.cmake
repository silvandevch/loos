set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR AMD64)

# LLVM's MSVC-ABI tools (llvm-lib / llvm-rc / lld-link) are not always on PATH
# even when clang-cl is. Look in the usual places too (Fedora's standalone rocm
# LLVM bundle ships them).
set(_loos_llvm_hints
  /usr/lib64/rocm/llvm/bin
  /usr/lib/llvm/bin
  /usr/lib64/llvm/bin
  /usr/local/opt/llvm/bin)

find_program(LOOS_CLANG_CL clang-cl REQUIRED)
find_program(LOOS_COFF_LIB NAMES llvm-lib lib.exe lib REQUIRED HINTS ${_loos_llvm_hints})
find_program(LOOS_RC NAMES llvm-rc rc.exe rc REQUIRED HINTS ${_loos_llvm_hints})
find_program(LOOS_LLD_LINK NAMES lld-link REQUIRED HINTS ${_loos_llvm_hints})

set(CMAKE_C_COMPILER "${LOOS_CLANG_CL}" CACHE FILEPATH "clang-cl C compiler")
set(CMAKE_C_COMPILER_TARGET x86_64-pc-windows-msvc)
set(CMAKE_AR "${LOOS_COFF_LIB}" CACHE FILEPATH "COFF librarian")
set(CMAKE_RC_COMPILER "${LOOS_RC}" CACHE FILEPATH "Windows resource compiler")
set(CMAKE_LINKER "${LOOS_LLD_LINK}" CACHE FILEPATH "COFF linker")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# clang-cl defaults to MSVC's link.exe; use the LLVM linker instead.
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-fuse-ld=lld")

# The Windows SDK and MSVC-compatible libraries must be discoverable through
# the host environment or CMake's standard Windows SDK detection.

# No MSVC CRT headers are assumed; the subsystem supplies its own string.h /
# stdlib.h via the shared freestanding headers.
get_filename_component(LOOS_FREESTANDING_INCLUDE
  "${CMAKE_CURRENT_LIST_DIR}/../freestanding/include" ABSOLUTE)
set(CMAKE_C_FLAGS_INIT "-imsvc${LOOS_FREESTANDING_INCLUDE}")
