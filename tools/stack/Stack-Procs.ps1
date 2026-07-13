<#
.SYNOPSIS
    For each given TCP port, report the listening process PID and its uptime in seconds.
    One process snapshot for the whole stack (AgentHub merges it onto the service cards), so the
    UI can show "PID 1234 · 2h" without a separate per-service probe.

.OUTPUTS
    A single compact JSON line: [{ "port": 13001, "pid": 4710, "uptimeSec": 8123 }, ...]
    Ports with no listener (or an unreadable process) are simply omitted.
#>
# Ports arrive as a comma-separated string ("13001,3456"): when invoked via `-File`, PowerShell does
# NOT bind a comma list to [int[]], so take a string and split it ourselves.
param([string] $Ports)

$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$portList = @()
if ($Ports) { $portList = $Ports -split '[,\s]+' | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [int]$_ } }

$now = Get-Date
$result = foreach ($p in $portList) {
    $conn = Get-NetTCPConnection -State Listen -LocalPort $p -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $conn) { continue }
    $proc = Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue
    # StartTime can throw for protected processes; treat as unknown (0) rather than failing.
    $uptime = 0
    if ($proc -and $proc.StartTime) { $uptime = [int][math]::Max(0, ($now - $proc.StartTime).TotalSeconds) }
    [pscustomobject]@{ port = $p; pid = [int]$conn.OwningProcess; uptimeSec = $uptime }
}

# Force an array shape even for 0/1 results so the consumer always parses a JSON array.
ConvertTo-Json -Compress -InputObject @($result)
