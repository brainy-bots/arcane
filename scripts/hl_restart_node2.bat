@echo off
REM #289 restart-convergence probe: kill node2 mid-run, restart it, watch it converge.
REM Run against an ALREADY RUNNING stack (hl_stack.bat nopin) with players active.
setlocal
set ROOT=%~dp0..
set BIN=%ROOT%\target\release
set C2=22222222-2222-2222-2222-222222222222

echo [restart] killing node2 (pid by port 8083 stats)...
for /f "tokens=5" %%p in ('netstat -ano ^| findstr :8082 ^| findstr LISTENING') do taskkill /f /pid %%p
timeout /t 5 /nobreak >nul

echo [restart] restarting node2 with a COLD map (the #289 test: no state, no history)...
start "node2r" /min cmd /c "set NODE_ID=%C2%&& set REDIS_URL=redis://127.0.0.1:6379&& set NEIGHBOR_IDS=11111111-1111-1111-1111-111111111111,33333333-3333-3333-3333-333333333333,44444444-4444-4444-4444-444444444444&& set NODE_WS_PORT=8082&& set NODE_STATS_PORT=8083&& set NODE_STATE_PUBLISH_TICKS=10&& set ARCANE_INPUT_FORWARDING=on&& set NODE_CLUSTER_ADDRS=11111111-1111-1111-1111-111111111111:127.0.0.1:8080,%C2%:127.0.0.1:8082,33333333-3333-3333-3333-333333333333:127.0.0.1:8084,44444444-4444-4444-4444-444444444444:127.0.0.1:8086&& set ARCANE_RESYNC_EVERY_N_TICKS=20&& %BIN%\arcane-node.exe 2> %ROOT%\..\temp\hl_node2_restarted.log"
echo [restart] node2 restarted.
endlocal
