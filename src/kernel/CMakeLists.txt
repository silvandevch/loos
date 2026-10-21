add_library(loos_kernel_core STATIC core.c)
target_include_directories(loos_kernel_core PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_kernel_core PRIVATE ${LOOS_C_WARNINGS})

add_library(loos_kernel_mm STATIC mm.c)
target_include_directories(loos_kernel_mm PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_kernel_mm PRIVATE ${LOOS_C_WARNINGS})

if(TARGET loos_kcrt)
    target_link_libraries(loos_kernel_core PUBLIC loos_kcrt)
endif()

if(TARGET loos_kcrt)
    target_link_libraries(loos_kernel_mm PUBLIC loos_kcrt)
endif()
