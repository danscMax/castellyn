<#
.SYNOPSIS
    Roll out the canonical ScriptKit.ps1 to every vendored copy under a root.

.DESCRIPTION
    CANON = <this folder>\ScriptKit.ps1 (Castellyn\tools).
    Scans -Root (default E:\Scripts) for every other ScriptKit.ps1, compares each
    against the canon by SHA-256 and by the $script:SK_Version drift marker, and:
      -Check  (default)  report drift only, change nothing.
      -Apply             overwrite every drifted copy from canon (UTF-8, no BOM).

    Read-only unless -Apply is passed.

.EXAMPLE
    pwsh -File Sync-ScriptKit.ps1            # show drift
    pwsh -File Sync-ScriptKit.ps1 -Apply     # roll out canon everywhere
#>
param(
    [string]$Root = 'E:\Scripts',
    [switch]$Apply,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }

$canon = Join-Path $PSScriptRoot 'ScriptKit.ps1'
if (-not (Test-Path -LiteralPath $canon)) {
    Write-Host "Canon not found: $canon" -ForegroundColor Red
    exit 1
}

# Dot-source canon: gives us $script:SK_Version, Get-FileHashSHA256, Write-* UI.
. $canon
$canonVersion = $script:SK_Version
$canonHash    = Get-FileHashSHA256 -Path $canon
$canonResolved = (Resolve-Path -LiteralPath $canon).Path

function Get-SkVersion {
    param([string]$Path)
    $m = Select-String -LiteralPath $Path -Pattern '\$script:SK_Version\s*=\s*(\d+)' -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($m) { return [int]$m.Matches[0].Groups[1].Value }
    return 1   # no marker -> legacy v1
}

Write-Banner "Sync-ScriptKit" ("canon v{0} -- {1}" -f $canonVersion, $canonResolved) 'Cyan'

$copies = Get-ChildItem -LiteralPath $Root -Recurse -Filter 'ScriptKit.ps1' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -ne $canonResolved } |
    Sort-Object FullName

if (-not $copies) {
    Write-Info "No vendored copies found under $Root."
    exit 0
}

# -Check forces report-only even if -Apply is also given (explicit drift check).
$doApply = $Apply -and -not $Check
$inSync = 0; $drift = 0; $applied = 0; $failed = 0
foreach ($c in $copies) {
    # Hash inside try/catch: under ErrorActionPreference='Stop' a single locked/unreadable vendored
    # copy would otherwise halt the whole sync — count it as failed and keep going.
    try {
        $hash = Get-FileHashSHA256 -Path $c.FullName
    } catch {
        $failed++
        Write-Status ("хэш не прочитан: {0} — {1}" -f $c.FullName, $_.Exception.Message) 'FAIL'
        continue
    }
    if ($hash -eq $canonHash) {
        $inSync++
        Write-Status ("v{0}  {1}" -f $canonVersion, $c.FullName) 'OK'
        continue
    }
    $drift++
    $ver = Get-SkVersion -Path $c.FullName
    Write-Status ("v{0} -> v{1}  {2}" -f $ver, $canonVersion, $c.FullName) 'WARN'
    if ($doApply) {
        try {
            # Back up the existing copy first so an unexpected rollout is reversible.
            Copy-Item -LiteralPath $c.FullName -Destination ("{0}.bak" -f $c.FullName) -Force
            Copy-Item -LiteralPath $canonResolved -Destination $c.FullName -Force
            $applied++
            Write-Ok ("rolled out -> {0}" -f $c.FullName)
        } catch {
            $failed++
            Write-Fail ("copy failed {0}: {1}" -f $c.FullName, $_.Exception.Message)
        }
    }
}

Write-Host ""
if ($doApply) {
    Write-Host ("  Done. {0} in sync, {1} drifted, {2} rolled out, {3} failed." -f $inSync, $drift, $applied, $failed) -ForegroundColor White
    if ($failed -gt 0) { exit 1 }
} else {
    Write-Host ("  {0} in sync, {1} drifted. Run with -Apply to roll out canon." -f $inSync, $drift) -ForegroundColor White
}
exit 0
