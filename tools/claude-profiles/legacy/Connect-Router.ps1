# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (connect_router_native); kept for reference only.
<#
.SYNOPSIS
    Turnkey: route a profile's Claude Code through claude-code-router (ccr) to an OpenAI engine.

.DESCRIPTION
    One coherent flow (fixes the disjointed setup that left a profile pointed at an OpenAI
    endpoint directly — which Claude Code can't parse). Steps, all via the existing scripts (DRY):
      1. Setup-Router.ps1 -Action configure  → write ccr config for <Backend>/<Model>, restart ccr
      2. ensure ccr is up (`ccr start`)
      3. Manage-Provider.ps1 -Action set     → bind <Profile> to http://127.0.0.1:3456 (Anthropic),
                                               token = ccr APIKEY (empty when ccr has none)
    After this, the profile talks Anthropic→ccr→OpenAI engine. No Read-Host.

.EXAMPLE
    .\Connect-Router.ps1 -Backend http://localhost:1234/v1 -Model qwen2.5-coder -Profile cc4 -Name lmstudio
#>
param(
    [Parameter(Mandatory)][string]$Backend,
    [Parameter(Mandatory)][string]$Model,
    # Renamed away from the automatic variable of that name (it holds the profile script path, and
    # assigning to it has side effects). The -Profile alias keeps the documented command line working.
    [Parameter(Mandatory)][Alias('Profile')][string]$ProfileName,
    [string]$Name = 'lmstudio',
    [switch]$WhatIf
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

$ccrBase = 'http://127.0.0.1:3456'

Write-Host "=== Подключение через роутер: $Name → профиль $ProfileName ===" -ForegroundColor Cyan

# 1. Configure ccr for this backend/model (+ ccr restart inside Setup-Router).
$setup = Join-Path $scriptDir 'Setup-Router.ps1'
$a = @{ Action = 'configure'; Backend = $Backend; Model = $Model; Name = $Name }
if ($WhatIf) { $a.WhatIf = $true }
& $setup @a
if ($LASTEXITCODE -ne 0) { Write-Host 'Прервано: не удалось настроить ccr.' -ForegroundColor Red; exit 1 }

# 2. Ensure ccr is running (and verify the port actually came up).
if (-not $WhatIf) {
    if (Get-Command ccr -ErrorAction SilentlyContinue) {
        Write-Host '  ccr start …' -ForegroundColor Gray
        cmd /c 'ccr start' 2>&1 | ForEach-Object { Write-Host "    $_" }
        Start-Sleep 4
        $up = $null -ne (Get-NetTCPConnection -LocalPort 3456 -State Listen -ErrorAction SilentlyContinue)
        if ($up) {
            Write-Host '  ccr слушает :3456 ✓' -ForegroundColor Green
        } else {
            Write-Host '  [ВНИМАНИЕ] ccr не поднял порт :3456. Конфиг и привязка сделаны, но сервер не запущен.' -ForegroundColor Yellow
            Write-Host '            Попробуй: обновить ccr (вкладка «Обновления»), либо запусти «ccr code» в терминале (он сам поднимает сервер).' -ForegroundColor Yellow
        }
    }
}

# 3. Read ccr APIKEY (token the profile must send; empty when ccr is open on localhost).
$token = ''
$cfgPath = Join-Path $HOME '.claude-code-router\config.json'
if (Test-Path -LiteralPath $cfgPath) {
    # An unreadable ccr config just means "no APIKEY": step 4 then omits -Token and Manage-Provider
    # writes its dummy bearer, which is the correct behaviour for an open localhost ccr.
    try { $token = [string]((Get-Content -LiteralPath $cfgPath -Raw -Encoding UTF8 | ConvertFrom-Json).APIKEY) }
    catch { Write-Host "  [ВНИМАНИЕ] не прочитал APIKEY из $cfgPath — привязываю без токена: $($_.Exception.Message)" -ForegroundColor Yellow }
}

# 4. Bind the profile to ccr (Anthropic endpoint).
#    Token: pass ccr's APIKEY when it has one; otherwise omit it so Manage-Provider writes a dummy
#    bearer (single source of the dummy-token rule) — a bare `claude` then skips the login screen.
$prov = Join-Path $scriptDir 'Manage-Provider.ps1'
$pa = @{ Action = 'set'; Name = $ProfileName; BaseUrl = $ccrBase; Model = $Model }
if ($token) { $pa.Token = $token }
if ($WhatIf) { $pa.WhatIf = $true }
& $prov @pa
if ($LASTEXITCODE -ne 0) { Write-Host 'Прервано: не удалось привязать профиль.' -ForegroundColor Red; exit 1 }

Write-Host "Готово. Профиль '$ProfileName' → $ccrBase (ccr) → $Name ($Model). Перезапусти профиль." -ForegroundColor Green
