#!/usr/bin/env pwsh
# Local CI — run every green-gate in order, stop at the first failure.
# Single source of truth for the gates: the pre-push hook (.githooks/pre-push) and
# `npm run verify` both call this. Mirrors the gate list in CLAUDE.md / docs/BUILD.md.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Keep a failing native command reportable by Step() below instead of throwing straight out of the
# scriptblock (PowerShell 7.3+ turns a non-zero native exit into a terminating error by default).
$PSNativeCommandUseErrorActionPreference = $false
Set-Location -LiteralPath $PSScriptRoot

# cargo isn't always on PATH (see memory: cargo-windows-invocation) — fall back to the default home.
# Resolve in TWO steps on purpose: under `Set-StrictMode -Version Latest`, the old one-liner
# `(Get-Command cargo -EA SilentlyContinue).Source` leaves $cargo *unassigned* (not $null) when cargo
# is missing, so the very next line — the fallback itself — threw. With $ErrorActionPreference at its
# default the script then sailed on, skipped BOTH Rust gates while $LASTEXITCODE still held the 0 of
# the previous gate, and printed "All gates green." A missing toolchain must fail loudly, not quietly.
$cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
$cargo = if ($cargoCmd) { $cargoCmd.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargo)) {
  Write-Host "FAILED: cargo not found — not on PATH and no binary at $cargo" -ForegroundColor Red
  exit 1
}

$pssaSettings = Join-Path $PSScriptRoot 'PSScriptAnalyzerSettings.psd1'

function Step($name, [scriptblock]$cmd) {
  Write-Host ""
  Write-Host "==> $name" -ForegroundColor Cyan
  # Never inherit the previous gate's exit code — that is what let a skipped gate look green.
  $global:LASTEXITCODE = 0
  & $cmd
  # Trust $LASTEXITCODE, not stderr: cargo/npm write progress to stderr without failing.
  if ($LASTEXITCODE -ne 0) {
    Write-Host "FAILED: $name (exit $LASTEXITCODE)" -ForegroundColor Red
    exit 1
  }
}

Step 'i18n parity (ru/en/zh)'   { npm run check:i18n }
Step 'PSScriptAnalyzer'          {
  # The gate CI runs, run locally too — otherwise "single source of truth" above is a lie.
  if (-not (Get-Module -ListAvailable PSScriptAnalyzer)) {
    Write-Host "    installing PSScriptAnalyzer 1.22.0 (one-off, CurrentUser)" -ForegroundColor DarkGray
    Install-Module PSScriptAnalyzer -RequiredVersion 1.22.0 -Force -Scope CurrentUser -SkipPublisherCheck
  }
  Import-Module PSScriptAnalyzer
  # Every tracked PowerShell file, not just tools/ — build_all.ps1, verify.ps1 and the notify hook
  # live outside it and are just as capable of breaking a user's machine.
  $issues = @()
  foreach ($f in (git ls-files '*.ps1' '*.psm1')) {
    $issues += Invoke-ScriptAnalyzer -Path $f -Settings $pssaSettings
  }
  if ($issues) {
    $issues | Format-Table Severity, ScriptName, Line, RuleName -AutoSize
    Write-Host "PSScriptAnalyzer: $($issues.Count) issue(s)" -ForegroundColor Red
    $global:LASTEXITCODE = 1
  }
}
Step 'svelte-check (types+i18n)' { npm run check }
Step 'vitest'                    { npm test }
Step 'frontend build'            { npm run build }
Step 'cargo clippy'              { & $cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings }
Step 'cargo test'                { & $cargo test  --manifest-path src-tauri/Cargo.toml }

Write-Host ""
Write-Host "All gates green." -ForegroundColor Green
