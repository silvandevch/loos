if(TARGET loos_rust)
    # Hosted builds: SHA-256 / HMAC / PBKDF2 come from crates/loos-security through the C ABI (same symbols and header).
    add_library(loos_crypto INTERFACE)
    target_include_directories(loos_crypto INTERFACE ${CMAKE_CURRENT_SOURCE_DIR})
    target_compile_definitions(loos_crypto INTERFACE LOOS_RUST_CRYPTO=1)
    target_link_libraries(loos_crypto INTERFACE loos_rust)
else()
    # Freestanding kernel preset (no Rust target yet): the original C implementation.
    add_library(loos_crypto STATIC crypto.c kdf.c)
    target_include_directories(loos_crypto PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
    target_compile_options(loos_crypto PRIVATE ${LOOS_C_WARNINGS})

    if(TARGET loos_kcrt)
        target_link_libraries(loos_crypto PUBLIC loos_kcrt)
    endif()
endif()
