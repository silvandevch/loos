@echo off
rem build-amd64.cmd - initialise a razzle build environment and compile the
rem x64 (AMD64) image, capped to 4 threads.
rem
rem Arch selection follows razzle.cmd:
rem   * On an AMD64 host (native), no arch switch is needed - razzle takes its
rem     Native Support path and sets _BuildArch=amd64 from
rem     %PROCESSOR_ARCHITECTURE%.  This is the path Wine takes (it reports AMD64).
rem   * On an x86 host, pass "win64 amd64" - razzle requires Win64 to gate AMD64
rem     and then selects tools\win64\x86\amd64 as the cross toolchain.
rem
rem Requires NT_SRC (source tree) and LOOS_ROOT (workspace); 09-build-all.sh sets
rem both.  CFG_CROSS=1 forces the cross ("win64 amd64") form.
setlocal
if "%NT_SRC%"==""    set "NT_SRC=%~dp0..\srv03rtm"
if "%LOOS_ROOT%"=="" set "LOOS_ROOT=%~dp0.."

if not exist "%NT_SRC%\tools\razzle.cmd" echo [FAIL] missing razzle.cmd & exit /b 3
cd /d "%NT_SRC%"

if "%CFG_CROSS%"=="1" (
    set "RAZZLE_ARGS=free offline no_certcheck win64 amd64"
) else (
    set "RAZZLE_ARGS=free offline no_certcheck"
)

echo [build-amd64] SDXROOT=%NT_SRC%
echo [build-amd64] args   =%RAZZLE_ARGS%
call "%NT_SRC%\tools\razzle.cmd" %RAZZLE_ARGS% exec "%LOOS_ROOT%\scripts\_build-pass.cmd"
set RC=%errorlevel%
echo [build-amd64] exit=%RC%
exit /b %RC%
