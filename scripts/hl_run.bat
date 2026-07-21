@echo off
REM Run the migration_observer harness against the hl_stack.
REM Usage: hl_run.bat <static|migrate> [players] [duration_secs] [extra args...]
setlocal
set ROOT=%~dp0..
set PHASE=%1
if "%PHASE%"=="" set PHASE=static
set PLAYERS=%2
if "%PLAYERS%"=="" set PLAYERS=4
set DUR=%3
if "%DUR%"=="" set DUR=60
shift & shift & shift
set EXTRA=
:collect
if "%1"=="" goto run
set EXTRA=%EXTRA% %1
shift
goto collect
:run
%ROOT%\target\release\examples\migration_observer.exe --manager http://127.0.0.1:7777 --clusters ws://127.0.0.1:8080,ws://127.0.0.1:8082,ws://127.0.0.1:8084,ws://127.0.0.1:8086 --players %PLAYERS% --duration %DUR% --phase %PHASE%%EXTRA%
endlocal
