@echo off
REM Headless control-plane stack: 4 Rust arcane-node + real arcane-manager + docker Redis.
REM Usage: hl_stack.bat [nopin] [nofwd]
REM   nopin - disable the pin feature (players may migrate while connected)
REM   nofwd - disable D1 input forwarding (split-brain demo mode)
REM Companion runner: hl_run.bat <phase> <players> <duration>
REM
REM Ports: nodes ws 8080/8082/8084/8086 (+1 each for stats), manager http 7777.
REM Cluster ids are fixed so observers/manager agree across restarts.

setlocal
set ROOT=%~dp0..
set BIN=%ROOT%\target\release
set REDIS_URL=redis://127.0.0.1:6379

set C1=11111111-1111-1111-1111-111111111111
set C2=22222222-2222-2222-2222-222222222222
set C3=33333333-3333-3333-3333-333333333333
set C4=44444444-4444-4444-4444-444444444444

set PIN_NODE=client_anchor
set PIN_MGR=client_anchor
set FWD=on
:parse
if "%1"=="nopin" ( set PIN_NODE=& set PIN_MGR=& shift & goto parse )
if "%1"=="nofwd" ( set FWD=off& shift & goto parse )

echo [stack] pin=%PIN_NODE% forwarding=%FWD%

REM Flush stale control-plane state from previous stacks (state keys poison staleness/partition).
docker exec arcane-redis redis-cli FLUSHALL >nul 2>&1

set COMMON=REDIS_URL=%REDIS_URL%
for %%N in (1 2 3 4) do call :node %%N
goto manager

:node
set /a WSPORT=8078+2*%1
set /a STATSPORT=WSPORT+1
if "%1"=="1" set NID=%C1%& set NEIGH=%C2%,%C3%,%C4%
if "%1"=="2" set NID=%C2%& set NEIGH=%C1%,%C3%,%C4%
if "%1"=="3" set NID=%C3%& set NEIGH=%C1%,%C2%,%C4%
if "%1"=="4" set NID=%C4%& set NEIGH=%C1%,%C2%,%C3%
start "node%1" /min cmd /c "set NODE_ID=%NID%&& set REDIS_URL=%REDIS_URL%&& set NEIGHBOR_IDS=%NEIGH%&& set NODE_WS_PORT=%WSPORT%&& set NODE_STATS_PORT=%STATSPORT%&& set NODE_STATE_PUBLISH_TICKS=10&& set NODE_PIN_FEATURE=%PIN_NODE%&& set ARCANE_INPUT_FORWARDING=%FWD%&& set ARCANE_RESYNC_EVERY_N_TICKS=20&& %BIN%\arcane-node.exe 2> %ROOT%\..\temp\hl_node%1.log"
exit /b

:manager
timeout /t 3 /nobreak >nul
start "manager" /min cmd /c "set MANAGER_CLUSTERS=%C1%:127.0.0.1:8080,%C2%:127.0.0.1:8082,%C3%:127.0.0.1:8084,%C4%:127.0.0.1:8086&& set MANAGER_HTTP_PORT=7777&& set REDIS_URL=%REDIS_URL%&& set MANAGER_CADENCE_MS=500&& set MANAGER_JOIN_POLICY=round-robin&& set MANAGER_PIN_FEATURE=%PIN_MGR%&& %BIN%\arcane-manager.exe 2> %ROOT%\..\temp\hl_manager.log"
echo [stack] 4 nodes + manager starting. Logs: temp\hl_node*.log, temp\hl_manager.log
echo [stack] stop with: taskkill /f /im arcane-node.exe ^& taskkill /f /im arcane-manager.exe
endlocal
