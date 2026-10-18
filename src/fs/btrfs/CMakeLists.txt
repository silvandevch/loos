add_library(loos_btrfs STATIC btrfs.c)
target_include_directories(loos_btrfs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_btrfs PRIVATE -Wall -Wextra -Werror -pedantic)
