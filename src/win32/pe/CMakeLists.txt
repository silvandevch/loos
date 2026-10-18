add_library(loos_pe STATIC pe.c)
target_include_directories(loos_pe PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_link_libraries(loos_pe PUBLIC loos_apiset)
