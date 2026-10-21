add_library(loos_crypto STATIC crypto.c kdf.c)
target_include_directories(loos_crypto PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_crypto PRIVATE ${LOOS_C_WARNINGS})

if(TARGET loos_kcrt)
    target_link_libraries(loos_crypto PUBLIC loos_kcrt)
endif()
