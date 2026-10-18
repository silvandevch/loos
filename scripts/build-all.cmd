@echo off
rem build-all.cmd - entry point for the build.
rem On this host razzle builds AMD64 natively, so a single pass is correct
rem (see build-amd64.cmd for the x86-host cross alternative).
setlocal
echo ################################################################
echo # LoOS NT 5.2 x64 build   (native AMD64 under Wine)
echo ################################################################

call "%~dp0build-amd64.cmd"
set "RC=%errorlevel%"
if not "%RC%"=="0" goto BuildFailed

echo [OK] build completed
exit /b 0

:BuildFailed
echo [FAIL] amd64 build failed with exit code %RC%
exit /b %RC%
