# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (run_engine + load_engine_cfg); kept for reference only.
<#
.SYNOPSIS
    Start / stop a local LLM engine (proxy/router) listed in config\engines.json.

.DESCRIPTION
      start : launch the engine's command detached in its own window
              (.bat → cmd, .py → python). Empty command = status-only entry.
      stop  : find the PID listening on the engine's port and kill it.

    Status is read natively by the dashboard (TCP port check), not here.
    No Read-Host (dashboard-safe). -WhatIf previews.

.EXAMPLE
    .\Manage-Engine.ps1 -Action start -Id router-glm
    .\Manage-Engine.ps1 -Action stop  -Id freedeepseek -WhatIf
#>
param(
    [Parameter(Mandatory)][ValidateSet('start', 'stop')][string]$Action,
    [Parameter(Mandatory)][string]$Id,
    [switch]$WhatIf
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# Expand manifest path placeholders so engines.json can stay machine-independent.
# {{SCRIPTS_ROOT}} -> $env:SCRIPTS_ROOT (default E:\Scripts); {{USERPROFILE}} -> the home dir.
# A path without placeholders passes through unchanged (backward compatible).
function Expand-ManifestPath([string]$p) {
    if (-not $p) { return $p }
    $root = if ($env:SCRIPTS_ROOT) { $env:SCRIPTS_ROOT } else { 'E:\Scripts' }
    return $p.Replace('{{SCRIPTS_ROOT}}', $root).Replace('{{USERPROFILE}}', $env:USERPROFILE)
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfgPath = Join-Path $scriptDir 'config\engines.json'
if (-not (Test-Path -LiteralPath $cfgPath)) {
    Write-Host "ОШИБКА: engines.json не найден ($cfgPath)." -ForegroundColor Red
    exit 1
}

try {
    $cfg = Get-Content -LiteralPath $cfgPath -Raw -Encoding UTF8 | ConvertFrom-Json
} catch {
    Write-Host "ОШИБКА: engines.json не парсится ($($_.Exception.Message))." -ForegroundColor Red
    exit 1
}

$engine = $cfg.engines | Where-Object { $_.id -eq $Id } | Select-Object -First 1
if (-not $engine) {
    Write-Host "ОШИБКА: движок '$Id' не найден в engines.json." -ForegroundColor Red
    exit 1
}

Write-Host "=== Engine: $Action $($engine.name) (порт $($engine.port)) ===" -ForegroundColor Cyan
if ($WhatIf) { Write-Host '[DRY RUN] изменения не применяются' -ForegroundColor Magenta }

if ($Action -eq 'start') {
    # Engines with a shell start command (e.g. ccr) run it inline; file-based engines launch the file.
    $startShell = Expand-ManifestPath ([string]$engine.start)
    if ($startShell) {
        if ($WhatIf) { Write-Host "  [DRY RUN] выполнил бы: $startShell" -ForegroundColor Magenta; exit 0 }
        Write-Host "  > $startShell" -ForegroundColor Gray
        cmd /c $startShell 2>&1 | ForEach-Object { Write-Host "    $_" }
        Write-Host "  Запрошен запуск. Эндпоинт: $($engine.baseUrl)" -ForegroundColor Green
        exit 0
    }
    $cmd = Expand-ManifestPath ([string]$engine.command)
    if (-not $cmd) {
        Write-Host '  У движка нет команды запуска (status-only) — запустите его вручную.' -ForegroundColor Yellow
        exit 0
    }
    if (-not (Test-Path -LiteralPath $cmd)) {
        Write-Host "  ОШИБКА: файл запуска не найден: $cmd" -ForegroundColor Red
        exit 1
    }
    $ext = [IO.Path]::GetExtension($cmd).ToLower()
    if ($WhatIf) {
        Write-Host "  [DRY RUN] запустил бы: $cmd" -ForegroundColor Magenta
        exit 0
    }
    try {
        if ($ext -eq '.py') {
            Start-Process -FilePath 'python' -ArgumentList "`"$cmd`"" -WorkingDirectory (Split-Path -Parent $cmd)
        } else {
            # .bat / .cmd / .exe — launch directly in its own window.
            Start-Process -FilePath $cmd -WorkingDirectory (Split-Path -Parent $cmd)
        }
        Write-Host "  Запущено. Дашборд/порт: $($engine.baseUrl)" -ForegroundColor Green
    } catch {
        Write-Host "  ОШИБКА запуска: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
} elseif ([string]$engine.stop) {
    # Engines with a shell stop command (e.g. ccr) — run it inline.
    $stopShell = Expand-ManifestPath ([string]$engine.stop)
    if ($WhatIf) { Write-Host "  [DRY RUN] выполнил бы: $stopShell" -ForegroundColor Magenta; exit 0 }
    Write-Host "  > $stopShell" -ForegroundColor Gray
    cmd /c $stopShell 2>&1 | ForEach-Object { Write-Host "    $_" }
    Write-Host '  Запрошена остановка.' -ForegroundColor Green
} else {
    # stop: kill whatever listens on the engine port.
    $port = [int]$engine.port
    $pids = @()
    try {
        $pids = @(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty OwningProcess -Unique)
    } catch {
        # Get-NetTCPConnection is absent on non-Windows and can fail without the TCP/IP WMI provider.
        # Treat it as "nothing listening" — the branch below reports the engine as already stopped.
        Write-Verbose "не удалось опросить порт ${port}: $_"
    }
    if (-not $pids -or $pids.Count -eq 0) {
        Write-Host "  На порту $port никто не слушает — движок уже остановлен." -ForegroundColor Yellow
        exit 0
    }
    foreach ($procId in $pids) {
        $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
        $pname = if ($p) { $p.ProcessName } else { '?' }
        if ($WhatIf) {
            Write-Host "  [DRY RUN] остановил бы PID $procId ($pname) на порту $port" -ForegroundColor Magenta
            continue
        }
        try {
            Stop-Process -Id $procId -Force -ErrorAction Stop
            Write-Host "  Остановлен PID $procId ($pname)." -ForegroundColor Green
        } catch {
            Write-Host "  ОШИБКА остановки PID $procId : $($_.Exception.Message)" -ForegroundColor Red
        }
    }
}
