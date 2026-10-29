# CMake generated Testfile for 
# Source directory: /run/media/silvan/DATA/LoOS/tests/win32
# Build directory: /run/media/silvan/DATA/LoOS/build-rust/tests/win32
# 
# This file includes the relevant testing commands required for 
# testing this directory and lists subdirectories to be tested as well.
add_test([=[win32.apiset_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/apiset_test")
set_tests_properties([=[win32.apiset_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;29;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.compat_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/compat_test")
set_tests_properties([=[win32.compat_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;30;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.pe_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/pe_test" "/usr/lib64/wine-wow64/wine/i386-windows/kernel32.dll" "/usr/lib64/wine-wow64/wine/i386-windows/ntdll.dll" "/usr/lib64/wine-wow64/wine/i386-windows/user32.dll" "/usr/lib64/wine-wow64/wine/x86_64-windows/kernel32.dll" "/usr/lib64/wine-wow64/wine/x86_64-windows/ntdll.dll" "/usr/lib64/wine-wow64/wine/x86_64-windows/user32.dll")
set_tests_properties([=[win32.pe_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;31;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.ldr_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/ldr_test")
set_tests_properties([=[win32.ldr_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;32;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.kernel32_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/kernel32_test")
set_tests_properties([=[win32.kernel32_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;37;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.ansi_adapter_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/ansi_adapter_test")
set_tests_properties([=[win32.ansi_adapter_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;39;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.sync_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/sync_test")
set_tests_properties([=[win32.sync_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;41;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.heap_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/heap_test")
set_tests_properties([=[win32.heap_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;43;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.ldr_api_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/ldr_api_test")
set_tests_properties([=[win32.ldr_api_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;44;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.artifacts_test]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/artifacts_test" "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/pe")
set_tests_properties([=[win32.artifacts_test]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;57;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.pe_exec]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/pe_exec" "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/exec")
set_tests_properties([=[win32.pe_exec]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;70;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.app_compat]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/app_compat" "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/app")
set_tests_properties([=[win32.app_compat]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;82;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
add_test([=[win32.ucrt_app_compat]=] "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/ucrt_app_compat" "/run/media/silvan/DATA/LoOS/build-rust/tests/win32/app" "ucrt-hello.exe")
set_tests_properties([=[win32.ucrt_app_compat]=] PROPERTIES  _BACKTRACE_TRIPLES "/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;14;add_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;89;loos_win32_test;/run/media/silvan/DATA/LoOS/tests/win32/CMakeLists.txt;0;")
