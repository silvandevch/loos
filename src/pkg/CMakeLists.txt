add_library(loos_pkg STATIC deb.c dpkgdb.c solver.c apt.c pgp.c)
target_include_directories(loos_pkg PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_pkg PRIVATE ${LOOS_C_WARNINGS})
target_link_libraries(loos_pkg PUBLIC loos_crypto)
if(TARGET loos_rust)
    # Debian version ordering comes from crates/dpkg. The solver, apt engine and PGP stay C: they are broader than the
    # Rust resolver (multiarch, pins, holds, autoremove, upgrade-all) and are covered by tests/pkg.
    target_compile_definitions(loos_pkg PRIVATE LOOS_RUST_DPKG=1)
    target_link_libraries(loos_pkg PUBLIC loos_rust)
endif()
