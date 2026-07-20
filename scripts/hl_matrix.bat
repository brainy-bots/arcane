@echo off
REM Full decision-layer matrix under the manager/router execution split.
REM Restacks (with FLUSHALL) before each phase; results to temp\mx_<phase>.txt
setlocal
set ROOT=%~dp0..
set OBS=%ROOT%\target\release\examples\migration_observer.exe
set CLUSTERS=ws://127.0.0.1:8080,ws://127.0.0.1:8082,ws://127.0.0.1:8084,ws://127.0.0.1:8086
set MGR=http://127.0.0.1:7777
set TEMP_DIR=%ROOT%\..\temp

call :phase cluster 6 90 nopin
call :phase defector 6 150 nopin
call :phase gradient 6 90 nopin
call :phase spectrum-idle 8 90 "nopin nolegacy"
call :phase spectrum-warmup 6 150 "nopin nolegacy"
call :restartphase

taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
echo [matrix] DONE
exit /b 0

:phase
set PH=%1
set NP=%2
set DUR=%3
set STACKARGS=%~4
echo [matrix] === %PH% (%NP% players, %DUR%s, stack: %STACKARGS%) ===
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
ping -n 3 127.0.0.1 >nul
start /min cmd /c "%ROOT%\scripts\hl_stack.bat %STACKARGS%"
ping -n 12 127.0.0.1 >nul
%OBS% --manager %MGR% --clusters %CLUSTERS% --players %NP% --duration %DUR% --phase %PH% > %TEMP_DIR%\mx_%PH%.txt 2>&1
findstr VERDICT %TEMP_DIR%\mx_%PH%.txt
exit /b 0

:restartphase
echo [matrix] === restart (4 players, 120s, stack: nopin) ===
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
ping -n 3 127.0.0.1 >nul
start /min cmd /c "%ROOT%\scripts\hl_stack.bat nopin"
ping -n 12 127.0.0.1 >nul
start /min cmd /c "ping -n 35 127.0.0.1 >nul && %ROOT%\scripts\hl_restart_node2.bat"
%OBS% --manager %MGR% --clusters %CLUSTERS% --players 4 --duration 120 --phase restart > %TEMP_DIR%\mx_restart.txt 2>&1
findstr VERDICT %TEMP_DIR%\mx_restart.txt
exit /b 0
