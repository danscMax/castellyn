# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (manage_provider_native / apply_provider_env); kept for reference only.
<#
.SYNOPSIS
    Bind / unbind an LLM provider to ONE Claude Code profile via its settings.json env.

.DESCRIPTION
    Claude Code reads the active provider from the per-profile
    ~/.claude-<name>/settings.json "env" block. This script merges those env keys
    (preserving every other setting), so each profile can talk to a different
    provider / local engine:
        ANTHROPIC_BASE_URL             custom endpoint (e.g. http://localhost:4000)
        ANTHROPIC_AUTH_TOKEN           bearer token; a dummy is written for a keyless
                                       endpoint so a bare `claude` skips the login screen
        ANTHROPIC_DEFAULT_SONNET_MODEL main model  (driven by -Model)
        ANTHROPIC_DEFAULT_OPUS_MODEL   main model  (driven by -Model)
        ANTHROPIC_DEFAULT_HAIKU_MODEL  small/fast model (driven by -SmallModel)

      set   : write ANTHROPIC_BASE_URL (+ the optional keys when provided; keys with
              an empty value are REMOVED so 'set' is the full desired provider state).
              A custom endpoint with no token gets a dummy bearer (never left tokenless).
              Legacy ANTHROPIC_MODEL / ANTHROPIC_SMALL_FAST_MODEL are migrated away.
      clear : remove all provider keys → back to the default Anthropic login.

    settings.json is backed up (.bak) before writing; UTF-8 no BOM; -WhatIf previews.
    The token is stored in plaintext in settings.json (Claude Code's native mechanism);
    settings.json is machine-local and not synced. No Read-Host (dashboard-safe).

.EXAMPLE
    .\Manage-Provider.ps1 -Action set -Name cc3 -BaseUrl http://localhost:4000 -Token sk-x -Model glm-4.7
    .\Manage-Provider.ps1 -Action clear -Name cc3 -WhatIf
#>
param(
    [Parameter(Mandatory)][ValidateSet('set', 'clear')][string]$Action,
    [Parameter(Mandatory)][string]$Name,
    [string]$BaseUrl,
    [string]$Token,
    [switch]$TokenStdin,
    [string]$Model,
    [string]$SmallModel,
    [switch]$KeepToken,
    [switch]$WhatIf
)

# -TokenStdin: read the bearer from STDIN instead of -Token, so the secret never lands in the
# process command line (argv is world-readable on Windows via Win32_Process). Empty → dummy below.
if ($TokenStdin) { $Token = ([Console]::In.ReadToEnd()).Trim() }

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir 'ProfileLib.ps1')

# Validate the profile name against the canonical list.
$known = @((Get-ClaudeProfiles -ConfigDir (Join-Path $scriptDir 'config')).Name)
if ($known -notcontains $Name) {
    Write-Host "ОШИБКА: профиль '$Name' не найден ($($known -join ', '))." -ForegroundColor Red
    exit 1
}

# Tier model keys are the current Claude Code mechanism; ANTHROPIC_MODEL / ANTHROPIC_SMALL_FAST_MODEL
# are legacy and listed here only so `clear` (and the set-migration below) scrub them too.
$ENV_KEYS = @(
    'ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN',
    'ANTHROPIC_DEFAULT_SONNET_MODEL', 'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_DEFAULT_HAIKU_MODEL',
    'ANTHROPIC_MODEL', 'ANTHROPIC_SMALL_FAST_MODEL'
)
# Placeholder bearer for keyless local gateways: its only job is to get Claude Code past the
# "Not logged in" screen on a bare launch; a keyless gateway ignores the value.
$DummyToken = 'agenthub-local'
$settingsPath = Join-Path $HOME ".claude-$Name\settings.json"
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

# Load settings as a mutable hashtable (BOM-tolerant), or start empty.
$settings = @{}
if (Test-Path -LiteralPath $settingsPath) {
    try {
        # TrimStart handles the BOM; the old `-replace "^\xEF\xBB\xBF"` was dead (PS escapes with a
        # backtick, so `\xEF` was a literal that never matched).
        $raw = (Get-Content -LiteralPath $settingsPath -Raw -Encoding UTF8).TrimStart([char]0xFEFF)
        if ($raw.Trim()) { $settings = $raw | ConvertFrom-Json -AsHashtable }
    } catch {
        Write-Host "ОШИБКА: не удалось прочитать settings.json ($($_.Exception.Message))." -ForegroundColor Red
        exit 1
    }
}
if ($settings -isnot [hashtable]) { $settings = @{} }
if (-not $settings.ContainsKey('env') -or $settings['env'] -isnot [hashtable]) { $settings['env'] = @{} }
$env = $settings['env']

Write-Host "=== Provider: $Action $Name ===" -ForegroundColor Cyan
if ($WhatIf) { Write-Host '[DRY RUN] изменения не применяются' -ForegroundColor Magenta }

if ($Action -eq 'set') {
    if (-not $BaseUrl) {
        Write-Host 'ОШИБКА: для set нужен -BaseUrl (или используйте clear).' -ForegroundColor Red
        exit 1
    }
    # BASE_URL: always authoritative. MODEL/SMALL_FAST_MODEL: authoritative when the param
    # is passed (empty value removes the override) — they're readable, so the dialog is the
    # source of truth. TOKEN: keep-or-set — left untouched when -KeepToken (we can't read it
    # back to re-display), otherwise set/removed by the -Token value.
    $env['ANTHROPIC_BASE_URL'] = $BaseUrl
    # TOKEN: keep existing (-KeepToken), set the supplied one, or — for a custom endpoint with
    # no token — write a dummy bearer so a bare `claude` skips the login screen even with no real
    # Anthropic login. A 'set' always carries a custom BaseUrl, so it must never be left tokenless.
    if ($KeepToken) {
        # leave ANTHROPIC_AUTH_TOKEN as-is
    } elseif ($Token) {
        $env['ANTHROPIC_AUTH_TOKEN'] = $Token
    } else {
        $env['ANTHROPIC_AUTH_TOKEN'] = $DummyToken
    }
    # MODELS: map to Claude's tier env vars. -Model → Sonnet+Opus, -SmallModel → Haiku. The legacy
    # single-value keys are always scrubbed on set so the tier vars are the one source of truth.
    $env.Remove('ANTHROPIC_MODEL'); $env.Remove('ANTHROPIC_SMALL_FAST_MODEL')
    if ($PSBoundParameters.ContainsKey('Model')) {
        if ($Model) { $env['ANTHROPIC_DEFAULT_SONNET_MODEL'] = $Model; $env['ANTHROPIC_DEFAULT_OPUS_MODEL'] = $Model }
        else { $env.Remove('ANTHROPIC_DEFAULT_SONNET_MODEL'); $env.Remove('ANTHROPIC_DEFAULT_OPUS_MODEL') }
    }
    if ($PSBoundParameters.ContainsKey('SmallModel')) {
        if ($SmallModel) { $env['ANTHROPIC_DEFAULT_HAIKU_MODEL'] = $SmallModel } else { $env.Remove('ANTHROPIC_DEFAULT_HAIKU_MODEL') }
    }
    $shown = if ($KeepToken) { '(без изменений)' } elseif ($Token) { '(задан)' } else { "(dummy: $DummyToken)" }
    Write-Host "  BaseUrl=$BaseUrl  Model=$(if($Model){$Model}else{'—'})  SmallModel=$(if($SmallModel){$SmallModel}else{'—'})  Token=$shown" -ForegroundColor Gray
} else {
    foreach ($k in $ENV_KEYS) { $env.Remove($k) }
    Write-Host '  Провайдер сброшен на стандартный Anthropic-логин.' -ForegroundColor Gray
}

# Drop an empty env block for cleanliness.
if ($env.Count -eq 0) { $settings.Remove('env') }

if ($WhatIf) {
    Write-Host '[DRY RUN] settings.json не изменён. Итоговый env:' -ForegroundColor Magenta
    if ($settings.ContainsKey('env')) { $settings['env'].GetEnumerator() | ForEach-Object { Write-Host ("    {0} = {1}" -f $_.Key, $(if ($_.Key -eq 'ANTHROPIC_AUTH_TOKEN') { '***' } else { $_.Value })) -ForegroundColor DarkGray } }
    else { Write-Host '    (пусто — стандартный Anthropic)' -ForegroundColor DarkGray }
    exit 0
}

# Backup then write (UTF-8 no BOM).
$dir = Split-Path -Parent $settingsPath
if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
if (Test-Path -LiteralPath $settingsPath) { Copy-Item -LiteralPath $settingsPath -Destination "$settingsPath.bak" -Force }
[System.IO.File]::WriteAllText($settingsPath, ($settings | ConvertTo-Json -Depth 10), $Utf8NoBom)
Write-Host "  settings.json обновлён (бэкап .bak). Перезапустите профиль '$Name', чтобы провайдер применился." -ForegroundColor Green
exit 0
