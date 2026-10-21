add_library(loos_diskfmt STATIC gpt.c fat32.c)
target_include_directories(loos_diskfmt PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_diskfmt PRIVATE ${LOOS_C_WARNINGS})
