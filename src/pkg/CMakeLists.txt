add_library(loos_pkg STATIC deb.c dpkgdb.c solver.c apt.c pgp.c)
target_include_directories(loos_pkg PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_pkg PRIVATE ${LOOS_C_WARNINGS})
target_link_libraries(loos_pkg PUBLIC loos_crypto)
