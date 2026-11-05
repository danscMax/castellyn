<#
  fork-sync — entry point (Phase 1: read-only fork status reporter).
  Thin wrapper: sets UTF-8, takes a single-instance lock, imports the core
  module, runs Invoke-ForkSync, returns its exit code.

  Usage:
    .\update-forks.bat                  # pretty, interactive (double-click)
    pwsh -File .\update-forks.ps1       # same, from a shell
    pwsh -File .\update-forks.ps1 -NoFetch      # offline: use already-fetched refs
    pwsh -File .\update-forks.ps1 -Unattended   # no toast/prompts (scheduler)
#>
#Requires -Version 7.2
[CmdletBinding()]
param(
    [switch]$Unattended,
    [switch]$NoFetch,
    [switch]$NoGh,
    [string[]]$Roots,
    [string[]]$Paths,
    [int]$FetchTimeoutSec,
    [int]$GhTimeoutSec,
    # Phase 2 — safe mutations (all backed up; never auto force-push):
    [switch]$Apply,            # = -FfMain -DeleteMerged together
    [switch]$FfMain,           # fast-forward behind default branches
    [switch]$DeleteMerged,     # delete merged topic branches (local + fork)
    [switch]$NormalizeRemotes, # rename remotes to canon (origin=fork, upstream=original)
    [switch]$Rebase,           # rebase open PR branches onto fresh upstream (local; conflict→abort)
    [switch]$SyncWipLocal,     # rebase the personal wip-local branch onto fresh upstream (local; no push)
    [switch]$DeleteWip,        # delete a redundant wip-local that has no own commits (local; backed up)
    [switch]$Prune,            # prune stale ': gone]' tracking branches (local; backed up)
    [switch]$PushRebased,      # after rebase, force-with-lease push to update the PRs (asks)
    [switch]$DryRun,           # with an action flag: print the plan, change nothing
    [switch]$Yes,              # skip confirmations (for scripting / the skill)
    [string]$Single,          # strict single-repo mode (process ONLY this path) — concurrent-safe
    [string]$OutFile,         # write the status JSON here instead of the shared fork-sync.last.json
    [string]$ConfigPath,      # durable fork config (Castellyn %APPDATA%\castellyn\forks.json)
    [string]$ScriptsRoot
)

chcp 65001 | Out-Null
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$localScripts = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$sharedModule = Join-Path $localScripts 'fork-updater/GitRepository.psm1'
if (-not (Test-Path -LiteralPath $sharedModule)) { throw "Canonical fork-updater is missing: $sharedModule" }
Import-Module $sharedModule -Force
$resolvedRoot = Resolve-ScriptsRoot -ScriptsRoot $ScriptsRoot -StartPath $PSScriptRoot
Import-Module (Join-Path $resolvedRoot 'fork-updater/ForkSync.psm1') -Force

# Single-instance lock. A whole-stack run takes the global lock; a -Single run takes a PER-REPO
# lock so different repos run concurrently, while the same repo still can't double-run.
$lockName = if ($Single) { 'fork-sync.' + ($Single -replace '[^A-Za-z0-9]', '_') + '.lock' } else { 'fork-sync.lock' }
$lockPath = Join-Path $env:TEMP $lockName
$lockStream = $null
try {
    try { $lockStream = [System.IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None') }
    catch {
        Write-Host '  [!!] fork-sync уже запущен (lock занят). Выходим.' -ForegroundColor Red
        exit 3
    }

    $code = Invoke-ForkSync -Root $PSScriptRoot -Unattended:$Unattended -NoFetch:$NoFetch -NoGh:$NoGh `
        -Roots $Roots -Paths $Paths -FetchTimeoutSec $FetchTimeoutSec -GhTimeoutSec $GhTimeoutSec `
        -Apply:$Apply -FfMain:$FfMain -DeleteMerged:$DeleteMerged -NormalizeRemotes:$NormalizeRemotes `
        -Rebase:$Rebase -SyncWipLocal:$SyncWipLocal -DeleteWip:$DeleteWip -Prune:$Prune `
        -PushRebased:$PushRebased -DryRun:$DryRun -Yes:$Yes `
        -Single $Single -OutFile $OutFile -ConfigPath $ConfigPath -ScriptsRoot $resolvedRoot

    exit ([int]$code)
}
finally {
    # Only the instance that ACQUIRED the lock may release it — a second instance that lost the race
    # (no $lockStream) must not delete the holder's lock file.
    if ($lockStream) {
        $lockStream.Close(); $lockStream.Dispose()
        Remove-Item -LiteralPath $lockPath -ErrorAction SilentlyContinue
    }
}
