@echo off
setlocal
call "C:\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
cd /d "%~dp0"
if not exist C:\LoOS\out mkdir C:\LoOS\out
echo Building Task Manager resources ...
rc.exe /fo C:\LoOS\out\LoTaskMgr.res taskmgr.rc
if errorlevel 1 exit /b %errorlevel%
echo Building LoTaskMgr ...
cl /nologo /EHsc /O2 /W4 /DUNICODE /D_UNICODE /Fe:C:\LoOS\out\LoTaskMgr.exe taskmgr.c C:\LoOS\out\LoTaskMgr.res comctl32.lib comdlg32.lib user32.lib kernel32.lib gdi32.lib shell32.lib advapi32.lib psapi.lib wtsapi32.lib version.lib shlwapi.lib iphlpapi.lib pdh.lib setupapi.lib /link /SUBSYSTEM:WINDOWS
exit /b %errorlevel%
