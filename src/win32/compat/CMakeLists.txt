add_library(loos_compat STATIC compat.c)
target_include_directories(loos_compat PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
