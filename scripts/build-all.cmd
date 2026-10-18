@echo off
rem build-all.cmd - entry point for the build.
rem On this host razzle builds AMD64 natively, so a single pass is correct
rem (see build-amd64.cmd for the x86-host cross alternative).
setlocal
echo ################################################################
echo # LoOS NT 5.2 x64 build   (native AMD64 under Wine)
echo ################################################################

call "%~dp0build-amd64.cmd"
if errorlevel 1 echo [FAIL] amd64 build failed & exit /b 1

echo [OK] build completed
exit /b 0
