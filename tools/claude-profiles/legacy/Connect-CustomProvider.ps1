# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (connect_custom_native); kept for reference only.
<#
.SYNOPSIS
    Register a custom OpenAI-compatible provider in the freellmapi gateway via its admin API.

.DESCRIPTION
    POSTs to {Gateway}/api/keys/custom (the "Add a custom OpenAI-compatible model" surface of the
    freellmapi dashboard). The dashboard API is session-protected, so this script authenticates
    first: it uses a saved session token if present, otherwise logs in via POST /api/auth/login
    with email+password to obtain one. All secrets arrive on STDIN as one JSON object — never argv
    (argv is world-readable on Windows via WMI / Get-CimInstance Win32_Process). No Read-Host.

    STDIN (JSON): { "token": "", "email": "", "password": "", "apiKey": "" }
      - token present  → used directly as the dashboard session.
      - token empty    → POST /api/auth/login {email,password} → token.

.EXAMPLE
    '{"email":"me@x.io","password":"p","apiKey":"sk-..."}' | .\Connect-CustomProvider.ps1 `
        -Gateway http://localhost:13001 -BaseUrl https://api.deepseek.com/v1 -Model deepseek-chat -DisplayName DeepSeek
#>
param(
    [Parameter(Mandatory)][string]$Gateway,
    [Parameter(Mandatory)][string]$BaseUrl,
    [string]$Model,
    [string]$DisplayName,
    [string]$Label
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# Secrets via STDIN as a single JSON object.
$raw = [Console]::In.ReadToEnd()
try { $auth = $raw | ConvertFrom-Json } catch {
    Write-Host 'ОШИБКА: не удалось прочитать данные авторизации (STDIN).' -ForegroundColor Red; exit 1
}
$token = "$($auth.token)".Trim()
$email = "$($auth.email)".Trim()
$password = "$($auth.password)"
$apiKey = "$($auth.apiKey)".Trim()
$base = $Gateway.TrimEnd('/')

# Authenticate: reuse the token, else log in with email+password.
function Invoke-Login {
    Write-Host '  Вход в freellmapi (email+пароль)…' -ForegroundColor Gray
    $body = @{ email = $email; password = $password } | ConvertTo-Json
    $r = Invoke-RestMethod -Method Post -Uri "$base/api/auth/login" -Body $body -ContentType 'application/json'
    if (-not $r.token) { throw 'login не вернул токен' }
    return "$($r.token)"
}

if (-not $token) {
    if (-not $email -or -not $password) {
        Write-Host 'ОШИБКА: нет токена и неполные email/пароль freellmapi.' -ForegroundColor Red; exit 1
    }
    try { $token = Invoke-Login } catch {
        # No .Response on a transport-level failure (DNS/TLS/timeout) — keep the status unknown so
        # the branches below fall through to the generic message rather than inventing a code.
        $st = $null; try { $st = [int]$_.Exception.Response.StatusCode } catch { $st = $null }
        if ($st -eq 401) { Write-Host '  ОШИБКА входа (401): неверный email или пароль freellmapi.' -ForegroundColor Red }
        elseif ($st -eq 429) { Write-Host '  ОШИБКА входа (429): слишком много попыток, подождите ~15 мин.' -ForegroundColor Red }
        else { Write-Host "  ОШИБКА входа в freellmapi: $($_.Exception.Message)" -ForegroundColor Red }
        exit 1
    }
}

$uri = "$base/api/keys/custom"
$payload = @{
    baseUrl     = $BaseUrl
    displayName = if ($DisplayName) { $DisplayName } else { $BaseUrl }
}
if ($Label) { $payload['label'] = $Label }
if ($Model) { $payload['models'] = @($Model) }
if ($apiKey) { $payload['apiKey'] = $apiKey }

Write-Host '=== freellmapi: регистрация custom-провайдера ===' -ForegroundColor Cyan
Write-Host "  POST $uri  (baseUrl=$BaseUrl, model=$(if ($Model) { $Model } else { '—' }))" -ForegroundColor Gray

try {
    $resp = Invoke-RestMethod -Method Post -Uri $uri `
        -Headers @{ Authorization = "Bearer $token" } `
        -Body ($payload | ConvertTo-Json -Depth 6) -ContentType 'application/json'
    Write-Host "  OK: провайдер зарегистрирован (keyId=$($resp.keyId), platform=$($resp.platform))." -ForegroundColor Green
    if ($resp.models) { Write-Host "  Модели: $($resp.models -join ', ')" -ForegroundColor Gray }
    Write-Host '  Готово. Провайдер доступен через freellmapi (:13001) для Claude Code (ccr) и opencode.' -ForegroundColor Green
    exit 0
} catch {
    $msg = $_.Exception.Message
    # No .Response on a transport-level failure (DNS/TLS/timeout) — keep the status unknown so the
    # branches below fall through to the generic message rather than inventing a code.
    $st = $null; try { $st = [int]$_.Exception.Response.StatusCode } catch { $st = $null }
    if ($st -eq 401 -or $st -eq 403) {
        Write-Host "  ОШИБКА авторизации ($st): сессия freellmapi недействительна — переавторизуйтесь (Вход freellmapi)." -ForegroundColor Red
    } elseif ($st -eq 400) {
        Write-Host "  ОШИБКА (400): freellmapi отклонил baseUrl или тело запроса. $msg" -ForegroundColor Red
    } else {
        Write-Host "  ОШИБКА запроса к freellmapi: $msg" -ForegroundColor Red
    }
    exit 1
}
