add_library(loos_net STATIC netutil.c dns.c dhcp.c iphelp.c fw.c http.c)
target_include_directories(loos_net PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(loos_net PRIVATE ${LOOS_C_WARNINGS})
