@echo off
:: Castellyn -- сборка релизного exe + ярлык на рабочем столе.
:: Аргументы пробрасываются в build_all.ps1:
::   build_all.bat              -- standalone exe + ярлык
::   build_all.bat -Bundle      -- + установщики NSIS/MSI
::   build_all.bat -SkipCheck   -- без svelte-check
powershell.exe -ExecutionPolicy Bypass -File "%~dp0build_all.ps1" %*
pause
