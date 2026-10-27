# The Rust crates as one static library with a C ABI (crates/ffi). Hosted builds only: the freestanding kernel preset
# has no cross-compiled Rust target yet and keeps the C fallbacks (see docs/RUST-WIRING.md).
find_program(LOOS_CARGO NAMES cargo)
if(NOT LOOS_CARGO)
    message(STATUS "cargo not found: Rust-backed modules fall back to their C implementations")
    return()
endif()

set(LOOS_RUST_TOOLCHAIN "+1.97.1" CACHE STRING "rustup toolchain selector for cargo ('' = whatever cargo resolves)")
set(LOOS_RUST_TARGET_DIR ${CMAKE_BINARY_DIR}/rust-target)
set(LOOS_RUST_LIB ${LOOS_RUST_TARGET_DIR}/release/libloos_ffi.a)
file(GLOB_RECURSE LOOS_RUST_SOURCES CONFIGURE_DEPENDS
     ${CMAKE_SOURCE_DIR}/crates/*/src/*.rs ${CMAKE_SOURCE_DIR}/crates/*/Cargo.toml ${CMAKE_SOURCE_DIR}/crates/Cargo.toml)
list(FILTER LOOS_RUST_SOURCES EXCLUDE REGEX "/(target|vendor)/")

set(_cargo ${LOOS_CARGO})
if(LOOS_RUST_TOOLCHAIN)
    set(_toolchain_arg ${LOOS_RUST_TOOLCHAIN})
endif()
add_custom_command(
    OUTPUT ${LOOS_RUST_LIB}
    COMMAND ${CMAKE_COMMAND} -E env CARGO_TARGET_DIR=${LOOS_RUST_TARGET_DIR}
            ${_cargo} ${_toolchain_arg} build --release --offline -p loos-ffi --manifest-path ${CMAKE_SOURCE_DIR}/crates/Cargo.toml
    DEPENDS ${LOOS_RUST_SOURCES}
    COMMENT "cargo build loos-ffi (Rust crates -> libloos_ffi.a)"
    VERBATIM)
add_custom_target(loos_rust_build DEPENDS ${LOOS_RUST_LIB})

add_library(loos_rust STATIC IMPORTED GLOBAL)
set_target_properties(loos_rust PROPERTIES IMPORTED_LOCATION ${LOOS_RUST_LIB}
                      INTERFACE_INCLUDE_DIRECTORIES ${CMAKE_CURRENT_SOURCE_DIR}
                      INTERFACE_LINK_LIBRARIES "pthread;dl;m;gcc_s")
add_dependencies(loos_rust loos_rust_build)
set(LOOS_HAVE_RUST ON CACHE INTERNAL "Rust FFI library is available")
