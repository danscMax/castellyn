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

function New-Dir([string]$p) { if (-not (Test-Path -LiteralPath $p)) { New-Item -ItemType Directory -Path $p -Force | Out-Null } }

# UTF-8 WITHOUT BOM — Castellyn's own writers never emit a BOM (CLAUDE.md convention).
function Write-Text([string]$path, [string]$text) {
  New-Dir (Split-Path -Parent $path)
  [System.IO.File]::WriteAllText($path, $text, [System.Text.UTF8Encoding]::new($false))
}
function Write-Json([string]$path, $obj) { Write-Text $path ($obj | ConvertTo-Json -Depth 12) }

# ── -Wipe ────────────────────────────────────────────────────────────────────
if ($Wipe) {
  if (Test-Path -LiteralPath $World) {
    Remove-Item -LiteralPath $World -Recurse -Force
    Write-Host "🧹 Мир снесён: $World" -ForegroundColor Yellow
  } else {
    Write-Host "   (мир не существовал: $World)" -ForegroundColor DarkGray
  }
  return
}

# Deterministic rebuild: nuke any prior world first so a re-run reproduces the same tree.
if (Test-Path -LiteralPath $World) { Remove-Item -LiteralPath $World -Recurse -Force }
New-Dir $World; New-Dir $HomeDir; New-Dir $Scripts; New-Dir $Bin

Write-Host "▶  Строю мир: $World" -ForegroundColor Cyan

# ════════════════════════════════════════════════════════════════════════════
# LAYER 1 — home\  (becomes USERPROFILE)
# ════════════════════════════════════════════════════════════════════════════

# One profile home: settings.json (env block, dummy token), .credentials.json (expired), hooks\.
function New-ProfileHome([string]$dirName, [int]$n, [hashtable]$env) {
  $dir = Join-Path $HomeDir $dirName
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

# Canonical shared MCP servers (2 fakes) — read_mcp surfaces name+command from mcpServers.
Write-Json (Join-Path $Profiles 'config\.mcp.json') ([ordered]@{
  mcpServers = [ordered]@{
    'iso-echo'  = [ordered]@{ command = 'node'; args = @('iso-echo-server.js') }
    'iso-fetch' = [ordered]@{ command = 'npx';  args = @('-y', 'iso-fetch-mcp') }
  }
})

# Schedules fixture — covers every ScheduleTab branch + the watcher's failed-transition path:
# ok / failed / disabled / not-created. lastResult 0=ok, non-zero=fail (schedules_watch task_failed).
$now = Get-Date
Write-Json (Join-Path $Profiles 'schedules.last.json') ([ordered]@{
  schemaVersion = 1
  timestamp     = (Get-Date -Format 'o')
  tasks = @(
    [ordered]@{ id = 'update-all'; label = 'Обновить всё (ночью)'; exists = $true;  enabled = $true;  time = '03:00'; defaultTime = '03:00'; nextRun = $now.AddHours(9).ToString('o');  lastRun = $now.AddHours(-15).ToString('o'); lastResult = 0 }
    [ordered]@{ id = 'forks-sync'; label = 'Синхронизация форков';  exists = $true;  enabled = $true;  time = '04:30'; defaultTime = '04:30'; nextRun = $now.AddHours(10).ToString('o'); lastRun = $now.AddHours(-14).ToString('o'); lastResult = 1 }
    [ordered]@{ id = 'plugins';    label = 'Обновление плагинов';   exists = $true;  enabled = $false; time = '05:00'; defaultTime = '05:00'; nextRun = $null;                            lastRun = $null;                             lastResult = $null }
    [ordered]@{ id = 'cargo';      label = 'Cargo-бинарники';        exists = $false; enabled = $false; time = $null;   defaultTime = '06:00'; nextRun = $null;                            lastRun = $null;                             lastResult = $null }
  )
})

# 2d. Forks — real git repos with a local bare remote. alpha clean, beta dirty + a branch.
$forks = Join-Path $Scripts 'forks'
$bare  = Join-Path $forks '.bare'
New-Dir $bare
New-Dir (Join-Path $Scripts 'External')   # present but empty

function Invoke-Git { param([string[]]$GitArgs) & git @GitArgs 2>&1 | Out-Null; if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed ($LASTEXITCODE)" } }

function New-ForkRepo([string]$name, [bool]$dirty) {
  $bareRepo = Join-Path $bare "$name.git"
  $work     = Join-Path $forks "repo-$name"
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
Write-Host "  Компонентов-заглушек: $stubCount  ·  форки: repo-alpha (clean), repo-beta (dirty)" -ForegroundColor DarkGray
Write-Host ''
Write-Host 'Env-экспорты для харнесса (iso-test.ps1 -World выставляет их процессу):' -ForegroundColor DarkGray
Write-Host "  USERPROFILE=$HomeDir" -ForegroundColor Gray
Write-Host "  SCRIPTS_ROOT=$Scripts" -ForegroundColor Gray
Write-Host "  CASTELLYN_SETTINGS_DIR=$Settings" -ForegroundColor Gray
Write-Host "  PATH=$Bin;`$env:PATH" -ForegroundColor Gray
Write-Host ''
