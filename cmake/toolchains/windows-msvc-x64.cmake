set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR AMD64)

find_program(LOOS_CLANG_CL clang-cl REQUIRED)
find_program(LOOS_COFF_LIB NAMES llvm-lib lib.exe lib REQUIRED)
find_program(LOOS_RC NAMES llvm-rc rc.exe rc REQUIRED)

set(CMAKE_C_COMPILER "${LOOS_CLANG_CL}" CACHE FILEPATH "clang-cl C compiler")
set(CMAKE_C_COMPILER_TARGET x86_64-pc-windows-msvc)
set(CMAKE_AR "${LOOS_COFF_LIB}" CACHE FILEPATH "COFF librarian")
set(CMAKE_RC_COMPILER "${LOOS_RC}" CACHE FILEPATH "Windows resource compiler")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# The Windows SDK and MSVC-compatible libraries must be discoverable through
# the host environment or CMake's standard Windows SDK detection.
