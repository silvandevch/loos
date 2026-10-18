@echo off
setlocal
call "C:\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
cd /d "%~dp0"
if not exist C:\LoOS\out mkdir C:\LoOS\out
echo Building Notepad resources ...
rc.exe /fo C:\LoOS\out\LoNotepad.res notepad.rc
if errorlevel 1 exit /b %errorlevel%
echo Building LoNotepad ...
cl /nologo /EHsc /O2 /W4 /DUNICODE /D_UNICODE /Fe:C:\LoOS\out\LoNotepad.exe notepad.c C:\LoOS\out\LoNotepad.res comdlg32.lib shell32.lib comctl32.lib winspool.lib gdi32.lib ole32.lib UxTheme.lib user32.lib kernel32.lib advapi32.lib /link /SUBSYSTEM:WINDOWS
exit /b %errorlevel%
