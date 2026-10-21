add_library(loos_shell STATIC installer.c desktop.c draw.c compositor.c font.c raster.c font_data.c inventory.c launcher.c taskbar.c wm.c ui.c backend_mem.c settings.c apphost.c util.c filemgr.c taskmon.c session.c perms.c notify.c)
target_include_directories(loos_shell PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_shell PRIVATE ${LOOS_C_WARNINGS})
target_link_libraries(loos_shell PUBLIC loos_crypto)

if(TARGET loos_compat)
    target_sources(loos_shell PRIVATE compat_console.c)
    target_link_libraries(loos_shell PUBLIC loos_compat)
endif()
