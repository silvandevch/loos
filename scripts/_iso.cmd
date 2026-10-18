@echo off
rem _iso.cmd - build a bootable ISO inside an initialised razzle environment,
rem using the tree's own oscdimg.  Invoked via:
rem   razzle.cmd free offline no_certcheck exec _iso.cmd
rem Inputs (env): VARIANT (SKU dir, default srv), ISO_OUT (output iso path)
if "%SDXROOT%"=="" echo [FAIL] not inside a razzle prompt & exit /b 2
if "%VARIANT%"=="" set "VARIANT=srv"
if "%ISO_OUT%"=="" set "ISO_OUT=%SDXROOT%\..\out\loos.iso"
echo [_iso] variant=%VARIANT% out=%ISO_OUT%
call "%SDXROOT%\tools\oscdimg.cmd" %VARIANT% "%ISO_OUT%"
set RC=%errorlevel%
echo [_iso] exit=%RC%
exit /b %RC%
