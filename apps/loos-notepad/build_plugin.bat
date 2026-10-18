@echo off
rem ==========================================================================
rem  build_plugin.bat - compile a LoOS Notepad plugin from C to a .DLL
rem  Usage:  build_plugin.bat demo
rem  Sources live in plugins\<name>\*.c next to np_plugin.h, the DLL is
rem  installed straight into %%APPDATA%%\LoOS\Notepad\plugins\ (created).
rem ==========================================================================
setlocal
if "%~1"=="" (
  echo Usage: build_plugin.bat ^<name^>
  echo Example: build_plugin.bat demo
  exit /b 2
)

where gcc >nul 2>&1
if errorlevel 1 (
  echo.
  echo ERROR: gcc not found on PATH - cannot compile "%~1".
  echo.
  echo Install it with ONE of these, then re-run this script:
  echo   1^)  winget install -e --id GNU.Mingw
  echo   2^)  MSYS2 from https://www.msys2.org/ , then:  pacman -S mingw-w64-x86_64-gcc
  echo   3^)  winlibs standalone build from https://winlibs.com/ ,
  echo       unzip and add its bin\ folder to PATH.
  echo.
  exit /b 1
)

if not exist "%~dp0plugins\%~1" (
  echo ERROR: no such plugin folder: "%~dp0plugins\%~1"
  exit /b 2
)

set DST=%APPDATA%\LoOS\Notepad\plugins
if not exist "%DST%" mkdir "%DST%"
echo Compiling plugin "%~1" ...
gcc -shared -O2 -municode -I"%~dp0plugins" -o "%DST%\%~1.dll" "%~dp0plugins\%~1\*.c"
if errorlevel 1 (
  echo ERROR: gcc failed - see messages above.
  exit /b 1
)
echo Installed: %DST%\%~1.dll
echo Restart Notepad - the commands appear under Tools.
