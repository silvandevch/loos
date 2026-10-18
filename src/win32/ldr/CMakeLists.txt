add_library(loos_ldr STATIC ldr.c)
target_include_directories(loos_ldr PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_link_libraries(loos_ldr PUBLIC loos_pe loos_apiset)
