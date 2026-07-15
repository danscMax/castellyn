<#
.SYNOPSIS
  Spin up an ISOLATED Castellyn dev instance for full UI testing (safe click-through).

.DESCRIPTION
  Runs Castellyn with its OWN %APPDATA%/%LOCALAPPDATA% (so its config.json, keyring re-home,
  localStorage and window state are isolated in a scratch dir — a test run cannot corrupt the
  real Castellyn config) and exposes a CDP endpoint on a dedicated port so a Playwright clicker
  can drive it without touching the user's real browser.

  ISOLATION BOUNDARY (read this): only Castellyn's OWN config lives under %APPDATA%\castellyn.
  The profiles it manages (~/.claude*, ~/.ssh/config, the forks under SCRIPTS_ROOT, providers,
  freeapi.db analytics) live in the REAL filesystem — Castellyn is a control center for the real
  system, so a test instance still SEES real data. Anything that WRITES the real system
  (maintenance scripts, git actions, registry autostart, launching paid Claude/codex/opencode
  sessions) must stay off-limits during a test run. See the safety rules in the /max skill.

  PORT CONSTRAINT: vite is pinned to 1420 (strictPort) and the debug exe loads devUrl=1420, so the
  isolated instance and your own `npm run tauri dev` CANNOT run at the same time. Stop your own dev
  first. Isolation is in the DATA + the CDP port, not the vite port.

.PARAMETER Stop
  Tear down: kill castellyn.exe and free the vite port. Leaves the scratch profile in place
  (re-usable) unless -Fresh is also given.

.PARAMETER Fresh
  Wipe the isolated profile before starting (true first-run: onboarding wizard appears).
  With -Stop, also deletes the scratch profile.

.PARAMETER Build
  `cargo build` the debug exe first (needed after Rust changes; the frontend hot-reloads via vite).

.PARAMETER CdpPort
  Remote-debugging port for the isolated WebView2 (default 9223; the real dev instance uses 9222).

