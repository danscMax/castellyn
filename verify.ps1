#!/usr/bin/env pwsh
# Local CI — run every green-gate in order, stop at the first failure.
# Single source of truth for the gates: the pre-push hook (.githooks/pre-push) and
# `npm run verify` both call this. Mirrors the gate list in CLAUDE.md / docs/BUILD.md.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-StrictMode -Version Latest
Set-Location -LiteralPath $PSScriptRoot

# cargo isn't always on PATH (see memory: cargo-windows-invocation) — fall back to the default home.
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }

function Step($name, [scriptblock]$cmd) {
  Write-Host ""
  Write-Host "==> $name" -ForegroundColor Cyan
  & $cmd
  # Trust $LASTEXITCODE, not stderr: cargo/npm write progress to stderr without failing.
  if ($LASTEXITCODE -ne 0) {
    Write-Host "FAILED: $name (exit $LASTEXITCODE)" -ForegroundColor Red
    exit 1
  }
}

Step 'i18n parity (ru/en/zh)'   { npm run check:i18n }
Step 'svelte-check (types+i18n)' { npm run check }
Step 'vitest'                    { npm test }
Step 'frontend build'            { npm run build }
Step 'cargo clippy'              { & $cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings }
Step 'cargo test'                { & $cargo test  --manifest-path src-tauri/Cargo.toml }

Write-Host ""
Write-Host "All gates green." -ForegroundColor Green
