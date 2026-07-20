<#
.SYNOPSIS
  Build a COMPLETE fake world so an isolated Castellyn sees a "real" system where every
  button is safe to click: profiles, maintenance scripts, forks, agent CLIs.

.DESCRIPTION
  Writes ONLY under $World (default %TEMP%\castellyn-iso\world) — never the real %USERPROFILE%
  or E:\Scripts. Deterministic + idempotent: a normal run wipes any prior world and rebuilds it
  from scratch, so re-running gives the same structure. -Wipe just removes it.

  Layers (see tools/dev-iso-env/brief-iso-world.md):
    home\    -> becomes USERPROFILE  (.claude / .claude-cc1 / .claude-cc2, .codex, .opencode, .ssh, .agents)
    scripts\ -> becomes SCRIPTS_ROOT (manifest copy + per-component stubs, SettingsMCP\ClaudeProfiles, forks)
    bin\     -> prepended to PATH    (claude/codex/opencode/gh fakes; git/pwsh/node stay real)

  Fixture shapes are derived from live code, not invented:
    settings.json env keys ....... lib.rs read_profile_env (~5023)  ANTHROPIC_BASE_URL / *_SONNET_ / *_HAIKU_ / *_AUTH_TOKEN / HTTPS_PROXY
    .credentials.json ............ limits.rs read_access_token (~175) claudeAiOauth.accessToken
    .codex\auth.json ............. limits.rs read_codex_auth (~561)   tokens.access_token / account_id
    profiles.json ................ lib.rs profile_names (~4980)       profiles[].name
    .mcp.json .................... lib.rs read_mcp (~7828)            mcpServers{ name:{command} }
    schedules.last.json .......... schedules_watch.rs task_* + ScheduleTab.svelte  tasks[].{id,label,enabled,exists,lastResult,nextRun,lastRun}
    status envelope .............. tools/ScriptKit.ps1 Write-StatusJson (~252)      schemaVersion/component/status/timestamp/mode/durationSec/counts/summary
    agent-status file ............ assets/castellyn_status.py (~57)   {state,event,claudeSessionId,ts} in %APPDATA%\castellyn\agent-status\<sid>.json
    gh repo list JSON ............ lib.rs list_github_repos (~4922)   name/owner.login/nameWithOwner/isPrivate/isFork/isArchived/url/updatedAt/description/primaryLanguage.name/stargazerCount
    session CLI names ............ lib.rs launch (~14866, ~14954)     claude/codex/opencode, env CASTELLYN_SESSION_ID

.PARAMETER Wipe
  Remove the world and exit (no rebuild).

.PARAMETER Root
  Override the world root (default %TEMP%\castellyn-iso\world).
#>
param(
  [switch]$Wipe,
  [string]$Root = (Join-Path $env:TEMP 'castellyn-iso\world')
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$World    = $Root
$HomeDir  = Join-Path $World 'home'
$Scripts  = Join-Path $World 'scripts'
$Bin      = Join-Path $World 'bin'
$Settings = Join-Path $Scripts 'SettingsMCP'
$Profiles = Join-Path $Settings 'ClaudeProfiles'
$RepoRoot = Split-Path -Parent $PSScriptRoot   # tools\ -> repo root

# The world-building helpers below declare SupportsShouldProcess because their verbs promise state
# change. Impact stays default (Medium < the default $ConfirmPreference of High) so building the
# world never prompts. NOTE: -WhatIf is deliberately NOT wired into this script's own param block —
# a half-applied preview (dirs skipped, file writes still attempted) would crash mid-build, so the
# gates exist for a caller that dot-sources these helpers, not for a script-level dry run.
function New-Dir {
  [CmdletBinding(SupportsShouldProcess)]
  param([string]$p)
  if (-not (Test-Path -LiteralPath $p) -and $PSCmdlet.ShouldProcess($p, 'create directory')) {
    New-Item -ItemType Directory -Path $p -Force -Confirm:$false | Out-Null
  }
}

# UTF-8 WITHOUT BOM — Castellyn's own writers never emit a BOM (CLAUDE.md convention).
function Write-Text([string]$path, [string]$text) {
  New-Dir (Split-Path -Parent $path)
  [System.IO.File]::WriteAllText($path, $text, [System.Text.UTF8Encoding]::new($false))
}
function Write-Json([string]$path, $obj) { Write-Text $path ($obj | ConvertTo-Json -Depth 12) }

# Robust recursive delete: a running/recently-stopped sandbox can leave a handle on a log
# (e.g. a stray serena mcp writing under world\home\.serena\logs) that makes a plain Remove-Item
# throw and abort the whole rebuild, corrupting the world half-way. Retry a few times, and on the
# last try skip the locked leftovers rather than failing — the important tree (profiles/scripts)
# always rebuilds. (live find 2026-07-18: a re-gen over a live instance wiped the profiles and died.)
function Remove-WorldTree {
  [CmdletBinding(SupportsShouldProcess)]
  param([string]$path)
  if (-not (Test-Path -LiteralPath $path)) { return }
  if (-not $PSCmdlet.ShouldProcess($path, 'delete the sandbox world tree')) { return }
  foreach ($attempt in 1..4) {
    try { Remove-Item -LiteralPath $path -Recurse -Force -Confirm:$false -ErrorAction Stop; return }
    catch {
      if ($attempt -eq 4) {
        # Best-effort file-by-file; leave whatever is still locked, warn, continue.
        Get-ChildItem -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue |
          Sort-Object FullName -Descending |
          ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -Recurse -Confirm:$false -ErrorAction SilentlyContinue }
        Write-Host "   ⚠ часть файлов залочена (запущенный инстанс?) — пересобираю поверх." -ForegroundColor DarkYellow
        return
      }
      Start-Sleep -Milliseconds (200 * $attempt)
    }
  }
}

# ── -Wipe ────────────────────────────────────────────────────────────────────
if ($Wipe) {
  if (Test-Path -LiteralPath $World) {
    Remove-WorldTree $World
    Write-Host "🧹 Мир снесён: $World" -ForegroundColor Yellow
  } else {
    Write-Host "   (мир не существовал: $World)" -ForegroundColor DarkGray
  }
  return
}

# Deterministic rebuild: nuke any prior world first so a re-run reproduces the same tree.
Remove-WorldTree $World
New-Dir $World; New-Dir $HomeDir; New-Dir $Scripts; New-Dir $Bin

