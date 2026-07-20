# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (opencode_provider_native); kept for reference only.
<#
.SYNOPSIS
    Bind / unbind a custom OpenAI-compatible LLM provider for the opencode agent,
    by MERGE-patching its global config (~/.config/opencode/opencode.json).

.DESCRIPTION
    opencode reads providers from a single global opencode.json. A custom
    OpenAI-compatible provider looks like:
        "provider": {
          "<id>": {
            "npm": "@ai-sdk/openai-compatible",
            "name": "<display name>",
            "options": { "baseURL": "...", "apiKey": "..." },
            "models": { "<model>": { "name": "<model>" } }
          }
        }
    and the active model is the top-level "model": "<id>/<model>".

      set   : merge provider.<id> (npm/name/options.baseURL), apiKey =
                -Key <literal>  |  -EnvKey <VAR> → "{env:VAR}"  |  (neither) → keep existing,
              add models.<model> if a -Model is given and set the top-level "model".
      clear : remove provider.<id> (and the top-level "model" if it points at it).

    EVERY other key (other providers, curated model lists, agent/compaction settings)
    is preserved. opencode.json is backed up (.bak) before writing; UTF-8 no BOM;
    -WhatIf previews. apiKey is stored as opencode itself stores it (machine-local).
    No Read-Host (dashboard-safe).

.EXAMPLE
    .\Manage-OpenCode-Provider.ps1 -Action set -ProviderId freellmapi -Name "FreeLLMAPI" -BaseUrl http://localhost:13001/v1 -Model auto
    .\Manage-OpenCode-Provider.ps1 -Action set -ProviderId freellmapi -BaseUrl http://localhost:13001/v1 -EnvKey FREELLMAPI_KEY
    .\Manage-OpenCode-Provider.ps1 -Action clear -ProviderId freellmapi -WhatIf
#>
param(
    [Parameter(Mandatory)][ValidateSet('set', 'clear')][string]$Action,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')][string]$ProviderId,
    [string]$Name,
    [string]$BaseUrl,
    [string]$Model,
    [string]$Key,
    [switch]$KeyStdin,
    [string]$EnvKey,
    [switch]$KeepKey,
    [switch]$WhatIf
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# -KeyStdin: read the literal apiKey from STDIN instead of -Key, so the secret never lands in the
# process command line (argv is world-readable on Windows via Win32_Process).
if ($KeyStdin) { $Key = ([Console]::In.ReadToEnd()).Trim() }

# Config path: $OPENCODE_CONFIG → $XDG_CONFIG_HOME\opencode → ~/.config/opencode.
$cfgPath =
if ($env:OPENCODE_CONFIG) { $env:OPENCODE_CONFIG }
elseif ($env:XDG_CONFIG_HOME) { Join-Path $env:XDG_CONFIG_HOME 'opencode\opencode.json' }
else { Join-Path $HOME '.config\opencode\opencode.json' }

$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

# Load opencode.json as a mutable hashtable (BOM-tolerant), or start minimal.
$cfg = @{}
if (Test-Path -LiteralPath $cfgPath) {
    try {
        # TrimStart handles the BOM; the old `-replace "^\xEF\xBB\xBF"` was dead (PS escapes with a
        # backtick, so `\xEF` was a literal backslash-x-E-F that never matched).
        $raw = (Get-Content -LiteralPath $cfgPath -Raw -Encoding UTF8).TrimStart([char]0xFEFF)
        if ($raw.Trim()) { $cfg = $raw | ConvertFrom-Json -AsHashtable }
    } catch {
        Write-Host "ОШИБКА: не удалось прочитать opencode.json ($($_.Exception.Message))." -ForegroundColor Red
        exit 1
    }
}
if ($cfg -isnot [hashtable]) { $cfg = @{} }
if (-not $cfg.ContainsKey('$schema')) { $cfg['$schema'] = 'https://opencode.ai/config.json' }
if (-not $cfg.ContainsKey('provider') -or $cfg['provider'] -isnot [hashtable]) { $cfg['provider'] = @{} }
$providers = $cfg['provider']

Write-Host "=== opencode provider: $Action $ProviderId ===" -ForegroundColor Cyan
if ($WhatIf) { Write-Host '[DRY RUN] изменения не применяются' -ForegroundColor Magenta }

if ($Action -eq 'set') {
    if (-not $BaseUrl) {
        Write-Host 'ОШИБКА: для set нужен -BaseUrl (или используйте clear).' -ForegroundColor Red
        exit 1
    }
    if (-not $providers.ContainsKey($ProviderId) -or $providers[$ProviderId] -isnot [hashtable]) {
        $providers[$ProviderId] = @{}
    }
    $p = $providers[$ProviderId]
    $p['npm'] = '@ai-sdk/openai-compatible'
    if ($Name) { $p['name'] = $Name } elseif (-not $p.ContainsKey('name')) { $p['name'] = $ProviderId }
    if (-not $p.ContainsKey('options') -or $p['options'] -isnot [hashtable]) { $p['options'] = @{} }
    $p['options']['baseURL'] = $BaseUrl
    # apiKey: literal -Key, else {env:VAR} via -EnvKey, else keep whatever is already there.
    if ($KeepKey) {
        # leave options.apiKey as-is
    } elseif ($Key) {
        $p['options']['apiKey'] = $Key
    } elseif ($EnvKey) {
        $p['options']['apiKey'] = "{env:$EnvKey}"
    }
    # Model: register it (preserve existing curated models) and make it the active model.
    if ($Model) {
        if (-not $p.ContainsKey('models') -or $p['models'] -isnot [hashtable]) { $p['models'] = @{} }
        if (-not $p['models'].ContainsKey($Model)) { $p['models'][$Model] = @{ name = $Model } }
        $cfg['model'] = "$ProviderId/$Model"
    }
    $keyShown = if ($KeepKey) { '(без изменений)' } elseif ($Key) { '(литерал)' } elseif ($EnvKey) { "{env:$EnvKey}" } else { '(без изменений)' }
    Write-Host "  baseURL=$BaseUrl  model=$(if($Model){"$ProviderId/$Model"}else{'—'})  apiKey=$keyShown" -ForegroundColor Gray
} else {
    [void]$providers.Remove($ProviderId)
    if ($cfg.ContainsKey('model') -and "$($cfg['model'])".StartsWith("$ProviderId/")) { [void]$cfg.Remove('model') }
    Write-Host "  Провайдер '$ProviderId' удалён из opencode.json." -ForegroundColor Gray
}

if ($WhatIf) {
    Write-Host '[DRY RUN] opencode.json не изменён. Итоговый provider:' -ForegroundColor Magenta
    if ($providers.ContainsKey($ProviderId)) {
        $opt = $providers[$ProviderId]['options']
        Write-Host ("    {0}: baseURL={1} apiKey={2}" -f $ProviderId, $opt['baseURL'], $(if ($opt.ContainsKey('apiKey')) { '***' } else { '(нет)' })) -ForegroundColor DarkGray
    } else { Write-Host "    (провайдер '$ProviderId' отсутствует)" -ForegroundColor DarkGray }
    exit 0
}

# Backup then write (UTF-8 no BOM).
$dir = Split-Path -Parent $cfgPath
if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
if (Test-Path -LiteralPath $cfgPath) { Copy-Item -LiteralPath $cfgPath -Destination "$cfgPath.bak" -Force }
[System.IO.File]::WriteAllText($cfgPath, ($cfg | ConvertTo-Json -Depth 12), $Utf8NoBom)
Write-Host "  opencode.json обновлён (бэкап .bak): $cfgPath" -ForegroundColor Green
exit 0
