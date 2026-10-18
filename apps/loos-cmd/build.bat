@echo off
setlocal
call "C:\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
cd /d "%~dp0"
if not exist C:\LoOS\out mkdir C:\LoOS\out
echo Building LoCmd ...
cl /nologo /O2 /W0 /DUNICODE /D_UNICODE /Fe:C:\LoOS\out\LoCmd.exe cmd.c iphlpapi.lib netapi32.lib advapi32.lib shell32.lib user32.lib kernel32.lib
exit /b %errorlevel%
