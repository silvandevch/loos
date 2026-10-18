set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

find_program(LOOS_CLANG clang REQUIRED)
set(CMAKE_C_COMPILER "${LOOS_CLANG}" CACHE FILEPATH "Clang C compiler")
set(CMAKE_C_COMPILER_TARGET x86_64-unknown-none-elf)
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

set(CMAKE_C_FLAGS_INIT "-ffreestanding -fno-stack-protector -fno-pic -mno-red-zone")