.PARAMETER Repo
  Castellyn repo root (default: the parent of this script's tools/ dir).

.EXAMPLE
  pwsh -File tools/iso-test.ps1              # start isolated instance, reuse scratch profile
  pwsh -File tools/iso-test.ps1 -Fresh       # start with a clean profile (onboarding)
  pwsh -File tools/iso-test.ps1 -Build       # rebuild the exe, then start
  pwsh -File tools/iso-test.ps1 -Stop        # tear down
#>
param(
  [switch]$Stop,
  [switch]$Fresh,
  [switch]$Build,
  [int]$CdpPort = 9223,
  [string]$Repo = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$VitePort = 1420  # pinned in vite.config.js (strictPort) + tauri.conf.json devUrl
$IsoRoot  = Join-Path $env:TEMP 'castellyn-iso'
$IsoApp   = Join-Path $IsoRoot 'Roaming'
$IsoLocal = Join-Path $IsoRoot 'Local'
$Exe      = Join-Path $Repo 'src-tauri\target\debug\castellyn.exe'

function Test-PortBusy([int]$Port) {
  return [bool](Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue)
}

function Stop-Iso {
  Write-Host '⏹  Останавливаю изолированный экземпляр…' -ForegroundColor Yellow
  # Only kill the ISOLATED (debug-build) exe, matched by its full path — never the user's real
  # release/dev Castellyn that happens to share the process name.
  $iso = @(Get-Process castellyn -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq $Exe })
  if ($iso.Count) {
    $iso | Stop-Process -Force -ErrorAction SilentlyContinue
    # Free the vite port only because WE had an iso instance up — otherwise this would nuke a real
    # `npm run tauri dev`'s vite (port 1420 is shared; the two can never run at once).
    Get-NetTCPConnection -State Listen -LocalPort $VitePort -ErrorAction SilentlyContinue |
      Select-Object -ExpandProperty OwningProcess -Unique |
      ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
  } else {
    Write-Host '   (изолированный debug-экземпляр не запущен — ничего не тронуто)' -ForegroundColor DarkGray
  }
  if ($Fresh -and (Test-Path $IsoRoot)) {
    Remove-Item -LiteralPath $IsoRoot -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "🧹 Профиль удалён: $IsoRoot" -ForegroundColor Yellow
  }
  Write-Host '✓ Остановлено.' -ForegroundColor Green
}

if ($Stop) { Stop-Iso; return }

# ── Preconditions ──────────────────────────────────────────────────────────────
if (Get-Process castellyn -ErrorAction SilentlyContinue) {
  throw "castellyn.exe уже запущен. Останови свой dev-экземпляр (или: pwsh -File tools/iso-test.ps1 -Stop) — vite 1420 и devUrl общие, два dev-экземпляра одновременно невозможны."
}
if (Test-PortBusy $VitePort) {
  throw "Порт $VitePort занят (вероятно, твой `npm run tauri dev`). Изолированному экземпляру нужен тот же порт — останови рабочий dev сначала."
}
if (Test-PortBusy $CdpPort) {
  throw "CDP-порт $CdpPort занят. Освободи его или задай другой: -CdpPort 9224."
}

if ($Build) {
  Write-Host '🔨 cargo build (debug)…' -ForegroundColor Cyan
  Push-Location $Repo
  # `& cargo` is a native call — it sets $LASTEXITCODE but does NOT throw, so a compile error would
  # fall through and launch a STALE exe below. Fail loudly instead of testing outdated Rust.
  try {
    & cargo build --manifest-path 'src-tauri\Cargo.toml'
    if ($LASTEXITCODE -ne 0) { throw "cargo build упал (код $LASTEXITCODE) — не запускаю устаревший exe." }
  } finally { Pop-Location }
}
if (-not (Test-Path $Exe)) {
  throw "Не найден $Exe — собери сперва: pwsh -File tools/iso-test.ps1 -Build"
}

# ── Isolated profile ───────────────────────────────────────────────────────────
if ($Fresh -and (Test-Path $IsoRoot)) {
  Remove-Item -LiteralPath $IsoRoot -Recurse -Force
  Write-Host '🧹 Свежий профиль (онбординг появится).' -ForegroundColor Yellow
}
New-Item -ItemType Directory -Force -Path $IsoApp, $IsoLocal | Out-Null

# ── 1) vite FIRST (fixes the chrome-error-then-blank webview: the exe used to load 1420 before
#       vite was ready). Only once vite answers do we start the exe, which then loads on first try.
Write-Host "▶  Старт vite на :$VitePort …" -ForegroundColor Cyan
# npm is npm.cmd on Windows — Start-Process needs the resolved path (or a cmd shim). Resolve it,
# falling back to `cmd /c npm` if Get-Command can't (e.g. a shell function shadow).
$npm = (Get-Command npm.cmd -ErrorAction SilentlyContinue)?.Source
if ($npm) {
  $vite = Start-Process -FilePath $npm -ArgumentList 'run','dev' -WorkingDirectory $Repo -PassThru -WindowStyle Hidden
} else {
  $vite = Start-Process -FilePath $env:ComSpec -ArgumentList '/c','npm','run','dev' -WorkingDirectory $Repo -PassThru -WindowStyle Hidden
}
$deadline = (Get-Date).AddSeconds(90)
while (-not (Test-PortBusy $VitePort)) {
  if ((Get-Date) -gt $deadline) { throw 'vite не поднялся за 90с.' }
  Start-Sleep -Milliseconds 500
}
Start-Sleep -Seconds 1  # let SvelteKit finish its first optimize pass

# ── 2) the isolated exe (own APPDATA/LOCALAPPDATA + a dedicated CDP port) ────────
Write-Host "▶  Старт Castellyn (CDP :$CdpPort, изолированный конфиг) …" -ForegroundColor Cyan
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.UseShellExecute = $false
$psi.EnvironmentVariables['APPDATA'] = $IsoApp
$psi.EnvironmentVariables['LOCALAPPDATA'] = $IsoLocal
$psi.EnvironmentVariables['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = "--remote-debugging-port=$CdpPort"
$proc = [System.Diagnostics.Process]::Start($psi)

# ── 3) wait for the CDP endpoint to answer ──────────────────────────────────────
$cdpUrl = "http://127.0.0.1:$CdpPort/json/version"
$deadline = (Get-Date).AddSeconds(60)
while ($true) {
  try { Invoke-RestMethod -Uri $cdpUrl -TimeoutSec 2 | Out-Null; break } catch {}
  if ((Get-Date) -gt $deadline) { throw "CDP не ответил за 60с на $cdpUrl" }
  Start-Sleep -Milliseconds 500
}

Write-Host ''
Write-Host '════════════════════════════════════════════════════════════' -ForegroundColor Green
Write-Host '✓ Изолированный Castellyn готов' -ForegroundColor Green
Write-Host "  CDP:        http://127.0.0.1:$CdpPort" -ForegroundColor Green
Write-Host "  vite:       http://localhost:$VitePort" -ForegroundColor Green
Write-Host "  Конфиг:     $IsoApp\castellyn  (изолирован)" -ForegroundColor Green
Write-Host "  exe PID:    $($proc.Id)  ·  vite PID: $($vite.Id)" -ForegroundColor Green
Write-Host '  Стоп:       pwsh -File tools/iso-test.ps1 -Stop' -ForegroundColor Green
Write-Host '════════════════════════════════════════════════════════════' -ForegroundColor Green
Write-Host ''
Write-Host 'Playwright-рецепт (окно уже грузит devUrl; при chrome-error сделай page.goto):' -ForegroundColor DarkGray
Write-Host "  const b = await chromium.connectOverCDP('http://127.0.0.1:$CdpPort');" -ForegroundColor DarkGray
Write-Host "  const page = b.contexts().flatMap(c => c.pages())[0];" -ForegroundColor DarkGray
Write-Host "  if (!page.url().includes('localhost:$VitePort')) await page.goto('http://localhost:$VitePort/');" -ForegroundColor DarkGray
