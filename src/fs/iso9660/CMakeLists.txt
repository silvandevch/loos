add_library(loos_iso9660 STATIC iso9660.c)
target_include_directories(loos_iso9660 PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_iso9660 PRIVATE ${LOOS_C_WARNINGS})

if(TARGET loos_kcrt)
    target_link_libraries(loos_iso9660 PUBLIC loos_kcrt)
endif()
