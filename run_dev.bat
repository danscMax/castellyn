@echo off
:: Castellyn -- launch DEV build (Vite HMR + Tauri debug window).
:: Invoked by the "Castellyn (dev)" desktop shortcut. Keeps a console open so you
:: see HMR / build logs; close it (or Ctrl+C) to stop the dev server.
chcp 65001 >nul
:: rustup puts cargo here but it's not always on the system PATH -> add it back.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
echo === Castellyn dev :: npm run tauri dev ===
echo (close this window or press Ctrl+C to stop)
echo.
:: A stale vite dev server (node) may still hold port 1420 from a previous run and
:: make "tauri dev" abort with "Port 1420 is already in use" -> free it first.
echo Freeing port 1420 if a stale dev server is holding it...
powershell -NoProfile -Command "Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }"
call npm run tauri dev
echo.
echo [dev server stopped]
pause
