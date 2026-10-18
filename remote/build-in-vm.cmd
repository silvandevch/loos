@echo off
rem build-in-vm.cmd - runs INSIDE an initialised razzle environment on the VM.
rem   (no args)  -> prebuild + build + make ISO
rem   iso         -> skip the build, only make the ISO from an existing tree
rem Inputs (env): VARIANT (SKU dir, default srv), ISO_OUT (iso path)
setlocal
if "%SDXROOT%"=="" echo [FAIL] not inside a razzle prompt & exit /b 2
if "%VARIANT%"=="" set "VARIANT=srv"
if "%ISO_OUT%"=="" set "ISO_OUT=%SDXROOT%\..\out\loos.iso"

if /I "%~1"=="iso" goto doiso

echo ============================================================
echo  build-in-vm: prebuild ^(SDXROOT=%SDXROOT%, arch=%_BuildArch%^)
echo ============================================================
call "%SDXROOT%\tools\prebuild.cmd"
if errorlevel 1 echo [WARN] prebuild rc=%errorlevel% - continuing

echo ============================================================
echo  build-in-vm: build -bcZP -M 4
echo ============================================================
build -bcZP -M 4
if errorlevel 1 echo [FAIL] build failed rc=%errorlevel% & exit /b 1

:doiso
echo ============================================================
echo  build-in-vm: oscdimg  variant=%VARIANT%  out=%ISO_OUT%
echo ============================================================
if not exist "%ISO_OUT%\.." mkdir "%ISO_OUT%\.."
call "%SDXROOT%\tools\oscdimg.cmd" %VARIANT% "%ISO_OUT%"
if errorlevel 1 echo [FAIL] oscdimg failed rc=%errorlevel% & exit /b 1
echo [OK] ISO written: %ISO_OUT%
exit /b 0
