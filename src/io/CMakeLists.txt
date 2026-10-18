add_library(loos_block STATIC block.c)
target_include_directories(loos_block PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_block PRIVATE ${LOOS_C_WARNINGS})
