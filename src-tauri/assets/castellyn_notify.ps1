# castellyn-notify-version: 2
# Codex `notify` program -> Castellyn agent-status file.
#
# Codex has no lifecycle hook we can wire without editing the user's ~/.codex/config.toml, but it
# does accept a `notify` program per invocation (`codex -c 'notify=[...]'`), which it runs once each
# time a turn finishes. Castellyn passes that flag when it spawns a codex pane, so this script is the
# ONLY authoritative "the agent stopped" signal codex has; without it the pane can merely guess from
# how long the PTY has been quiet, and never dares claim a turn is done.
#
# Written by Castellyn into %APPDATA%\castellyn\hooks; manual edits are overwritten on updates.
# Fail-open: never fail the turn, never write anything for a codex started outside Castellyn.

# A codex run outside Castellyn has no pane id -> no-op. The id is `s` + 15 hex (gen_session_id).
$sid = $env:CASTELLYN_SESSION_ID
if (-not $sid -or $sid.Length -gt 32 -or $sid -notmatch '^[A-Za-z0-9]+$') { exit 0 }

try {
    # Codex appends the event JSON as the last argument. PowerShell re-parses argv and can strip the
    # JSON's quotes, so match the event name as a substring instead of parsing it. An older codex that
    # passes nothing still means "turn complete" — that is the only event legacy notify ever emits.
    $raw = $args -join ' '
    if ($raw -and $raw -notlike '*agent-turn-complete*') { exit 0 }

    $base = $env:APPDATA
    if (-not $base) { exit 0 }
    $dir = Join-Path $base 'castellyn\agent-status'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null

    $fp = Join-Path $dir "$sid.json"
    $ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $payload = '{"state":"idle","event":"agent-turn-complete","ts":' + $ts + '}'

    # Same temp+rename the Rust writer uses: the poll thread must never read a half-written file.
    $tmp = "$fp.tmp"
    [System.IO.File]::WriteAllText($tmp, $payload, (New-Object System.Text.UTF8Encoding $false))
    [System.IO.File]::Move($tmp, $fp, $true)
} catch {
    # A broken notifier must never break the agent: codex runs this program synchronously at the end
    # of a turn, so anything thrown here would surface as a turn-level failure. The cost of losing
    # one status write is a pane that self-heals from PTY quiet; the cost of throwing is the turn.
    Write-Verbose "castellyn-notify: status write skipped: $_"
}
exit 0
