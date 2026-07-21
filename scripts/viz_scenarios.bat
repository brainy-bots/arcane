@echo off
REM Run all decision-layer scenarios sequentially against a fresh stack each.
setlocal
set SC=%~dp0..\..\arcane-viz\target\release\arcane-viz-scenarios.exe
set TEMP_DIR=E:\code\pgp-demo\temp

for %%S in (converge crossing bridge split chase) do call :one %%S
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
echo [scenarios] ALL DONE
exit /b 0

:one
echo [scenarios] === %1 ===
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
ping -n 3 127.0.0.1 >nul
start /min cmd /c "E:\code\pgp-demo\arcane\scripts\hl_stack.bat nopin"
ping -n 12 127.0.0.1 >nul
%SC% --scenario %1 > %TEMP_DIR%\sc_%1.txt 2>&1
findstr /c:"total player flips" /c:"WARN" /c:"SPLIT" %TEMP_DIR%\sc_%1.txt
exit /b 0
