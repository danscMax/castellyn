# Compatibility import for legacy callers. The maintained engine lives in Scripts/fork-updater.
$scriptsRoot = if ($env:SCRIPTS_ROOT) { $env:SCRIPTS_ROOT } else { Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) }
$canonical = Join-Path $scriptsRoot 'fork-updater/ForkSync.psm1'
if (-not (Test-Path -LiteralPath $canonical)) { throw "Canonical fork-updater is missing: $canonical" }
Import-Module $canonical -Force
Export-ModuleMember -Function *-*
