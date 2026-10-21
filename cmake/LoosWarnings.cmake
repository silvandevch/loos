# LoOS shared compiler-warning flags.
#
# gcc/clang (GNU drivers) accept -Wall -Wextra -Werror -pedantic.
# clang-cl (MSVC-simulated) rejects -pedantic, so use MSVC spelling there.
if(MSVC)
    set(LOOS_C_WARNINGS "")
else()
    set(LOOS_C_WARNINGS -Wall -Wextra -Werror -pedantic)
endif()
