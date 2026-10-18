add_library(loos_apiset STATIC apiset.c apiset_table.c)
target_include_directories(loos_apiset PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
