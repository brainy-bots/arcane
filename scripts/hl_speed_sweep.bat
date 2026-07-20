@echo off
REM Speed sweep: measure the MAX SUPPORTED CLOSING SPEED of the attention
REM pipeline. Runs the `speed` phase at increasing traveler speeds (fresh
REM stack per run, nolegacy = interest-only replication); each run prints
REM   SPEED: v=<u/s> first_sighting t=<s> margin=<u>
REM and PASS/FAIL (margin > 60u contact threshold). The highest passing v
REM is the supported closing speed. Results: temp\speed_<v>.txt + summary.
setlocal enabledelayedexpansion
set ROOT=%~dp0..
set OBS=%ROOT%\target\release\examples\migration_observer.exe
set CLUSTERS=ws://127.0.0.1:8080,ws://127.0.0.1:8082,ws://127.0.0.1:8084,ws://127.0.0.1:8086
set MGR=http://127.0.0.1:7777
set TEMP_DIR=%ROOT%\..\temp
set SPEEDS=%*
if "%SPEEDS%"=="" set SPEEDS=50 100 200 400 800

echo [sweep] speeds: %SPEEDS%
del %TEMP_DIR%\speed_summary.txt 2>nul

for %%V in (%SPEEDS%) do call :one %%V

echo [sweep] ===== SUMMARY =====
type %TEMP_DIR%\speed_summary.txt
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
exit /b 0

:one
set V=%1
REM Duration: 20s settle + travel time (2000u at V u/s) + 10s tail, capped sanely.
set /a TRAVEL=2000/%V%
set /a DUR=20+TRAVEL+15
if %DUR% LSS 45 set DUR=45
echo [sweep] === v=%V% u/s (duration %DUR%s) ===
taskkill /f /im arcane-node.exe >nul 2>&1
taskkill /f /im arcane-manager.exe >nul 2>&1
taskkill /f /im arcane-router.exe >nul 2>&1
ping -n 3 127.0.0.1 >nul
start /min cmd /c "%ROOT%\scripts\hl_stack.bat nopin nolegacy"
ping -n 12 127.0.0.1 >nul
%OBS% --manager %MGR% --clusters %CLUSTERS% --players 3 --duration %DUR% --phase speed --travel-speed %V% > %TEMP_DIR%\speed_%V%.txt 2>&1
for /f "tokens=*" %%L in ('findstr /c:"SPEED:" %TEMP_DIR%\speed_%V%.txt') do echo %%L >> %TEMP_DIR%\speed_summary.txt
for /f "tokens=*" %%L in ('findstr /c:"VERDICT" %TEMP_DIR%\speed_%V%.txt') do echo   v=%V%: %%L >> %TEMP_DIR%\speed_summary.txt
findstr /c:"SPEED:" %TEMP_DIR%\speed_%V%.txt
findstr /c:"VERDICT" %TEMP_DIR%\speed_%V%.txt
exit /b 0