Write-Host "▶  Строю мир: $World" -ForegroundColor Cyan

# ════════════════════════════════════════════════════════════════════════════
# LAYER 1 — home\  (becomes USERPROFILE)
# ════════════════════════════════════════════════════════════════════════════

# One profile home: settings.json (env block, dummy token), .credentials.json (expired), hooks\.
function New-ProfileHome {
  [CmdletBinding(SupportsShouldProcess)]
  param([string]$dirName, [int]$n, [hashtable]$env)
  $dir = Join-Path $HomeDir $dirName
  if (-not $PSCmdlet.ShouldProcess($dir, 'create a fake profile home')) { return }
  New-Dir $dir
  New-Dir (Join-Path $dir 'hooks')
  Write-Json (Join-Path $dir 'settings.json') ([ordered]@{
    '$schema'    = 'https://json.schemastore.org/claude-code-settings.json'
    env          = $env
    includeCoAuthoredBy = $false
  })
  # accessToken is a dummy: the usage endpoint answers 401 -> UI shows "expired" (honest + safe).
  Write-Json (Join-Path $dir '.credentials.json') ([ordered]@{
    claudeAiOauth = [ordered]@{ accessToken = "iso-dummy-token-$n"; refreshToken = "iso-dummy-refresh-$n"; expiresAt = 0 }
  })
  # .claude.json: the per-profile MCP deployment target. read_mcp enumerates profiles by THIS file's
  # top-level mcpServers (lib.rs profile_mcp_servers ~7811) — without it the MCP tab shows no profiles
  # and "Развернуть во все профили" has nowhere to write (live find 2026-07-18). One pre-deployed
  # server on cc1 so the deployed-count column shows a non-zero state out of the box.
  $mcp = if ($dirName -eq '.claude-cc1') { [ordered]@{ 'iso-echo' = [ordered]@{ command = 'node'; args = @('iso-echo-server.js') } } } else { [ordered]@{} }
  Write-Json (Join-Path $dir '.claude.json') ([ordered]@{ mcpServers = $mcp })
}

# `.claude` — the native/default profile (no provider env, uses login).
New-ProfileHome '.claude' 1 ([ordered]@{})
# cc1 — direct Anthropic-shaped env.
New-ProfileHome '.claude-cc1' 2 ([ordered]@{
  ANTHROPIC_BASE_URL             = 'https://api.anthropic.com'
  ANTHROPIC_AUTH_TOKEN           = 'iso-dummy-token-2'
  ANTHROPIC_DEFAULT_SONNET_MODEL = 'claude-sonnet-5'
  ANTHROPIC_DEFAULT_HAIKU_MODEL  = 'claude-haiku-4-5-20251001'
})
# cc2 — routed through a fake local gateway + proxy (exercises the proxy column).
New-ProfileHome '.claude-cc2' 3 ([ordered]@{
  ANTHROPIC_BASE_URL             = 'http://127.0.0.1:8787'
  ANTHROPIC_AUTH_TOKEN           = 'iso-dummy-token-3'
  ANTHROPIC_DEFAULT_SONNET_MODEL = 'glm-4.6'
  ANTHROPIC_DEFAULT_HAIKU_MODEL  = 'glm-4.5-air'
  HTTPS_PROXY                    = 'http://127.0.0.1:8888'
})

# Codex creds — read_codex_auth shape (tokens.access_token / account_id).
Write-Json (Join-Path $HomeDir '.codex\auth.json') ([ordered]@{ tokens = [ordered]@{ access_token = 'iso-dummy'; account_id = 'iso-acc' } })
Write-Text (Join-Path $HomeDir '.codex\config.toml') "# iso sandbox codex config`nmodel = `"gpt-5-codex`"`n"

New-Dir (Join-Path $HomeDir '.opencode')
New-Dir (Join-Path $HomeDir '.agents\skills')

# One fake SSH host pointing at loopback (safe: connect goes nowhere useful).
Write-Text (Join-Path $HomeDir '.ssh\config') "Host iso-remote`n    HostName 127.0.0.1`n    User iso`n    Port 22`n"

# ════════════════════════════════════════════════════════════════════════════
# LAYER 2 — scripts\  (becomes SCRIPTS_ROOT)
# ════════════════════════════════════════════════════════════════════════════

# 2a. Manifest — verbatim copy of the repo's canonical manifest.
$manifestSrc = Join-Path $RepoRoot 'manifest\maintenance-manifest.json'
$manifestDst = Join-Path $Scripts 'Castellyn\manifest\maintenance-manifest.json'
New-Dir (Split-Path -Parent $manifestDst)
Copy-Item -LiteralPath $manifestSrc -Destination $manifestDst -Force
$manifest = Get-Content -Raw -LiteralPath $manifestDst | ConvertFrom-Json

# Expand a manifest rel-path the same way lib.rs abs()/expand_placeholders does.
function Resolve-WorldPath([string]$rel) {
  $p = $rel.Replace('{{PROFILES}}', $Profiles).Replace('{{SCRIPTS_ROOT}}', $Scripts).Replace('{{SETTINGS}}', $Settings).Replace('{{USERPROFILE}}', $HomeDir)
  if ([System.IO.Path]::IsPathRooted($p)) { return $p }
  return (Join-Path $Scripts $p)
}

