@echo off
rem _build-pass.cmd - executed *inside* an initialised razzle environment.
rem Generates the build system (prebuild) then compiles the tree for the arch
rem razzle has selected (_BuildArch: native AMD64 on an AMD64 host, or amd64
rem when invoked with "win64 amd64" from an x86 host).
rem
rem Threads are capped at 4: the leaked tree's build is unstable above that.
if "%SDXROOT%"=="" echo [FAIL] not inside a razzle prompt & exit /b 2

echo ============================================================
echo  prebuild  ^(generates makefiles ^& build tools^)
echo  SDXROOT=%SDXROOT%   BuildArch=%_BuildArch%
echo ============================================================
call "%SDXROOT%\tools\prebuild.cmd"
if errorlevel 1 echo [WARN] prebuild returned %errorlevel% - continuing to build

echo ============================================================
echo  build -bcZP -M 4
echo ============================================================
build -bcZP -M 4
set RC=%errorlevel%
echo [build] build exit=%RC%
exit /b %RC%
