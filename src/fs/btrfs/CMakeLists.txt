add_library(loos_btrfs STATIC btrfs.c mkfs.c)
target_include_directories(loos_btrfs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_btrfs PRIVATE ${LOOS_C_WARNINGS})

add_library(loos_btrfs_tools STATIC tools/integrity.c)
target_include_directories(loos_btrfs_tools PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/tools)
target_compile_options(loos_btrfs_tools PRIVATE ${LOOS_C_WARNINGS})
target_link_libraries(loos_btrfs_tools PUBLIC loos_btrfs)

if(TARGET loos_kcrt)
    target_link_libraries(loos_btrfs PUBLIC loos_kcrt)
endif()
