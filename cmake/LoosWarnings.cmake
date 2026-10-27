# LoOS shared compiler-warning flags.
#
# gcc/clang (GNU drivers) accept -Wall -Wextra -Werror -pedantic.
# clang-cl (MSVC-simulated) rejects -pedantic, so use MSVC spelling there.
if(MSVC)
    set(LOOS_C_WARNINGS "")
else()
    set(LOOS_C_WARNINGS -Wall -Wextra -Werror -pedantic)
    if(CMAKE_C_COMPILER_ID STREQUAL "GNU")
        # Recent gcc flags the project's compact one-line statement style (clang does not); format-truncation fires on
        # snprintf into deliberately small display buffers.
        list(APPEND LOOS_C_WARNINGS -Wno-misleading-indentation -Wno-format-truncation)
    endif()
endif()
