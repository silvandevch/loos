add_library(loos_shell STATIC desktop.c backend_mem.c apphost.c)
target_include_directories(loos_shell PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_shell PRIVATE ${LOOS_C_WARNINGS})
target_link_libraries(loos_shell PUBLIC loos_crypto)

if(TARGET loos_compat)
    target_sources(loos_shell PRIVATE compat_console.c)
    target_link_libraries(loos_shell PUBLIC loos_compat)
endif()
