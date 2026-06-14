# ============================================================================
# AgentHub — сборка релиза + ярлык на рабочий стол
# ============================================================================
# Простой однокомандный билд (по мотивам Sweet Whisper\build_all.ps1, но без
# подписи/Vulkan/NSIS — у этого приложения нет нативных зависимостей):
#   1. pre-flight (node / npm / cargo на PATH)
#   2. npm install (если нет node_modules)
#   3. svelte-check (типовой гейт; пропуск -SkipCheck)
#   4. tauri build  (standalone exe; -Bundle добавит установщики NSIS/MSI)
#   5. (пере)создаёт ярлык «AgentHub» на рабочем столе
#
# Использование:
#   .\build_all.ps1                 # exe + ярлык
#   .\build_all.ps1 -Bundle         # + установщики (bundle\)
#   .\build_all.ps1 -SkipCheck      # без svelte-check
#   .\build_all.ps1 -NoShortcut     # не трогать ярлык
#   .\build_all.ps1 -NoOpen         # не открывать Проводник
# ============================================================================
param(
    [switch]$Bundle,
    [switch]$SkipCheck,
    [switch]$NoShortcut,
    [switch]$NoOpen
)

try { chcp 65001 | Out-Null } catch { }
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'

$root = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Definition }
$kit = Join-Path $root 'tools\ScriptKit.ps1'
if (Test-Path -LiteralPath $kit) { . $kit }   # Write-Banner / Show-Notification (optional)
$totalStart = Get-Date

if (Get-Command Write-Banner -ErrorAction SilentlyContinue) {
    Write-Banner 'AgentHub — сборка' 'release exe + ярлык на рабочий стол'
} else {
    Write-Host '=== AgentHub — сборка ===' -ForegroundColor Cyan
}

# 1. Pre-flight
foreach ($c in 'node', 'npm', 'cargo') {
    if (-not (Get-Command $c -ErrorAction SilentlyContinue)) {
        Write-Host "  [FAIL] '$c' не найден на PATH." -ForegroundColor Red
        exit 1
    }
}
Write-Host '  [OK] node / npm / cargo на PATH' -ForegroundColor Green

Set-Location -LiteralPath $root

# 2. Зависимости
if (-not (Test-Path -LiteralPath (Join-Path $root 'node_modules'))) {
    Write-Host '  npm install...' -ForegroundColor Cyan
    npm install
    if ($LASTEXITCODE -ne 0) { Write-Host '  npm install не удался.' -ForegroundColor Red; exit 1 }
}

# 3. Типовой гейт
if (-not $SkipCheck) {
    Write-Host '  svelte-check...' -ForegroundColor Cyan
    npm run check
    if ($LASTEXITCODE -ne 0) {
        Write-Host '  svelte-check нашёл ошибки — сборка прервана (используй -SkipCheck, чтобы пропустить).' -ForegroundColor Red
        exit 1
    }
    Write-Host '  [OK] svelte-check без ошибок' -ForegroundColor Green
}

# 4. Сборка
$bundleLabel = if ($Bundle) { '(установщики + exe)' } else { '(standalone exe)' }
Write-Host "  tauri build $bundleLabel..." -ForegroundColor Cyan
$buildArgs = @('run', 'tauri', 'build')
if (-not $Bundle) { $buildArgs += @('--', '--no-bundle') }
& npm @buildArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host '  tauri build не удался.' -ForegroundColor Red
    if (Get-Command Show-Notification -ErrorAction SilentlyContinue) {
        Show-Notification -Title 'AgentHub: сборка ПРОВАЛЕНА' -Body 'tauri build failed — см. терминал.' -IsError
    }
    exit 1
}

$exe = Join-Path $root 'src-tauri\target\release\agenthub.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    Write-Host "  [FAIL] exe не найден: $exe" -ForegroundColor Red
    exit 1
}

# 5. Ярлык на рабочий стол
if (-not $NoShortcut) {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $lnk = Join-Path $desktop 'AgentHub.lnk'
    $ws = New-Object -ComObject WScript.Shell
    $sc = $ws.CreateShortcut($lnk)
    $sc.TargetPath = $exe
    $sc.WorkingDirectory = (Split-Path -Parent $exe)
    $sc.IconLocation = "$exe,0"
    $sc.Description = 'AgentHub — центр управления ИИ-агентами'
    $sc.Save()
    Write-Host "  [OK] Ярлык: $lnk" -ForegroundColor Green
}

# Итог
$dur = (Get-Date) - $totalStart
$time = '{0}:{1:D2}' -f [math]::Floor($dur.TotalMinutes), $dur.Seconds
$size = '{0:0.0} MB' -f ((Get-Item -LiteralPath $exe).Length / 1MB)
$cargo = Get-Content -LiteralPath (Join-Path $root 'src-tauri\Cargo.toml') -Raw -ErrorAction SilentlyContinue
$version = if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') { $matches[1] } else { '?' }

Write-Host ''
Write-Host "  ГОТОВО — v$version — $time — exe $size" -ForegroundColor Green
Write-Host "  $exe" -ForegroundColor DarkGray
if ($Bundle) { Write-Host '  Установщики: src-tauri\target\release\bundle\' -ForegroundColor DarkGray }

if (Get-Command Show-Notification -ErrorAction SilentlyContinue) {
    Show-Notification -Title 'AgentHub собран' -Body "v$version — $size — $time" -IconPath $exe
}

if (-not $NoOpen) {
    Start-Process explorer.exe -ArgumentList "/select,`"$exe`""
}
