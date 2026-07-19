# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (setup_router_native / apply_router_config); kept for reference only.
<#
.SYNOPSIS
    Install / configure claude-code-router (ccr) — the Anthropic↔OpenAI bridge that lets
    Claude Code talk to OpenAI-compatible engines (LM Studio, FreeLLMAPI, …).

.DESCRIPTION
      install   : npm install -g @musistudio/claude-code-router (skips if already present)
      configure : write ~/.claude-code-router/config.json so ccr forwards Claude Code to the
                  given OpenAI backend (-Backend) using -Model, then `ccr restart`.
                  Existing config is backed up and other providers are preserved.
      status    : report whether ccr is installed and configured.

    After configure+start, bind a profile to http://127.0.0.1:3456 (provider preset «Claude
    Code Router») and the profile's Claude Code will use the backend model. No Read-Host.

.EXAMPLE
    .\Setup-Router.ps1 -Action install
    .\Setup-Router.ps1 -Action configure -Backend http://localhost:1234/v1 -Model qwen2.5-coder -Name lmstudio
#>
param(
    [Parameter(Mandatory)][ValidateSet('install', 'configure', 'status')][string]$Action,
    [string]$Backend,
    [string]$Model,
    [string]$Name = 'lmstudio',
    [switch]$WhatIf
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$cfgDir = Join-Path $HOME '.claude-code-router'
$cfgPath = Join-Path $cfgDir 'config.json'
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Test-Ccr { [bool](Get-Command ccr -ErrorAction SilentlyContinue) }

Write-Host "=== Router (ccr): $Action ===" -ForegroundColor Cyan

if ($Action -eq 'status') {
    Write-Host ("  Установлен: {0}" -f $(if (Test-Ccr) { 'да' } else { 'нет' })) -ForegroundColor Gray
    Write-Host ("  Конфиг: {0}" -f $(if (Test-Path -LiteralPath $cfgPath) { $cfgPath } else { 'нет' })) -ForegroundColor Gray
    exit 0
}

if ($Action -eq 'install') {
    if (Test-Ccr) { Write-Host '  ccr уже установлен.' -ForegroundColor Green; exit 0 }
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        Write-Host '  ОШИБКА: npm не найден на PATH (нужен Node.js).' -ForegroundColor Red; exit 1
    }
    if ($WhatIf) { Write-Host '  [DRY RUN] npm install -g @musistudio/claude-code-router' -ForegroundColor Magenta; exit 0 }
    Write-Host '  npm install -g @musistudio/claude-code-router …' -ForegroundColor Gray
    cmd /c 'npm install -g @musistudio/claude-code-router' 2>&1 | ForEach-Object { Write-Host "    $_" }
    if (Test-Ccr) { Write-Host '  ccr установлен.' -ForegroundColor Green; exit 0 }
    else { Write-Host '  Не удалось подтвердить установку ccr.' -ForegroundColor Yellow; exit 1 }
}

# --- configure ---
if (-not $Backend) { Write-Host 'ОШИБКА: для configure нужен -Backend (URL движка).' -ForegroundColor Red; exit 1 }
if (-not $Model) { Write-Host 'ОШИБКА: для configure нужен -Model (например, из «Загрузить модели»).' -ForegroundColor Red; exit 1 }

# ccr wants the full chat-completions URL.
$apiBase = $Backend.TrimEnd('/')
if ($apiBase -notmatch '/chat/completions$') { $apiBase = "$apiBase/chat/completions" }

# Load existing config (preserve other providers/keys) or start fresh.
$cfg = $null
if (Test-Path -LiteralPath $cfgPath) {
    # A malformed/unreadable ccr config must not abort the configure step: the line below falls back
    # to a fresh @{}, which is the intended "start over" path. Warn instead of dying.
    try { $cfg = Get-Content -LiteralPath $cfgPath -Raw -Encoding UTF8 | ConvertFrom-Json -AsHashtable }
    catch { Write-Host "  [ВНИМАНИЕ] не разобрал $cfgPath — начинаю с пустого конфига: $($_.Exception.Message)" -ForegroundColor Yellow }
}
if ($cfg -isnot [hashtable]) { $cfg = @{} }
if (-not $cfg.ContainsKey('Providers') -or $cfg['Providers'] -isnot [System.Collections.IList]) { $cfg['Providers'] = @() }

# Upsert our provider by name.
$providers = [System.Collections.ArrayList]@()
$found = $false
foreach ($p in $cfg['Providers']) {
    if ($p.name -eq $Name) {
        [void]$providers.Add(@{ name = $Name; api_base_url = $apiBase; api_key = 'not-needed'; models = @($Model) })
        $found = $true
    } else { [void]$providers.Add($p) }
}
if (-not $found) {
    [void]$providers.Add(@{ name = $Name; api_base_url = $apiBase; api_key = 'not-needed'; models = @($Model) })
}
$cfg['Providers'] = $providers
if (-not $cfg.ContainsKey('Router') -or $cfg['Router'] -isnot [hashtable]) { $cfg['Router'] = @{} }
$cfg['Router']['default'] = "$Name,$Model"

Write-Host "  Провайдер '$Name' -> $apiBase  (модель $Model); Router.default = $Name,$Model" -ForegroundColor Gray

if ($WhatIf) {
    Write-Host '  [DRY RUN] config.json не записан. Итог:' -ForegroundColor Magenta
    Write-Host ($cfg | ConvertTo-Json -Depth 8) -ForegroundColor DarkGray
    exit 0
}

if (-not (Test-Path -LiteralPath $cfgDir)) { New-Item -ItemType Directory -Path $cfgDir -Force | Out-Null }
if (Test-Path -LiteralPath $cfgPath) { Copy-Item -LiteralPath $cfgPath -Destination "$cfgPath.bak" -Force }
[System.IO.File]::WriteAllText($cfgPath, ($cfg | ConvertTo-Json -Depth 8), $Utf8NoBom)
Write-Host '  config.json записан (бэкап .bak).' -ForegroundColor Green

if (Test-Ccr) {
    Write-Host '  ccr restart …' -ForegroundColor Gray
    cmd /c 'ccr restart' 2>&1 | ForEach-Object { Write-Host "    $_" }
}
Write-Host '  Готово. Навесь на профиль пресет «Claude Code Router» (http://127.0.0.1:3456).' -ForegroundColor Green
exit 0
