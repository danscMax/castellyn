# DEPRECATED — not invoked by the app. Castellyn runs the native Rust port in
# src-tauri/src/lib.rs (probe_provider); kept for reference only.
<#
.SYNOPSIS
    Liveness check for an OpenAI/Anthropic-compatible provider: GET {root}/v1/models with the
    provider's API key (read from STDIN, never argv). Prints a one-line JSON result.

.DESCRIPTION
    STDIN: the API key (plain, optional — local engines may need none).
    -Protocol openai|anthropic selects the auth header (Bearer vs x-api-key + anthropic-version).
    The base URL is normalized: a trailing /v1 is stripped and /v1/models is always queried, so
    it works whether the user typed ".../v1" or just the host.
    Output (stdout): { "ok": true|false, "detail": "...", "count": <n> }  (single line)
#>
param(
    [Parameter(Mandatory)][string]$BaseUrl,
    [string]$Protocol = 'openai'
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$key = ([Console]::In.ReadToEnd()).Trim()
# Normalize: strip a trailing /v1 then always query /v1/models (works with or without /v1 in base).
$root = ($BaseUrl.TrimEnd('/')) -replace '/v1$', ''
$uri = "$root/v1/models"

$headers = @{}
if ($Protocol -eq 'anthropic') {
    if ($key) { $headers['x-api-key'] = $key }
    $headers['anthropic-version'] = '2023-06-01'
} else {
    if ($key) { $headers['Authorization'] = "Bearer $key" }
}

try {
    $r = Invoke-RestMethod -Method Get -Uri $uri -Headers $headers -TimeoutSec 12
    $n = 0
    if ($r.data) { $n = @($r.data).Count }
    elseif ($r.models) { $n = @($r.models).Count }
    elseif ($r -is [System.Array]) { $n = $r.Count }
    @{ ok = $true; detail = "ответил (моделей: $n)"; count = $n } | ConvertTo-Json -Compress
} catch {
    # A transport-level failure (DNS, TLS, timeout) carries no .Response — leave the status unknown
    # so the message below reports "не отвечает" instead of a bogus HTTP code.
    $st = $null; try { $st = [int]$_.Exception.Response.StatusCode } catch { $st = $null }
    $detail =
    if ($st -eq 401 -or $st -eq 403) { "ключ отклонён ($st)" }
    elseif ($st) { "ответ HTTP $st" }
    else { "не отвечает: $($_.Exception.Message)" }
    @{ ok = $false; detail = $detail } | ConvertTo-Json -Compress
}