# 2b. Per-component stub scripts. Each writes a VALID status envelope to its lastJsonRel path,
# with the outcome driven by env ISO_OUTCOME (ok|changes|error|held) so the clicker can exercise
# every status branch. Envelope shape mirrors ScriptKit.ps1 Write-StatusJson (atomic temp+move).
$stubTemplate = @'
# ISO sandbox stub — writes a status envelope, does NOTHING real. Outcome via $env:ISO_OUTCOME.
$ErrorActionPreference = 'Stop'
$outcome = if ($env:ISO_OUTCOME) { $env:ISO_OUTCOME } else { 'ok' }
if ('ok','changes','error','held' -notcontains $outcome) { $outcome = 'ok' }
$mode = if ($args -match '^-Check$|^-DryRun$') { 'check' } else { 'apply' }
Start-Sleep -Milliseconds 400
$dur = [math]::Round((Get-Random -Minimum 10 -Maximum 30) / 10.0, 1)  # reported 1.0-3.0s
switch ($outcome) {
  'changes' { $changed = 3; $failed = 0; $total = 5; $summary = 'iso: 3 items updated' }
  'error'   { $changed = 0; $failed = 2; $total = 5; $summary = 'iso: 2 items failed' }
  'held'    { $changed = 0; $failed = 0; $total = 0; $summary = 'iso: held (patched locally)' }
  default   { $changed = 0; $failed = 0; $total = 5; $summary = 'iso: nothing to do' }
}
$payload = [ordered]@{
  schemaVersion = 1
  component     = '__COMPONENT__'
  status        = $outcome
  timestamp     = (Get-Date -Format 'o')
  mode          = $mode
  durationSec   = $dur
  counts        = [ordered]@{ changed = [int]$changed; failed = [int]$failed; total = [int]$total }
  summary       = $summary
}
$out = '__OUTFILE__'
$dir = Split-Path -Parent $out
if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
$tmp = "$out.tmp"
[System.IO.File]::WriteAllText($tmp, ($payload | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::Move($tmp, $out, $true)
Write-Host "[iso-stub] __COMPONENT__ -> $outcome ($summary)"
if ($outcome -eq 'error') { exit 1 } else { exit 0 }
'@

$stubCount = 0
foreach ($c in $manifest.components) {
  $scriptPath = Resolve-WorldPath $c.scriptRel
  $lastJson   = if ($c.lastJsonRel) { Resolve-WorldPath $c.lastJsonRel } else { Join-Path (Split-Path -Parent $scriptPath) ("{0}.last.json" -f $c.id) }
  $text = $stubTemplate.Replace('__COMPONENT__', $c.id).Replace('__OUTFILE__', $lastJson)
  Write-Text $scriptPath $text
  $stubCount++
}

# 2c. SettingsMCP\ClaudeProfiles config tree.
Write-Json (Join-Path $Profiles 'config\profiles.json') ([ordered]@{
  profiles = @(
    [ordered]@{ name = 'cc1'; color = 'Blue';  description = 'ISO sandbox profile 1'; linkedFolders = @() }
    [ordered]@{ name = 'cc2'; color = 'Green'; description = 'ISO sandbox profile 2'; linkedFolders = @() }
  )
})

# Profiles health: the Profiles tab runs {{PROFILES}}\Get-ProfilesStatus.ps1 and reads
# profiles.last.json next to it (lib.rs PROFILES_SCRIPT_REL/PROFILES_JSON_REL ~1944; live find
# 2026-07-18: without the script the tab errors code 64). Shape mirrors the real generator's
# output: one healthy profile, one with problems, plus the default — so every row-state renders.
$profilesStatus = {
  [ordered]@{
    generatedAt = (Get-Date -Format 'o'); machineName = 'ISO-SANDBOX'; isAdmin = $false
    profiles = @(
      [ordered]@{ name = 'cc1'; description = 'ISO sandbox profile 1'; color = 'Blue'
        exists = $true; credentialsPresent = $true; credentialsValid = $true; settingsPresent = $true
        onboardingComplete = $true; needsOnboarding = $false; logoutResidue = $false
        sharedLinks = 3; linksIntact = $true; linksValid = $true; linkProblems = @() }
      [ordered]@{ name = 'cc2'; description = 'ISO sandbox profile 2'; color = 'Green'
        exists = $true; credentialsPresent = $true; credentialsValid = $false; settingsPresent = $true
        onboardingComplete = $false; needsOnboarding = $true; logoutResidue = $true
        sharedLinks = 3; linksIntact = $false; linksValid = $false; linkProblems = @('settings.json link broken') }
    )
    backup = [ordered]@{ lastRun = (Get-Date).AddHours(-20).ToString('o'); lastSnapshot = 'snapshot-iso'
      ageHours = 20; stale = $false; snapshotPresent = $true }
    syncConflicts = [ordered]@{ count = 1; files = @('config\.mcp.sync-conflict-iso.json') }
  }
}
Write-Json (Join-Path $Profiles 'profiles.last.json') (& $profilesStatus)
Write-Text (Join-Path $Profiles 'Get-ProfilesStatus.ps1') @'
# ISO stub: refresh profiles.last.json with the same fixture the world was built with, bumping
# generatedAt so the "last run" visibly moves. `-CleanConflicts` (run_profiles clean-conflicts) also
# zeroes the syncConflicts block so the "clean" action shows a real effect. Exit 0 always (the tab
# treats non-zero as an error).
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$p = Join-Path $here 'profiles.last.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  $j.generatedAt = (Get-Date -Format 'o')
  if (($args -contains '-CleanConflicts') -and $j.syncConflicts) {
    $j.syncConflicts.count = 0
    $j.syncConflicts.files = @()
    Write-Host 'iso: конфликты синхронизации очищены (песочница)'
  }
  $j | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
exit 0
'@

# Canonical shared MCP servers (2 fakes) — read_mcp surfaces name+command from mcpServers.
Write-Json (Join-Path $Profiles 'config\.mcp.json') ([ordered]@{
  mcpServers = [ordered]@{
    'iso-echo'  = [ordered]@{ command = 'node'; args = @('iso-echo-server.js') }
    'iso-fetch' = [ordered]@{ command = 'npx';  args = @('-y', 'iso-fetch-mcp') }
  }
})

# Schedules fixture — covers every ScheduleTab branch + the watcher's failed-transition path:
# ok / failed / disabled / not-created. lastResult 0=ok, non-zero=fail (schedules_watch task_failed).
# A scriptblock so the self-check can restore it after exercising Schedule-Hub.ps1 (which mutates it).
$schedulesFixture = {
  $now = Get-Date
  [ordered]@{
    schemaVersion = 1
    timestamp     = (Get-Date -Format 'o')
    tasks = @(
      [ordered]@{ id = 'update-all'; label = 'Обновить всё (ночью)'; exists = $true;  enabled = $true;  time = '03:00'; defaultTime = '03:00'; nextRun = $now.AddHours(9).ToString('o');  lastRun = $now.AddHours(-15).ToString('o'); lastResult = 0 }
      [ordered]@{ id = 'forks-sync'; label = 'Синхронизация форков';  exists = $true;  enabled = $true;  time = '04:30'; defaultTime = '04:30'; nextRun = $now.AddHours(10).ToString('o'); lastRun = $now.AddHours(-14).ToString('o'); lastResult = 1 }
      [ordered]@{ id = 'plugins';    label = 'Обновление плагинов';   exists = $true;  enabled = $false; time = '05:00'; defaultTime = '05:00'; nextRun = $null;                            lastRun = $null;                             lastResult = $null }
      [ordered]@{ id = 'cargo';      label = 'Cargo-бинарники';        exists = $false; enabled = $false; time = $null;   defaultTime = '06:00'; nextRun = $null;                            lastRun = $null;                             lastResult = $null }
    )
  }
}
Write-Json (Join-Path $Profiles 'schedules.last.json') (& $schedulesFixture)

# Config-drift fixture (links.last.json) — the ConfigDriftStatus shape read_config_drift returns
# (ipc.ts ConfigDriftStatus ~797): one unlinked + one drifted so the tab's Relink / Sync-now actions
# have something to fix. Check-Integrity.ps1 rewrites it; Relink-SharedConfig.ps1 clears `unlinked`.
# A scriptblock for the same restore-after-self-check reason as the schedules fixture.
$linksFixture = {
  [ordered]@{
    generatedAt = (Get-Date -Format 'o'); drifted = 1; unlinked = 1; ok = $false
    items = @(
      [ordered]@{ name = '.mcp.json';            state = 'ok' }
      [ordered]@{ name = 'settings-shared.json'; state = 'unlinked' }
      [ordered]@{ name = 'keybindings.json';     state = 'drifted' }
    )
  }
}
Write-Json (Join-Path $Profiles 'links.last.json') (& $linksFixture)

# ── 2c-bis. Control-action stubs ────────────────────────────────────────────
# Every write/read PS script Castellyn spawns for a button (paths from lib.rs *_SCRIPT_REL, all under
# {{PROFILES}} except Stack-Procs). Each: catch-all param() so an unknown flag never fails, UTF-8, a
# progress line, a SAFE in-world mutation (so the UI sees a real effect), exit 0. Args mirror the live
# run_* callers (run_backup/run_profiles/run_mcp/run_schedule/run_config_drift/run_profile_mgmt/…).
# Restore point for the two profile-home MCP files Deploy-Mcp.ps1 mutates (self-check reverts to this).
function Set-ProfileMcp {
  [CmdletBinding(SupportsShouldProcess)]
  param([string]$dirName, $servers)
  $target = Join-Path $HomeDir (Join-Path $dirName '.claude.json')
  if (-not $PSCmdlet.ShouldProcess($target, 'write mcpServers')) { return }
  Write-Json $target ([ordered]@{ mcpServers = $servers })
}
$cc1McpSeed = { [ordered]@{ 'iso-echo' = [ordered]@{ command = 'node'; args = @('iso-echo-server.js') } } }

# Ordered so the self-check runs Deploy-Mcp before probing its effect; also the write order.
$controlStubNames = @(
  'Backup-ClaudeSetup.ps1', 'Restore-ClaudeSetup.ps1', 'Install-ClaudeProfiles.ps1',
  'Repair-ProfileLinks.ps1', 'Repair-Onboarding.ps1', 'Relink-SharedConfig.ps1',
  'Check-Integrity.ps1', 'Manage-Profiles.ps1', 'Deploy-Mcp.ps1', 'Schedule-Hub.ps1',
  'Deploy-ManagedSettings.ps1', 'Configure-Syncthing.ps1', 'Assert-Installation.ps1'
)
$controlStubs = @{}

# Backup-ClaudeSetup.ps1 — run_backup: -Force [-KeepSnapshots N] | -DeleteSnapshot <id>. Also the
# config-drift `sync-now` (-Force). WRITE: make/prune real snapshot dirs under Backups (list_backups
# reads the dir listing + .backup-state.json).
$controlStubs['Backup-ClaudeSetup.ps1'] = @'
param([switch]$Force, [int]$KeepSnapshots = 0, [string]$DeleteSnapshot, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$backups = Join-Path $PSScriptRoot 'Backups'
if (-not (Test-Path -LiteralPath $backups)) { New-Item -ItemType Directory -Path $backups -Force | Out-Null }
if ($DeleteSnapshot) {
  if ($DeleteSnapshot -notmatch '^[0-9]{4}-[0-9]{2}-[0-9]{2}_[0-9]{6}$') { Write-Host "iso: неверный id снапшота: $DeleteSnapshot"; exit 1 }
  $d = Join-Path $backups $DeleteSnapshot
  if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force }
  Write-Host "iso: снапшот удалён (песочница): $DeleteSnapshot"
  exit 0
}
$ts = Get-Date -Format 'yyyy-MM-dd_HHmmss'
$snap = Join-Path $backups $ts
New-Item -ItemType Directory -Path $snap -Force | Out-Null
'iso sandbox snapshot' | Set-Content -LiteralPath (Join-Path $snap 'MANIFEST.txt') -Encoding utf8NoBOM
if ($KeepSnapshots -gt 0) {
  $all = @(Get-ChildItem -LiteralPath $backups -Directory | Where-Object { $_.Name -match '^[0-9]{4}-[0-9]{2}-[0-9]{2}_[0-9]{6}$' } | Sort-Object Name -Descending)
  if ($all.Count -gt $KeepSnapshots) { $all[$KeepSnapshots..($all.Count - 1)] | ForEach-Object { Remove-Item -LiteralPath $_.FullName -Recurse -Force } }
}
$state = [ordered]@{ lastRun = (Get-Date -Format 'o'); lastManifestHash = 'iso'; lastWeekly = $null; lastSnapshot = $ts }
$state | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $backups '.backup-state.json') -Encoding utf8NoBOM
Write-Host "iso: снапшот создан (песочница): $ts"
exit 0
'@

# Restore-ClaudeSetup.ps1 — run_backup restore/restore-preview: -WhatIf [-Timestamp t] [-Profiles ...]
# [-IncludeCredentials]. NO-OP by design: overwriting the profile homes from a snapshot is exactly the
# destructive move a sandbox must not do. Honest log, never fakes a counter.
$controlStubs['Restore-ClaudeSetup.ps1'] = @'
param([switch]$WhatIf, [string]$Timestamp, [string[]]$Profiles, [switch]$IncludeCredentials, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if ($WhatIf) { Write-Host 'iso: предпросмотр восстановления — файлы НЕ менялись (песочница)' }
else { Write-Host 'iso: восстановление пропущено в песочнице — перезапись профилей небезопасна (no-op)' }
if ($Timestamp) { Write-Host "iso: снапшот: $Timestamp" }
exit 0
'@

# Install-ClaudeProfiles.ps1 — run_profiles reinstall: -Force. WRITE: ensure a ~/.claude-<name> home
# (settings.json + .claude.json) for every profiles.json name (profile_names / profile_mcp_servers).
$controlStubs['Install-ClaudeProfiles.ps1'] = @'
param([switch]$Force, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$names = @()
try { $names = (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'config\profiles.json') -Raw | ConvertFrom-Json).profiles.name } catch { }
$made = 0
foreach ($name in $names) {
  $dir = Join-Path $env:USERPROFILE ".claude-$name"
  if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null; $made++ }
  $sj = Join-Path $dir 'settings.json'
  if (-not (Test-Path -LiteralPath $sj)) { '{ "env": {} }' | Set-Content -LiteralPath $sj -Encoding utf8NoBOM }
  $cj = Join-Path $dir '.claude.json'
  if (-not (Test-Path -LiteralPath $cj)) { '{ "mcpServers": {} }' | Set-Content -LiteralPath $cj -Encoding utf8NoBOM }
}
Write-Host "iso: профили переустановлены — новых домов: $made (песочница)"
exit 0
'@

# Repair-ProfileLinks.ps1 — run_profiles repair/create + repair_all_profiles: -Name <n>. WRITE: flip
# that profile's link health in profiles.last.json (read_profiles) so the row goes healthy.
$controlStubs['Repair-ProfileLinks.ps1'] = @'
param([string]$Name, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'profiles.last.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  foreach ($pr in $j.profiles) { if ($pr.name -eq $Name) { $pr.linksIntact = $true; $pr.linksValid = $true; $pr.linkProblems = @() } }
  $j.generatedAt = (Get-Date -Format 'o')
  $j | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
Write-Host "iso: ссылки профиля '$Name' восстановлены (песочница)"
exit 0
'@

# Repair-Onboarding.ps1 — run_profiles fix-onboarding: -Name <n>. WRITE: clear the post-/logout
# onboarding residue for that profile in profiles.last.json.
$controlStubs['Repair-Onboarding.ps1'] = @'
param([string]$Name, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'profiles.last.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  foreach ($pr in $j.profiles) { if ($pr.name -eq $Name) { $pr.onboardingComplete = $true; $pr.needsOnboarding = $false; $pr.logoutResidue = $false } }
  $j.generatedAt = (Get-Date -Format 'o')
  $j | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
Write-Host "iso: онбординг профиля '$Name' восстановлен (песочница)"
exit 0
'@

# Relink-SharedConfig.ps1 — run_config_drift relink: -NonInteractive (real one self-elevates via UAC).
# WRITE: clear `unlinked` in links.last.json so the drift tab shows the relink took effect.
$controlStubs['Relink-SharedConfig.ps1'] = @'
param([switch]$NonInteractive, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'links.last.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  foreach ($it in $j.items) { if ($it.state -eq 'unlinked') { $it.state = 'ok' } }
  $j.unlinked = 0
  $j.ok = ($j.drifted -eq 0 -and $j.unlinked -eq 0)
  $j.generatedAt = (Get-Date -Format 'o')
  $j | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
Write-Host 'iso: общие конфиг-ссылки восстановлены (песочница)'
exit 0
'@

# Check-Integrity.ps1 — run_config_drift check: no args. READ-snapshot: (re)write links.last.json
# with the drifted+unlinked fixture (read_config_drift consumes it).
$controlStubs['Check-Integrity.ps1'] = @'
param([Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'links.last.json'
$payload = [ordered]@{
  generatedAt = (Get-Date -Format 'o'); drifted = 1; unlinked = 1; ok = $false
  items = @(
    [ordered]@{ name = '.mcp.json';            state = 'ok' }
    [ordered]@{ name = 'settings-shared.json'; state = 'unlinked' }
    [ordered]@{ name = 'keybindings.json';     state = 'drifted' }
  )
}
$payload | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
Write-Host 'iso: целостность ссылок проверена (песочница)'
exit 0
'@

# Manage-Profiles.ps1 — run_profile_mgmt: -Action add|remove|rename|recolor|redescribe|set-links
# -Name <n> [-NewName][-Color][-Description][-Enabled a,b]. WRITE: mutate config\profiles.json
# (read_profiles_config), the canonical profile list.
$controlStubs['Manage-Profiles.ps1'] = @'
param([string]$Action, [string]$Name, [string]$NewName, [string]$Color, [string]$Description, [string]$Enabled, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'config\profiles.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  $list = @($j.profiles)
  switch ($Action) {
    'add' {
      if (-not ($list | Where-Object { $_.name -eq $Name })) {
        $c = if ($Color) { $Color } else { 'White' }
        $list += [pscustomobject]@{ name = $Name; color = $c; description = $Description; linkedFolders = @() }
      }
    }
    'remove'     { $list = @($list | Where-Object { $_.name -ne $Name }) }
    'rename'     { foreach ($it in $list) { if ($it.name -eq $Name) { $it.name = $NewName } } }
    'recolor'    { foreach ($it in $list) { if ($it.name -eq $Name) { $it.color = $Color } } }
    'redescribe' { foreach ($it in $list) { if ($it.name -eq $Name) { $it.description = $Description } } }
    'set-links'  {
      $lf = if ($Enabled) { @($Enabled -split ',') } else { @() }
      foreach ($it in $list) { if ($it.name -eq $Name) { $it | Add-Member -NotePropertyName linkedFolders -NotePropertyValue $lf -Force } }
    }
  }
  $j.profiles = @($list)
  $j | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
Write-Host "iso: профиль '$Name' — действие '$Action' применено (песочница)"
exit 0
'@

# Deploy-Mcp.ps1 — run_mcp deploy: [-Only a,b]. WRITE (flagship): copy every canonical server from
# config\.mcp.json into each targeted profile's ~/.claude-<name>/.claude.json top-level mcpServers, so
# read_mcp's "развёрнут N/M" column actually grows. Real merge, respects -Only.
$controlStubs['Deploy-Mcp.ps1'] = @'
param([string]$Only, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$root = $PSScriptRoot
$src = $null
try { $src = (Get-Content -LiteralPath (Join-Path $root 'config\.mcp.json') -Raw | ConvertFrom-Json).mcpServers } catch { }
if (-not $src) { Write-Host 'iso: канонический .mcp.json пуст — нечего разворачивать'; exit 0 }
$names = @()
try { $names = (Get-Content -LiteralPath (Join-Path $root 'config\profiles.json') -Raw | ConvertFrom-Json).profiles.name } catch { }
if ($Only) { $want = @($Only -split ','); $names = @($names | Where-Object { $want -contains $_ }) }
$n = 0
foreach ($name in $names) {
  $cj = Join-Path $env:USERPROFILE ".claude-$name\.claude.json"
  $doc = $null
  if (Test-Path -LiteralPath $cj) { try { $doc = Get-Content -LiteralPath $cj -Raw | ConvertFrom-Json } catch { } }
  if (-not $doc) { $doc = [pscustomobject]@{} }
  if (-not $doc.PSObject.Properties['mcpServers']) { $doc | Add-Member -NotePropertyName mcpServers -NotePropertyValue ([pscustomobject]@{}) -Force }
  foreach ($prop in $src.PSObject.Properties) { $doc.mcpServers | Add-Member -NotePropertyName $prop.Name -NotePropertyValue $prop.Value -Force }
  $dir = Split-Path -Parent $cj
  if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $doc | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $cj -Encoding utf8NoBOM
  $n++
  Write-Host "iso: MCP -> .claude-$name (песочница)"
}
Write-Host "iso: MCP развёрнут в профилей: $n (песочница)"
exit 0
'@

# Schedule-Hub.ps1 — read_schedules/run_schedule: -Action query|create|enable|disable|run|delete
# [-Id <id>][-Time HH:mm]. WRITE: mutate the matching task in schedules.last.json (read_schedules).
$controlStubs['Schedule-Hub.ps1'] = @'
param([string]$Action, [string]$Id, [string]$Time, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$p = Join-Path $PSScriptRoot 'schedules.last.json'
try {
  $j = Get-Content -LiteralPath $p -Raw | ConvertFrom-Json
  $now = Get-Date
  $t = $j.tasks | Where-Object { $_.id -eq $Id } | Select-Object -First 1
  switch ($Action) {
    'enable'  { if ($t) { $t.exists = $true; $t.enabled = $true; if (-not $t.time) { $t.time = $t.defaultTime }; $t.nextRun = $now.AddHours(6).ToString('o') } }
    'disable' { if ($t) { $t.enabled = $false } }
    'run'     { if ($t) { $t.lastRun = $now.ToString('o'); $t.lastResult = 0; $t.nextRun = $now.AddHours(6).ToString('o') } }
    'create'  { if ($t) { $t.exists = $true; $t.enabled = $true; if ($Time) { $t.time = $Time } elseif (-not $t.time) { $t.time = $t.defaultTime }; $t.nextRun = $now.AddHours(6).ToString('o') } }
    'delete'  { if ($t) { $t.exists = $false; $t.enabled = $false; $t.time = $null; $t.nextRun = $null; $t.lastRun = $null; $t.lastResult = $null } }
  }
  $j.timestamp = (Get-Date -Format 'o')
  $j | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $p -Encoding utf8NoBOM
} catch { }
if ($Action -ne 'query') { Write-Host "iso: расписание '$Id' — '$Action' применено (песочница)" }
exit 0
'@

# Deploy-ManagedSettings.ps1 — run_managed_deploy (elevated). NO-OP: the real target is the machine's
# ProgramData managed-settings.json, outside the sandbox world. Drift is recomputed natively.
$controlStubs['Deploy-ManagedSettings.ps1'] = @'
param([Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Write-Host 'iso: managed-settings — системный путь ProgramData вне песочницы, пропущено (no-op)'
exit 0
'@

# Configure-Syncthing.ps1 — run_onboarding_step syncthing. NO-OP: Syncthing's REST API isn't running
# in the sandbox, so there's nothing to harden. Honest log.
$controlStubs['Configure-Syncthing.ps1'] = @'
param([Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Write-Host 'iso: Syncthing REST недоступен в песочнице — версионирование пропущено (no-op)'
exit 0
'@

# Assert-Installation.ps1 — run_onboarding_step verify. READ: print an assertion report, exit 0
# (run_onboarding_step just streams stdout to the log; there is no .last.json to write).
$controlStubs['Assert-Installation.ps1'] = @'
param([Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Write-Host 'iso: проверка установки (песочница)'
Write-Host '  ✓ дерево профилей на месте'
Write-Host '  ✓ манифест обслуживания читается'
Write-Host '  ✓ CLI-заглушки на PATH'
exit 0
'@

foreach ($nm in $controlStubNames) { Write-Text (Join-Path $Profiles $nm) $controlStubs[$nm] }

# Stack-Procs.ps1 — read_stack_procs: -Ports "a,b,c" -> JSON [{port,pid,uptimeSec}]. Lives under
# Castellyn\tools\stack (NOT {{PROFILES}}). Nothing of the stack listens in the sandbox -> empty array.
$stackProcsPath = Join-Path $Scripts 'Castellyn\tools\stack\Stack-Procs.ps1'
Write-Text $stackProcsPath @'
param([string]$Ports, [Parameter(ValueFromRemainingArguments = $true)]$rest)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
# No stack service actually listens in the sandbox; read_stack_procs tolerates an empty set.
Write-Output '[]'
exit 0
'@

# 2d. Forks — real git repos with a local bare remote. alpha clean, beta dirty + a branch.
$forks = Join-Path $Scripts 'forks'
$bare  = Join-Path $forks '.bare'
New-Dir $bare
New-Dir (Join-Path $Scripts 'External')   # present but empty

function Invoke-Git { param([string[]]$GitArgs) & git @GitArgs 2>&1 | Out-Null; if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed ($LASTEXITCODE)" } }

function New-ForkRepo {
  [CmdletBinding(SupportsShouldProcess)]
  param([string]$name, [bool]$dirty)
  $bareRepo = Join-Path $bare "$name.git"
  $work     = Join-Path $forks "repo-$name"
  if (-not $PSCmdlet.ShouldProcess($work, 'create a sandbox git repo + bare remote')) { return }
  Invoke-Git @('init', '--bare', '--initial-branch=main', $bareRepo)
  Invoke-Git @('init', '--initial-branch=main', $work)
  # Repo-local identity so commits don't depend on (or touch) the user's global git config.
  Invoke-Git @('-C', $work, 'config', 'user.name', 'iso')
  Invoke-Git @('-C', $work, 'config', 'user.email', 'iso@local')
  Invoke-Git @('-C', $work, 'config', 'commit.gpgsign', 'false')
  Write-Text (Join-Path $work 'README.md') "# repo-$name`nISO sandbox fork.`n"
  Invoke-Git @('-C', $work, 'add', '-A')
  Invoke-Git @('-C', $work, 'commit', '-m', 'iso: initial commit')
  Invoke-Git @('-C', $work, 'remote', 'add', 'origin', $bareRepo)
  Invoke-Git @('-C', $work, 'push', '-u', 'origin', 'main')
  if ($dirty) {
    Invoke-Git @('-C', $work, 'branch', 'feature/wip')
    Write-Text (Join-Path $work 'dirty.txt') "uncommitted local change`n"   # leaves the tree dirty
  }
}
New-ForkRepo 'alpha' $false
New-ForkRepo 'beta'  $true

# fork-sync.last.json — the payload the Forks tab actually renders (ForkStatus shape, src/lib/ipc.ts:
# ForkRepo fields). Without it the tab shows "Нет данных" even after a stub run (live find 2026-07-18).
# Paths point at the REAL sandbox repos so per-repo actions (status/plan) operate on real git.
$forkUpdater = Join-Path $Scripts 'fork-updater'
New-Dir $forkUpdater
$forkRepo = {
  param([string]$name, [bool]$dirty, [bool]$own)
  [ordered]@{
    Name = "repo-$name"; Path = (Join-Path $forks "repo-$name")
    upstream = if ($own) { $null } else { "https://github.com/upstream/$name" }
    fork = if ($own) { $null } else { "https://github.com/iso-user/$name" }
    forkOwnerRepo = if ($own) { $null } else { "iso-user/$name" }
    parentOwnerRepo = if ($own) { $null } else { "upstream/$name" }
    defaultBranch = 'main'; behindBy = if ($own) { 0 } else { 2 }; defaultAhead = 0
    ffSafe = -not $dirty; dirty = $dirty; untracked = $dirty; midOp = $false; opName = $null
    detached = $false; currentBranch = 'main'; isOwn = $own; rolesGuessed = $false
    wipLocal = $null; upstreamUpdated = (Get-Date).AddDays(-3).ToString('o')
    upstreamArchived = $false; upstreamDefaultBranch = 'main'
    branches = @(); Skipped = $null
  }
}
Write-Json (Join-Path $forkUpdater 'fork-sync.last.json') ([ordered]@{
  schemaVersion = 1; status = 'ok'; timestamp = (Get-Date).ToString('o')
  generatedAt = (Get-Date).ToString('o'); mode = 'check'; ghAvailable = $true; durationSec = 4
  summary = [ordered]@{ repos = 2; merged = 0; open = 0; conflict = 0; needHands = 1 }
  repos = @((& $forkRepo 'alpha' $false $false), (& $forkRepo 'beta' $true $true))
})

# ════════════════════════════════════════════════════════════════════════════
# LAYER 3 — bin\  (prepended to PATH; git/pwsh/node stay REAL)
# ════════════════════════════════════════════════════════════════════════════

# Shared interactive TUI fake: banner, stdin echo-loop, exit on "exit". Writes agent-status
# (busy at start, idle after each reply) when CASTELLYN_SESSION_ID + APPDATA are set.
$cliPs1 = @'
param([string]$Name = 'iso')
$ErrorActionPreference = 'SilentlyContinue'
$resumed = $false
foreach ($a in $args) { if ($a -eq '--resume') { $resumed = $true } }

function Write-AgentStatus([string]$state) {
  $sid = $env:CASTELLYN_SESSION_ID
  if (-not $sid -or -not $env:APPDATA) { return }
  if ($sid -notmatch '^[A-Za-z0-9]{1,32}$') { return }
  $dir = Join-Path $env:APPDATA 'castellyn\agent-status'
  if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $fp  = Join-Path $dir ($sid + '.json')
  $ts  = [int64]([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
  $payload = @{ state = $state; event = 'iso'; claudeSessionId = $sid; ts = $ts } | ConvertTo-Json -Compress
  $tmp = "$fp.tmp"
  [System.IO.File]::WriteAllText($tmp, $payload, [System.Text.UTF8Encoding]::new($false))
  [System.IO.File]::Move($tmp, $fp, $true)
}

if ($resumed) { Write-Host "[iso-$Name] resumed" } else { Write-Host "[iso-$Name] ready" }
Write-AgentStatus 'working'   # busy at start
while ($true) {
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) { break }                 # EOF -> session ends
  if ($line.Trim() -eq 'exit') { break }
  Write-Host "[iso-$Name] echo: $line"
  Write-AgentStatus 'idle'                        # idle after each reply
}
Write-AgentStatus 'ended'
'@
Write-Text (Join-Path $Bin 'iso-cli.ps1') $cliPs1

# .cmd shims resolve the sibling iso-cli.ps1 via %~dp0 (relocatable with -Root).
foreach ($tool in 'claude', 'codex', 'opencode') {
  Write-Text (Join-Path $Bin "$tool.cmd") "@echo off`r`npwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File `"%~dp0iso-cli.ps1`" -Name $tool %*`r`n"
}

# gh fake: `gh repo list --json ...` -> fixture array (GithubRepo shape); anything else -> exit 0.
$ghFixture = @(
  [ordered]@{ name = 'castellyn'; owner = [ordered]@{ login = 'iso-user' }; nameWithOwner = 'iso-user/castellyn'; isPrivate = $true;  isFork = $false; isArchived = $false; url = 'https://github.com/iso-user/castellyn'; updatedAt = (Get-Date -Format 'o'); description = 'ISO sandbox repo'; primaryLanguage = [ordered]@{ name = 'Rust' }; stargazerCount = 7 }
  [ordered]@{ name = 'some-fork';  owner = [ordered]@{ login = 'iso-user' }; nameWithOwner = 'iso-user/some-fork'; isPrivate = $false; isFork = $true;  isArchived = $false; url = 'https://github.com/iso-user/some-fork'; updatedAt = (Get-Date -Format 'o'); description = 'ISO sandbox fork'; primaryLanguage = [ordered]@{ name = 'TypeScript' }; stargazerCount = 0 }
) | ConvertTo-Json -Depth 8
Write-Text (Join-Path $Bin 'iso-gh.ps1') ('Write-Output @''' + "`n" + $ghFixture + "`n" + "'@`n")
Write-Text (Join-Path $Bin 'gh.cmd') "@echo off`r`nif /I `"%~1`"==`"repo`" if /I `"%~2`"==`"list`" (`r`n  pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File `"%~dp0iso-gh.ps1`"`r`n  exit /b 0`r`n)`r`nexit /b 0`r`n"

# ════════════════════════════════════════════════════════════════════════════
# SELF-CHECK — the generator validates its own world and fails exit 1 if wrong.
# ════════════════════════════════════════════════════════════════════════════
Write-Host "▶  Самопроверка…" -ForegroundColor Cyan
$fail = @()

# manifest parses as JSON
try { $null = Get-Content -Raw -LiteralPath $manifestDst | ConvertFrom-Json } catch { $fail += "manifest not valid JSON: $_" }

# one stub executes and writes a valid envelope with the required fields
try {
  $probe = $manifest.components[0]
  $probeScript = Resolve-WorldPath $probe.scriptRel
  $probeJson   = Resolve-WorldPath $probe.lastJsonRel
  if (Test-Path -LiteralPath $probeJson) { Remove-Item -LiteralPath $probeJson -Force }
  $env:ISO_OUTCOME = 'ok'
  & pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File $probeScript -Check | Out-Null
  Remove-Item Env:\ISO_OUTCOME -ErrorAction SilentlyContinue
  if (-not (Test-Path -LiteralPath $probeJson)) { throw "envelope not written: $probeJson" }
  $env = Get-Content -Raw -LiteralPath $probeJson | ConvertFrom-Json
  foreach ($k in 'schemaVersion', 'component', 'status', 'timestamp') {
    if ($null -eq $env.$k) { throw "envelope missing '$k'" }
  }
} catch { $fail += "stub/envelope: $_" }

# forks: alpha clean, beta dirty
try {
  $sa = & git -C (Join-Path $forks 'repo-alpha') status --porcelain
  if ($sa) { $fail += "repo-alpha not clean: $sa" }
  $sb = & git -C (Join-Path $forks 'repo-beta') status --porcelain
  if (-not $sb) { $fail += "repo-beta expected dirty but clean" }
} catch { $fail += "git status: $_" }

# claude.cmd answers an echo round and exits
try {
  $out = "hello`nexit`n" | & (Join-Path $Bin 'claude.cmd') 2>&1 | Out-String
  if ($out -notmatch '\[iso-claude\] echo: hello') { $fail += "claude.cmd echo round failed. Got: $out" }
} catch { $fail += "claude.cmd: $_" }

# Control-action stubs: each runs blank -> exit 0. USERPROFILE points at the world so the ones that
# touch profile homes (Deploy-Mcp / Install) write INTO the sandbox, never the real ~/.claude-*.
$savedUP = $env:USERPROFILE
try {
  $env:USERPROFILE = $HomeDir
  foreach ($nm in $controlStubNames) {
    & pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Profiles $nm) *> $null
    if ($LASTEXITCODE -ne 0) { $fail += "$nm exited $LASTEXITCODE" }
  }
  & pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File $stackProcsPath -Ports '1420,8787' *> $null
  if ($LASTEXITCODE -ne 0) { $fail += "Stack-Procs.ps1 exited $LASTEXITCODE" }
  # Deploy-Mcp really wrote the canonical servers into a profile (cc2 started empty).
  try {
    $cc2 = Get-Content -LiteralPath (Join-Path $HomeDir '.claude-cc2\.claude.json') -Raw | ConvertFrom-Json
    if (-not $cc2.mcpServers.'iso-fetch') { $fail += 'Deploy-Mcp did not write iso-fetch into .claude-cc2' }
  } catch { $fail += "Deploy-Mcp probe: $_" }
} finally {
  $env:USERPROFILE = $savedUP
}

# Restore every fixture the self-check mutated so the world the harness sees is the pristine initial
# state (cc2 empty again, drifted links, disabled/not-created schedules) — the branches the clicker
# must exercise. Backups snapshots are left (extra snapshots are harmless + realistic).
Write-Json (Join-Path $Profiles 'profiles.last.json')  (& $profilesStatus)
Write-Json (Join-Path $Profiles 'schedules.last.json') (& $schedulesFixture)
Write-Json (Join-Path $Profiles 'links.last.json')     (& $linksFixture)
Set-ProfileMcp '.claude-cc1' (& $cc1McpSeed)
Set-ProfileMcp '.claude-cc2' ([ordered]@{})

if ($fail.Count) {
  Write-Host ''
  Write-Host "✗ Самопроверка провалена:" -ForegroundColor Red
  $fail | ForEach-Object { Write-Host "    - $_" -ForegroundColor Red }
  exit 1
}
Write-Host "    ✓ манифест / стаб-конверт / форки / claude.cmd — ОК" -ForegroundColor Green

# ════════════════════════════════════════════════════════════════════════════
# OUTPUT — layer table + env-export line for the harness.
# ════════════════════════════════════════════════════════════════════════════
Write-Host ''
Write-Host '════════════════════════════════════════════════════════════' -ForegroundColor Green
Write-Host '✓ Мир-песочница готов' -ForegroundColor Green
Write-Host '════════════════════════════════════════════════════════════' -ForegroundColor Green
@(
  [pscustomobject]@{ Слой = 'home (USERPROFILE)'; Путь = $HomeDir }
  [pscustomobject]@{ Слой = 'scripts (SCRIPTS_ROOT)'; Путь = $Scripts }
  [pscustomobject]@{ Слой = 'settings (CASTELLYN_SETTINGS_DIR)'; Путь = $Settings }
  [pscustomobject]@{ Слой = 'bin (PATH prepend)'; Путь = $Bin }
) | Format-Table -AutoSize | Out-Host
$ctrlCount = $controlStubNames.Count + 2   # + Stack-Procs + Get-ProfilesStatus
Write-Host "  Компонентов-заглушек: $stubCount  ·  управляющих скриптов: $ctrlCount  ·  форки: repo-alpha (clean), repo-beta (dirty)" -ForegroundColor DarkGray
Write-Host ''
Write-Host 'Env-экспорты для харнесса (iso-test.ps1 -World выставляет их процессу):' -ForegroundColor DarkGray
Write-Host "  USERPROFILE=$HomeDir" -ForegroundColor Gray
Write-Host "  SCRIPTS_ROOT=$Scripts" -ForegroundColor Gray
Write-Host "  CASTELLYN_SETTINGS_DIR=$Settings" -ForegroundColor Gray
Write-Host "  PATH=$Bin;`$env:PATH" -ForegroundColor Gray
Write-Host ''
