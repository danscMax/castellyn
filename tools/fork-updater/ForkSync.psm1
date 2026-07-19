<#
.SYNOPSIS
  fork-sync core module — read-only multi-fork status reporter (Phase 1).

  All logic lives here so it is unit-testable (Pester) without running the
  interactive entry script. The entry point is `update-forks.ps1`, which just
  imports this module and calls Invoke-ForkSync.

  Phase 1 is READ-ONLY: the only network/IO side effect is `git fetch`
  (updates remote-tracking refs only) and read-only `gh` queries. It never
  changes a working tree, local branch, or pushes anything.

  Design notes:
   - GitHub PR state is the source of truth for "merged" (patch-id / git cherry
     is blind to squash- and rebase-merges). `git cherry` is kept only as a hint.
   - All local git analysis runs with `-c core.autocrlf=false` so Windows
     line-ending differences don't produce false "dirty"/false conflicts.
   - Conflict prediction uses `git merge-tree --write-tree --messages` (git>=2.38):
     a merge-based proxy for a future rebase conflict (honest heuristic).
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ─────────────────────────────────────────────────────────────────────────────
# region  Shared console UI / helpers (vendored ScriptKit — canonical source)
# ─────────────────────────────────────────────────────────────────────────────
# Dot-source the shared kit: box glyphs ($script:SK_*), Write-Banner/Status/Log,
# Show-Notification (with the success-icon fix), Invoke-TimedCommand[WithSpinner],
# Stop-NamedProcess, Get-FileHashSHA256, Get-AppVersion. These become module
# functions and are re-exported by `Export-ModuleMember -Function *-*` (harmless).
# Prefer a sibling ScriptKit.ps1 (standalone external layout); fall back to the parent
# folder's canonical copy (vendored under Castellyn\tools\fork-updater → Castellyn\tools).
$skPath = Join-Path $PSScriptRoot 'ScriptKit.ps1'
if (-not (Test-Path -LiteralPath $skPath)) {
    $skPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'ScriptKit.ps1'
}
. $skPath

# fork-sync-specific section header (titled box with an optional dimmed note).
# The kit has no titled-section helper, so this stays local. fork-sync renders
# 64-col boxes (kit Write-Banner is called with -Width 64 to match).
function Write-Section {
    param([string]$Text, [string]$Note = '', [string]$Color = 'Yellow')
    $bar = $script:SK_H * 64
    Write-Host ''; Write-Host "  $($script:SK_TL)$bar$($script:SK_TR)" -ForegroundColor $Color
    Write-Host "  $($script:SK_V)  $Text" -ForegroundColor $Color
    if ($Note) { Write-Host "  $($script:SK_V)  $Note" -ForegroundColor DarkGray }
    Write-Host "  $($script:SK_BL)$bar$($script:SK_BR)" -ForegroundColor $Color
}
# endregion

# ─────────────────────────────────────────────────────────────────────────────
# region  Process invocation
# ─────────────────────────────────────────────────────────────────────────────

# Fast, LOCAL git call — no timeout (local ops don't hang). Returns
# @{ Ok; Code; Out } where Out is trimmed stdout+stderr.
function Invoke-GitLocal {
    param([Parameter(Mandatory)][string]$RepoPath, [Parameter(Mandatory)][string[]]$GitArgs)
    # -c core.autocrlf=false: avoid Windows CRLF false positives in analysis.
    $full = @('-c', 'core.autocrlf=false', '-C', $RepoPath) + $GitArgs
    # Reset $LASTEXITCODE to a sentinel so a launch failure (git not executable)
    # is never mistaken for a previous command's exit code. try/catch maps an
    # actual spawn exception to Ok=$false; stderr text alone does NOT throw here
    # (the 2>&1 merge just captures it), so failed-to-run is distinguished from ran-and-failed.
    $global:LASTEXITCODE = -1
    try {
        $out = (& git @full 2>&1 | Out-String)
    } catch {
        return @{ Ok = $false; Code = $null; Out = "git не запустился: $($_.Exception.Message)" }
    }
    return @{ Ok = ($LASTEXITCODE -eq 0); Code = $LASTEXITCODE; Out = $out.Trim() }
}

# NETWORK calls (git fetch / gh) with a HARD timeout live in ScriptKit:
#   Invoke-Timed          -> Invoke-TimedCommand
#   Invoke-TimedSpinner   -> Invoke-TimedCommandWithSpinner
# Same @{ Ok; Code; Out } contract; callers below use the kit names.
# endregion

# ─────────────────────────────────────────────────────────────────────────────
# region  Repo discovery + role detection
# ─────────────────────────────────────────────────────────────────────────────

# Parse "owner/repo" from a GitHub remote URL (https or ssh).
function Get-OwnerRepoFromUrl {
    param([string]$Url)
    if (-not $Url) { return $null }
    $u = $Url.Trim() -replace '/+$', ''   # strip trailing slashes (…/repo/ , …/repo.git/)
    $u = $u -replace '\.git$', ''
    if ($u -match 'github\.com[/:]+([^/]+)/(.+)$') {
        return ('{0}/{1}' -f $Matches[1], $Matches[2])
    }
    return $null
}

# All remotes of a repo as @{ Name; Url; OwnerRepo }.
function Get-RepoRemotes {
    param([string]$RepoPath)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', '-v')
    if (-not $r.Ok) { return @() }
    $seen = @{}
    $list = New-Object System.Collections.Generic.List[object]
    foreach ($line in ($r.Out -split "`n")) {
        if ($line -match '^(\S+)\s+(\S+)\s+\(fetch\)') {
            $name = $Matches[1]
            if ($seen.ContainsKey($name)) { continue }
            $seen[$name] = $true
            $list.Add([pscustomobject]@{ Name = $name; Url = $Matches[2]; OwnerRepo = (Get-OwnerRepoFromUrl $Matches[2]) })
        }
    }
    return $list.ToArray()
}

# Determine which remote is the fork and which is the upstream (parent), plus
# the upstream's default branch. Uses gh (`--json parent,defaultBranchRef`) as
# the authority; falls back to a naming heuristic if gh is unavailable.
function Resolve-RepoRoles {
    param([string]$RepoPath, [int]$GhTimeoutSec = 60, [switch]$GhAvailable, [switch]$IsOwn)
    $remotes = Get-RepoRemotes -RepoPath $RepoPath
    if ($remotes.Count -eq 0) { return $null }

    $parent = $null; $defaultBranch = $null; $guessed = $true
    if ($GhAvailable) {
        # Query gh on any remote's owner/repo (they all resolve to the same node).
        $anchor = ($remotes | Where-Object { $_.OwnerRepo } | Select-Object -First 1)
        if ($anchor) {
            $g = Invoke-TimedCommand -FilePath 'gh' -TimeoutSec $GhTimeoutSec `
                -ArgList @('repo', 'view', $anchor.OwnerRepo, '--json', 'parent,defaultBranchRef,isFork,nameWithOwner')
            if ($g.Ok -and $g.Out) {
                try {
                    $j = $g.Out | ConvertFrom-Json
                    if ($j.parent) { $parent = ('{0}/{1}' -f $j.parent.owner.login, $j.parent.name) }
                    if ($j.defaultBranchRef) { $defaultBranch = $j.defaultBranchRef.name }
                    $guessed = $false
                } catch {
                    # Malformed gh payload: warn (named source) but keep heuristic fallback.
                    Write-Status "$($anchor.OwnerRepo) : не разобрал JSON от 'gh repo view' — $($_.Exception.Message)" 'WARN'
                }
            }
        }
    }

    # Own (non-fork) repo: `origin` is both the source of truth and where PRs/branches
    # live. No upstream-to-ff-from, no normalize. Compare branches to origin/<default>.
    if ($IsOwn) {
        $origin = ($remotes | Where-Object { $_.Name -eq 'origin' } | Select-Object -First 1)
        if (-not $origin) { $origin = ($remotes | Select-Object -First 1) }
        $ownRepo = if ($origin -and $origin.OwnerRepo) { $origin.OwnerRepo } else { $null }
        if (-not $defaultBranch) {
            $sr = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('symbolic-ref', '-q', "refs/remotes/$($origin.Name)/HEAD")
            $defaultBranch = if ($sr.Ok -and $sr.Out) { ($sr.Out.Trim() -split '/')[-1] } else { 'main' }
        }
        return [pscustomobject]@{
            Fork = $origin; Upstream = $origin; DefaultBranch = $defaultBranch
            ParentOwnerRepo = $ownRepo; IsFork = $false; IsOwn = $true; Guessed = $false
        }
    }

    $fork = $null; $upstream = $null
    if ($parent) {
        # Match parent owner/repo to a remote → that's upstream; the other → fork.
        $upstream = $remotes | Where-Object { $_.OwnerRepo -and ($_.OwnerRepo -ieq $parent) } | Select-Object -First 1
        $fork     = $remotes | Where-Object { $_.OwnerRepo -and ($_.OwnerRepo -ine $parent) } | Select-Object -First 1
    }
    if (-not $upstream) {
        # Heuristic fallback: prefer a remote literally named 'upstream', else 'origin'.
        $upstream = ($remotes | Where-Object { $_.Name -eq 'upstream' } | Select-Object -First 1)
        if (-not $upstream) { $upstream = ($remotes | Where-Object { $_.Name -eq 'origin' } | Select-Object -First 1) }
        if (-not $upstream) { $upstream = ($remotes | Select-Object -First 1) }
    }
    if (-not $fork) {
        $fork = ($remotes | Where-Object { $_.Name -in @('fork','origin') -and $_.Name -ne $upstream.Name } | Select-Object -First 1)
        if (-not $fork) { $fork = ($remotes | Where-Object { $_.Name -ne $upstream.Name } | Select-Object -First 1) }
    }
    if (-not $defaultBranch) {
        # Fall back to the upstream remote's HEAD symbolic ref, else 'main'.
        $sr = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('symbolic-ref', '-q', "refs/remotes/$($upstream.Name)/HEAD")
        if ($sr.Ok -and $sr.Out -match "refs/remotes/$([regex]::Escape($upstream.Name))/(.+)$") { $defaultBranch = $Matches[1] }
        if (-not $defaultBranch) { $defaultBranch = 'main' }
    }

    return [pscustomobject]@{
        Fork = $fork; Upstream = $upstream; DefaultBranch = $defaultBranch
        ParentOwnerRepo = $parent; IsFork = [bool]$parent; IsOwn = $false; Guessed = $guessed
    }
}

# Enumerate the configured repos: every git repo found under `roots/*` plus each
# explicit `paths` entry. Non-git dirs are skipped here; non-fork filtering
# happens after role resolution.
function Find-ManagedRepos {
    param([string[]]$Roots, [string[]]$Paths)
    $found = New-Object System.Collections.Generic.List[string]
    foreach ($root in ($Roots | Where-Object { $_ })) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue | ForEach-Object {
            if (Test-Path -LiteralPath (Join-Path $_.FullName '.git')) { $found.Add($_.FullName) }
        }
    }
    foreach ($p in ($Paths | Where-Object { $_ })) {
        if ((Test-Path -LiteralPath $p) -and (Test-Path -LiteralPath (Join-Path $p '.git'))) { $found.Add((Resolve-Path -LiteralPath $p).Path) }
    }
    return ($found | Select-Object -Unique)
}
# endregion

# ─────────────────────────────────────────────────────────────────────────────
# region  Read-only git analysis
# ─────────────────────────────────────────────────────────────────────────────

# Resolve the real git dir (robust against worktrees / .git files).
function Get-GitDir {
    param([string]$RepoPath)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--absolute-git-dir')
    if ($r.Ok) { return $r.Out }
    return (Join-Path $RepoPath '.git')
}

# Working-tree / in-progress-operation health.
function Get-RepoHealth {
    param([string]$RepoPath)
    $gitDir = Get-GitDir -RepoPath $RepoPath
    $dirty = $false; $untracked = $false
    $st = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('status', '--porcelain')
    if ($st.Ok -and $st.Out) {
        foreach ($line in ($st.Out -split "`n")) {
            $l = $line.Trim()
            if (-not $l) { continue }
            if ($l -match '^\?\?') { $untracked = $true } else { $dirty = $true }  # tracked change
        }
    }
    $midOp = $false; $opName = $null
    foreach ($pair in @(@('rebase-merge','rebase'), @('rebase-apply','rebase'), @('MERGE_HEAD','merge'), @('CHERRY_PICK_HEAD','cherry-pick'), @('REVERT_HEAD','revert'))) {
        if (Test-Path -LiteralPath (Join-Path $gitDir $pair[0])) { $midOp = $true; $opName = $pair[1]; break }
    }
    $hr = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('symbolic-ref', '-q', 'HEAD')
    $detached = -not $hr.Ok
    $current = if ($hr.Ok -and $hr.Out -match 'refs/heads/(.+)$') { $Matches[1] } else { $null }
    return [pscustomobject]@{ Dirty = $dirty; Untracked = $untracked; MidOp = $midOp; OpName = $opName; Detached = $detached; CurrentBranch = $current }
}

# Local branch names (short).
function Get-LocalBranches {
    param([string]$RepoPath)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('for-each-ref', '--format=%(refname:short)', 'refs/heads/')
    if (-not $r.Ok) { return @() }
    # Trim each line: Out-String joins with CRLF, so splitting on \n leaves a
    # trailing \r on every line but the last — which would corrupt ref names.
    return @($r.Out -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

# How far the local default branch is behind upstream's default, and whether a
# fast-forward is safe (local default is an ancestor of upstream default).
function Get-DefaultBranchStatus {
    param([string]$RepoPath, [string]$LocalDefault, [string]$UpstreamRef)
    $behind = $null; $ahead = $null; $ffSafe = $false; $exists = $false
    $chk = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', "refs/heads/$LocalDefault")
    if ($chk.Ok) {
        $exists = $true
        $rc = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-list', '--count', "$LocalDefault..$UpstreamRef")
        if ($rc.Ok -and $rc.Out -match '^\d+$') { $behind = [int]$rc.Out }
        # Commits in the LOCAL default not upstream — usually a mistake (committed straight to main),
        # and the reason ff is impossible. Surfaced so the UI can say "main has N own commits".
        $ra = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-list', '--count', "$UpstreamRef..$LocalDefault")
        if ($ra.Ok -and $ra.Out -match '^\d+$') { $ahead = [int]$ra.Out }
        $anc = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('merge-base', '--is-ancestor', $LocalDefault, $UpstreamRef)
        $ffSafe = $anc.Ok
    }
    return [pscustomobject]@{ Exists = $exists; BehindBy = $behind; AheadBy = $ahead; FfSafe = $ffSafe }
}

# Count of branch commits whose patch is NOT yet in upstream (`git cherry` '+').
# NOTE: blind to squash-merges of multi-commit branches — a HINT only.
function Get-BranchCherryPlus {
    param([string]$RepoPath, [string]$UpstreamRef, [string]$Branch)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('cherry', $UpstreamRef, $Branch)
    if (-not $r.Ok) { return $null }
    if (-not $r.Out) { return 0 }
    return @($r.Out -split "`n" | Where-Object { $_ -match '^\+' }).Count
}

# Commits on the branch ahead of upstream default.
function Get-BranchAhead {
    param([string]$RepoPath, [string]$UpstreamRef, [string]$Branch)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-list', '--count', "$UpstreamRef..$Branch")
    if ($r.Ok -and $r.Out -match '^\d+$') { return [int]$r.Out }
    return $null
}

# Predict whether the branch would conflict when integrated onto upstream
# default, via `git merge-tree --write-tree --messages` (git>=2.38). A
# merge-based proxy for a rebase conflict. Returns @{ Conflict; Files }.
function Test-RebaseConflict {
    param([string]$RepoPath, [string]$UpstreamRef, [string]$Branch)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('merge-tree', '--write-tree', '--messages', $UpstreamRef, $Branch)
    if ($r.Code -eq 0) { return [pscustomobject]@{ Conflict = $false; Files = @() } }
    if ($r.Code -eq 1) {
        # Exit 1 is overloaded: a real merge conflict AND failures like an
        # unresolvable ref ("<ref> - not something we can merge"). Only treat it
        # as a conflict when the output actually carries CONFLICT markers —
        # otherwise it's an analysis error, not a phantom "needs hands" conflict.
        $files = @()
        foreach ($line in ($r.Out -split "`n")) {
            if ($line -match 'CONFLICT \([^)]+\):\s+Merge conflict in (.+)$') { $files += $Matches[1].Trim() }
            elseif ($line -match 'CONFLICT \([^)]+\):\s+(.+)$') { $files += $Matches[1].Trim() }
        }
        if (($r.Out -notmatch 'CONFLICT \(') -and @($files).Count -eq 0) {
            # No conflict markers => not a real merge conflict (e.g. ref does not
            # resolve because the remote fetches into a non-standard refspec).
            return [pscustomobject]@{ Conflict = $null; Files = @(); Error = (($r.Out -split "`n") | Where-Object { $_ } | Select-Object -First 1) }
        }
        return [pscustomobject]@{ Conflict = $true; Files = ($files | Select-Object -Unique) }
    }
    # Other exit codes => analysis error (e.g. unknown ref); report as unknown.
    return [pscustomobject]@{ Conflict = $null; Files = @(); Error = $r.Out }
}

# Local branch vs its counterpart on the fork remote (ahead/behind). Tells
# whether a future rebase would require a force-push to update the PR.
function Get-ForkDivergence {
    param([string]$RepoPath, [string]$ForkRemote, [string]$Branch)
    if (-not $ForkRemote) { return [pscustomobject]@{ OnFork = $false; Ahead = $null; Behind = $null } }
    $ref = "refs/remotes/$ForkRemote/$Branch"
    $chk = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', $ref)
    if (-not $chk.Ok) { return [pscustomobject]@{ OnFork = $false; Ahead = $null; Behind = $null } }
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-list', '--left-right', '--count', "$ref...$Branch")
    if ($r.Ok -and $r.Out -match '^(\d+)\s+(\d+)') {
        return [pscustomobject]@{ OnFork = $true; Behind = [int]$Matches[1]; Ahead = [int]$Matches[2] }
    }
    return [pscustomobject]@{ OnFork = $true; Ahead = $null; Behind = $null }
}
# endregion

# ─────────────────────────────────────────────────────────────────────────────
# region  GitHub PRs (source of truth) + classifier (pure)
# ─────────────────────────────────────────────────────────────────────────────

# One gh call per repo: all PRs (any state) with their head branch. Returned as
# an array of PR objects. Swappable in tests (return a fixture array).
# $script:GetRepoPrsOverride lets tests inject a stub without mocking.
$script:GetRepoPrsOverride = $null
function Get-RepoPrs {
    param([string]$UpstreamOwnerRepo, [int]$GhTimeoutSec = 60, [switch]$GhAvailable)
    if ($script:GetRepoPrsOverride) { return (& $script:GetRepoPrsOverride $UpstreamOwnerRepo) }
    if (-not $GhAvailable -or -not $UpstreamOwnerRepo) { return $null }   # null => unknown
    $g = Invoke-TimedCommand -FilePath 'gh' -TimeoutSec $GhTimeoutSec -ArgList @(
        'pr', 'list', '--repo', $UpstreamOwnerRepo, '--state', 'all', '--limit', '200',
        '--json', 'number,state,url,headRefName,headRepositoryOwner,mergedAt,isCrossRepository,statusCheckRollup'
    )
    if (-not $g.Ok -or -not $g.Out) { return $null }
    try { return @($g.Out | ConvertFrom-Json) }
    catch {
        # Malformed gh payload: warn (named source) but stay 'unknown' (return $null).
        Write-Status "$UpstreamOwnerRepo : не разобрал JSON от 'gh pr list' — $($_.Exception.Message)" 'WARN'
        return $null
    }
}

# Summarize a PR's CI check rollup → 'pass' | 'fail' | 'pending' | 'none'.
function Get-ChecksSummary {
    param([pscustomobject]$Pr)
    if (-not $Pr) { return $null }
    if (-not ($Pr.PSObject.Properties.Name -contains 'statusCheckRollup')) { return 'none' }
    $roll = $Pr.statusCheckRollup
    if (-not $roll -or @($roll).Count -eq 0) { return 'none' }
    $fail = $false; $pending = $false; $any = $false
    foreach ($c in @($roll)) {
        $s = ''
        if (($c.PSObject.Properties.Name -contains 'conclusion') -and $c.conclusion) { $s = "$($c.conclusion)".ToUpper() }
        elseif (($c.PSObject.Properties.Name -contains 'state') -and $c.state) { $s = "$($c.state)".ToUpper() }
        elseif (($c.PSObject.Properties.Name -contains 'status') -and $c.status) { $s = "$($c.status)".ToUpper() }
        if (-not $s) { continue }   # unrecognized check item — don't count it as a pass
        $any = $true
        if ($s -match 'FAIL|ERROR|TIMED_OUT|CANCELL|ACTION_REQUIRED') { $fail = $true }
        elseif ($s -match 'PENDING|IN_PROGRESS|QUEUED|WAITING|EXPECTED|REQUESTED') { $pending = $true }
    }
    if ($fail) { return 'fail' }
    if ($pending) { return 'pending' }
    if ($any) { return 'pass' }
    return 'none'
}

# Find the PR (if any) for a given local branch in the repo's PR list.
function Find-PrForBranch {
    param($Prs, [string]$Branch, [string]$ForkOwner)
    if ($null -eq $Prs) { return $null }
    $cands = @($Prs | Where-Object { $_.headRefName -eq $Branch })
    if ($ForkOwner) {
        $byOwner = @($cands | Where-Object { $_.headRepositoryOwner -and ($_.headRepositoryOwner.login -ieq $ForkOwner) })
        if ($byOwner.Count -gt 0) { $cands = $byOwner }
    }
    if ($cands.Count -eq 0) { return $null }
    # Prefer a MERGED, then OPEN, then most-recent.
    $merged = @($cands | Where-Object { $_.state -eq 'MERGED' }) | Select-Object -First 1
    if ($merged) { return $merged }
    $open = @($cands | Where-Object { $_.state -eq 'OPEN' }) | Select-Object -First 1
    if ($open) { return $open }
    return ($cands | Select-Object -First 1)
}

<#
  PURE classifier — the testable heart. Given the git facts for a branch and its
  PR (or $null), return the outcome. PR state is authoritative for "merged".

  Priority:  merged  >  conflict  >  clean  >  local-only
   - merged    : PR.state == MERGED (squash-safe), OR no PR but cherry shows all
                 commits already upstream (cherryPlus == 0) AND it has commits.
   - conflict  : would conflict integrating onto upstream default.
   - clean     : has commits ahead, integrates cleanly, PR open / exists.
   - local-only: no PR and not (provably) merged — a local-only branch.
#>
function Get-ClassifiedOutcome {
    param(
        [pscustomobject]$Pr,         # may be $null
        [Nullable[int]]$AheadOfUpstream,
        [Nullable[int]]$CherryPlus,
        $Conflict                    # $true / $false / $null(unknown)
    )
    if ($Pr -and $Pr.state -eq 'MERGED') { return 'merged' }
    if ($Conflict -eq $true) { return 'conflict' }
    # Content already upstream: every branch commit's patch is in the upstream
    # default. Catches branches batch-merged / cherry-picked / squashed into
    # ANOTHER PR (which leaves THIS PR CLOSED, or leaves no PR) — still safe to
    # delete. NOTE: blind to a squash that rewrote the patches (then cherry > 0,
    # and we stay conservative: clean / closed-unmerged).
    $contentUpstream = (($null -ne $CherryPlus) -and ($CherryPlus -eq 0) -and ($AheadOfUpstream) -and ($AheadOfUpstream -gt 0))
    if (-not $Pr) {
        if ($contentUpstream) { return 'merged' }
        return 'local-only'
    }
    if ($Pr.state -eq 'CLOSED') {
        if ($contentUpstream) { return 'merged' }
        return 'closed-unmerged'
    }
    return 'clean'   # OPEN PR — never auto-classify as merged/deletable
}

# Human action hint for an outcome.
function Get-ActionHint {
    param([string]$Outcome, $Divergence)
    switch ($Outcome) {
        'merged'           { 'влито → ветку можно удалять' ; break }
        'conflict'         { 'будет конфликт → нужны руки' ; break }
        'clean'            { if ($Divergence -and $Divergence.OnFork -and $Divergence.Ahead -gt 0) { 'открыт; обновить PR можно force-with-lease' } else { 'открыт; ляжет на upstream чисто' } ; break }
        'closed-unmerged'  { 'PR закрыт без merge → решить, нужна ли ветка' ; break }
        'local-only'       { 'локальная ветка без PR' ; break }
        default            { '' }
    }
}
# endregion

# ─────────────────────────────────────────────────────────────────────────────
# region  Orchestrator
# ─────────────────────────────────────────────────────────────────────────────

function Get-ForkSyncConfig {
    param([string]$Root, [string[]]$Roots, [string[]]$Paths, [int]$FetchTimeoutSec, [int]$GhTimeoutSec, [string]$ConfigPath)
    # Durable config path (Castellyn writes %APPDATA%\castellyn\forks.json) wins over the vendored
    # repos.json next to this module — so a user's fork config isn't clobbered on a tool update.
    $cfgFile = if ($ConfigPath -and (Test-Path -LiteralPath $ConfigPath)) { $ConfigPath } else { Join-Path $Root 'repos.json' }
    $cfg = $null
    if (Test-Path -LiteralPath $cfgFile) {
        try { $cfg = Get-Content -Raw -LiteralPath $cfgFile | ConvertFrom-Json }
        catch {
            # Malformed repos.json: warn (named file) but keep going with defaults.
            Write-Status "repos.json : не разобрал ($cfgFile) — $($_.Exception.Message)" 'WARN'
        }
    }
    # Read by key PRESENCE, not truthiness — two distinct reasons, both live:
    #  * roots/paths: an empty array is falsy in PowerShell, so a registry the user emptied in the
    #    Forks tab used to fall through to the hardcoded developer defaults and keep scanning (and
    #    MUTATING, under -FfMain/-DeleteMerged) repos that were de-registered. Absent = defaults,
    #    present-but-empty = scan nothing.
    #  * timeouts: Set-StrictMode makes reading a missing property a TERMINATING error, and
    #    Castellyn's writer omits fetchTimeoutSec/ghTimeoutSec whenever they are unset — that threw
    #    before any status envelope was written, leaving the Forks card frozen on its last state.
    # Falsy ELEMENTS are still dropped (a JSON null lands as a one-null array under @()).
    $keys = if ($cfg) { @($cfg.PSObject.Properties.Name) } else { @() }
    $r = if ($Roots) { $Roots } elseif ($keys -contains 'roots') { @($cfg.roots | Where-Object { $_ }) } else { @('E:\Scripts\External') }
    $p = if ($Paths) { $Paths } elseif ($keys -contains 'paths') { @($cfg.paths | Where-Object { $_ }) } else { @('C:\Users\User\rtk-windows-hook-pr\rtk') }
    $op = if ($keys -contains 'ownPaths') { @($cfg.ownPaths | Where-Object { $_ }) } else { @() }
    # Timeouts keep the truthiness check on top of presence: a configured 0 is not a usable timeout.
    $ft = if ($FetchTimeoutSec) { $FetchTimeoutSec } elseif (($keys -contains 'fetchTimeoutSec') -and $cfg.fetchTimeoutSec) { [int]$cfg.fetchTimeoutSec } else { 120 }
    $gt = if ($GhTimeoutSec) { $GhTimeoutSec } elseif (($keys -contains 'ghTimeoutSec') -and $cfg.ghTimeoutSec) { [int]$cfg.ghTimeoutSec } else { 60 }
    return [pscustomobject]@{ Roots = $r; Paths = $p; OwnPaths = $op; FetchTimeoutSec = $ft; GhTimeoutSec = $gt }
}

# Analyze a single repo (read-only). Returns the per-repo report object.
function Get-RepoReport {
    param([string]$RepoPath, [pscustomobject]$Config, [switch]$NoFetch, [switch]$GhAvailable, [switch]$IsOwn)
    $name = Split-Path $RepoPath -Leaf
    $roles = Resolve-RepoRoles -RepoPath $RepoPath -GhTimeoutSec $Config.GhTimeoutSec -GhAvailable:$GhAvailable -IsOwn:$IsOwn
    if (-not $roles) { return [pscustomobject]@{ Name = $name; Path = $RepoPath; Skipped = 'no-remotes' } }
    if (-not $IsOwn -and -not $roles.IsFork -and $roles.Guessed -eq $false) {
        return [pscustomobject]@{ Name = $name; Path = $RepoPath; Skipped = 'not-a-fork' }
    }

    $upstreamName = $roles.Upstream.Name
    $upstreamRef  = "$upstreamName/$($roles.DefaultBranch)"
    $forkOwner      = if ($roles.Fork -and $roles.Fork.OwnerRepo) { ($roles.Fork.OwnerRepo -split '/')[0] } else { $null }
    $forkRemoteName = if ($roles.Fork) { $roles.Fork.Name } else { $null }

    if (-not $NoFetch) {
        $f = Invoke-TimedCommandWithSpinner -FilePath 'git' -TimeoutSec $Config.FetchTimeoutSec -Activity "fetch $name ($upstreamName)" `
            -ArgList @('-C', $RepoPath, 'fetch', '--quiet', $upstreamName)
        if (-not $f.Ok) { Write-Status "$name : fetch upstream не удался ($($f.Out -split "`n" | Select-Object -First 1))" 'WARN' }
    }

    $health = Get-RepoHealth -RepoPath $RepoPath
    $def    = Get-DefaultBranchStatus -RepoPath $RepoPath -LocalDefault $roles.DefaultBranch -UpstreamRef $upstreamRef

    $branchReports = New-Object System.Collections.Generic.List[object]
    $wip = $null
    if (-not $health.MidOp -and -not $health.Detached) {
        $prs = Get-RepoPrs -UpstreamOwnerRepo $roles.ParentOwnerRepo -GhTimeoutSec $Config.GhTimeoutSec -GhAvailable:$GhAvailable
        $skip = @($roles.DefaultBranch, 'wip-local')
        # PERF (P1-06, accepted as-is): each branch costs ~5 separate local git spawns
        # (ahead/cherry/merge-tree/fork-divergence + PR lookup), i.e. N_branches × ~5
        # processes per repo — merge-tree being the heaviest. Fine for typical fork
        # repos (few topic branches). If branch counts ever grow, batch the
        # ahead/cherry passes and reserve merge-tree for branches with ahead>0.
        foreach ($b in (Get-LocalBranches -RepoPath $RepoPath)) {
            if ($b -in $skip) { continue }
            $ahead   = Get-BranchAhead       -RepoPath $RepoPath -UpstreamRef $upstreamRef -Branch $b
            $cherry  = Get-BranchCherryPlus  -RepoPath $RepoPath -UpstreamRef $upstreamRef -Branch $b
            $conf    = Test-RebaseConflict   -RepoPath $RepoPath -UpstreamRef $upstreamRef -Branch $b
            $div     = Get-ForkDivergence    -RepoPath $RepoPath -ForkRemote $forkRemoteName -Branch $b
            $pr      = Find-PrForBranch -Prs $prs -Branch $b -ForkOwner $forkOwner
            $outcome = Get-ClassifiedOutcome -Pr $pr -AheadOfUpstream $ahead -CherryPlus $cherry -Conflict $conf.Conflict
            $branchReports.Add([pscustomobject]@{
                name = $b
                prNumber = if ($pr) { $pr.number } else { $null }
                prState  = if ($pr) { $pr.state } elseif ($null -eq $prs) { 'unknown' } else { 'none' }
                url      = if ($pr) { $pr.url } else { $null }
                outcome  = $outcome
                conflictFiles = $conf.Files
                aheadOfUpstream = $ahead
                cherryPlus = $cherry
                divergedFromForkAhead = $div.Ahead
                checks = if ($pr) { Get-ChecksSummary -Pr $pr } else { $null }
                action = (Get-ActionHint -Outcome $outcome -Divergence $div)
            })
        }
        # wip-local staleness (read-only; never touched). `git cherry upstream wip-local` lines:
        #   '-' = a wip-local patch ALREADY in upstream (merged); '+' = a UNIQUE patch not upstream.
        # uniquePatches == 0 ⇒ wip-local holds nothing new ⇒ it's redundant and can be deleted.
        $wipExists = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse','--verify','--quiet','refs/heads/wip-local')
        if ($wipExists.Ok) {
            $wbehind = Get-BranchAhead -RepoPath $RepoPath -UpstreamRef "wip-local" -Branch $upstreamRef  # commits in upstream not in wip-local
            $wmerged = $null; $wunique = $null
            $wc = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('cherry', $upstreamRef, 'wip-local')
            if ($wc.Ok) {
                $wlines  = @($wc.Out -split "`n")
                $wmerged = @($wlines | Where-Object { $_ -match '^-' }).Count
                $wunique = @($wlines | Where-Object { $_ -match '^\+' }).Count
            }
            $wip = [pscustomobject]@{ behindBy = $wbehind; mergedPatches = $wmerged; uniquePatches = $wunique }
        }
    }

    # When the upstream tip last moved (freshness of the original) — pairs with behindBy.
    $upstreamUpdated = $null
    $lc = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('log','-1','--format=%cI',$upstreamRef)
    if ($lc.Ok -and $lc.Out) { $upstreamUpdated = $lc.Out.Trim() }

    # Upstream lifecycle (best-effort, gh only, forks only): archived ⇒ dead fork; a different default
    # branch ⇒ the original renamed it (master→main) and your fork silently can't sync. One gh call/fork.
    $upstreamArchived = $null; $upstreamDefault = $null
    if ($GhAvailable -and -not $IsOwn -and $roles.ParentOwnerRepo) {
        $pv = Invoke-TimedCommand -FilePath 'gh' -TimeoutSec $Config.GhTimeoutSec `
            -ArgList @('repo','view',$roles.ParentOwnerRepo,'--json','isArchived,defaultBranchRef')
        if ($pv.Ok -and $pv.Out) {
            try {
                $pj = $pv.Out | ConvertFrom-Json
                $upstreamArchived = [bool]$pj.isArchived
                if ($pj.defaultBranchRef) { $upstreamDefault = $pj.defaultBranchRef.name }
            } catch {
                Write-Status "$($roles.ParentOwnerRepo) : не разобрал isArchived/defaultBranch — $($_.Exception.Message)" 'WARN'
            }
        }
    }

    return [pscustomobject]@{
        Name = $name; Path = $RepoPath
        upstream = "$upstreamName -> $($roles.ParentOwnerRepo)"
        fork = if ($roles.Fork) { "$($roles.Fork.Name) -> $($roles.Fork.OwnerRepo)" } else { $null }
        defaultBranch = $roles.DefaultBranch
        upstreamRemote = $upstreamName
        forkRemote = $forkRemoteName
        forkOwnerRepo = if ($roles.Fork) { $roles.Fork.OwnerRepo } else { $null }
        parentOwnerRepo = $roles.ParentOwnerRepo
        currentBranch = $health.CurrentBranch
        rolesGuessed = $roles.Guessed
        isOwn = [bool]$IsOwn
        behindBy = $def.BehindBy; ffSafe = $def.FfSafe; defaultAhead = $def.AheadBy
        dirty = $health.Dirty; untracked = $health.Untracked; midOp = $health.MidOp; opName = $health.OpName; detached = $health.Detached
        branches = $branchReports.ToArray()
        wipLocal = $wip
        upstreamUpdated = $upstreamUpdated
        upstreamArchived = $upstreamArchived
        upstreamDefaultBranch = $upstreamDefault
        Skipped = $null
    }
}

# Render the human-readable per-repo report.
function Write-RepoHuman {
    param([pscustomobject]$Rep, [int]$Index = 0, [int]$Total = 0)
    # [i/N] counter prefix on the section header (skipped repos are numbered too).
    $prefix = if ($Index -gt 0 -and $Total -gt 0) { "[$Index/$Total] " } else { '' }
    if ($Rep.Skipped) { Write-Status "$prefix$($Rep.Name): пропущен ($($Rep.Skipped))" 'SKIP'; return }
    $note = if ($Rep.isOwn) { "СВОЙ репозиторий (не форк): $($Rep.parentOwnerRepo)  |  default: $($Rep.defaultBranch)" } else { "$($Rep.fork)  |  upstream $($Rep.upstream)  |  default: $($Rep.defaultBranch)" }
    Write-Section "$prefix$($Rep.Name)" $note 'Cyan'
    if ($Rep.rolesGuessed) { Write-Status "роли remote определены эвристикой (gh недоступен?)" 'WARN' }
    if ($Rep.detached) { Write-Status "detached HEAD → ветки не анализирую" 'WARN'; return }
    if ($Rep.midOp)    { Write-Status "репозиторий в процессе '$($Rep.opName)' → нужны руки, ветки не анализирую" 'WARN'; return }
    if ($Rep.dirty)         { Write-Status "незакоммиченные изменения (tracked) — могут мешать ff/rebase" 'WARN' }
    elseif ($Rep.untracked) { Write-Status "есть неотслеживаемые файлы (untracked) — синку не мешают" 'INFO' }

    if ($null -ne $Rep.behindBy) {
        if ($Rep.behindBy -eq 0) { Write-Status "main/default в синхроне с upstream" 'OK' }
        elseif ($Rep.ffSafe)     { Write-Status "default отстаёт на $($Rep.behindBy) — fast-forward безопасен" 'INFO' }
        else                     { Write-Status "default разошёлся с upstream (отстаёт $($Rep.behindBy), ff невозможен — в него коммитили?)" 'WARN' }
    }
    if ($Rep.wipLocal) { Write-Status "wip-local: отстаёт на $($Rep.wipLocal.behindBy) от upstream, уже влито коммитов: $($Rep.wipLocal.mergedPatches) → подумать о пересборке" 'INFO' }

    if (@($Rep.branches).Count -eq 0) { Write-Status "своих топик-веток нет" 'SKIP'; return }
    foreach ($b in $Rep.branches) {
        $tag = switch ($b.outcome) { 'merged' {'MERGED'} 'conflict' {'CONFLICT'} 'clean' {'OPEN'} 'closed-unmerged' {'CLOSED'} 'local-only' {'LOCAL'} default {'INFO'} }
        $pr  = if ($b.prNumber) { "PR #$($b.prNumber) [$($b.prState)]" } else { "(нет PR)" }
        $act = $b.action
        if ($b.outcome -eq 'merged' -and $b.prState -eq 'CLOSED') { $act = 'влито в составе другого PR → можно удалять' }
        $txt = "{0,-34} {1,-16} {2}" -f $b.name, $pr, $act
        if ($b.checks -eq 'fail') { $txt += '  [CI ✗]' } elseif ($b.checks -eq 'pending') { $txt += '  [CI ⧗]' } elseif ($b.checks -eq 'pass') { $txt += '  [CI ✓]' }
        if ($b.outcome -eq 'conflict' -and @($b.conflictFiles).Count -gt 0) { $txt += "  — файлы: " + (($b.conflictFiles | Select-Object -First 4) -join ', ') }
        Write-Status $txt $tag
    }
}

function Write-ForkSyncJson {
    param([string]$Root, [pscustomobject]$Payload, [string]$OutFile)
    $path = if ($OutFile) { $OutFile } else { Join-Path $Root 'fork-sync.last.json' }
    # Atomic publish: write a sibling temp, then replace. The Rust reader polls this file right after
    # the run finishes; a plain WriteAllText truncates-then-writes, so a poll mid-write would read a
    # torn / empty JSON and the card would go stale. Move-with-overwrite swaps it in as one step.
    $tmp = "$path.tmp"
    [System.IO.File]::WriteAllText($tmp, ($Payload | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::Move($tmp, $path, $true)
    return $path
}

# ── Mutations (Phase 2 — safe, backed-up, confirmed; never auto force-push) ──

# Snapshot every LOCAL branch HEAD into refs/fork-sync/pre-sync/<stamp>/<branch>
# so a local-branch mutation is reversible: `git update-ref refs/heads/<b> <backup-sha>`.
# SCOPE: local branch heads ONLY. This does NOT back up remote-tracking refs, the
# remote configuration (see Invoke-NormalizeRemotes, which logs `git remote -v`
# separately), or branches deleted on the fork (a `push --delete` is undone by
# re-pushing the backed-up SHA, not by this snapshot).
function New-BackupRefs {
    # ShouldProcess so -WhatIf really stops the ref writes. ConfirmImpact stays default (Medium),
    # below the default $ConfirmPreference of High, so the unattended runs Castellyn launches
    # (-Yes -Unattended) never get a prompt here.
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$RepoPath, [string]$Stamp)
    if (-not $Stamp) { $Stamp = (Get-Date -Format 'yyyyMMdd_HHmmss') }
    $ns = "refs/fork-sync/pre-sync/$Stamp"
    $made = New-Object System.Collections.Generic.List[object]
    if (-not $PSCmdlet.ShouldProcess($RepoPath, "snapshot local branch heads into $ns")) {
        return [pscustomobject]@{ Namespace = $ns; Stamp = $Stamp; Refs = @() }
    }
    foreach ($b in (Get-LocalBranches -RepoPath $RepoPath)) {
        $sha = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', "refs/heads/$b")
        if ($sha.Ok -and $sha.Out) {
            $u = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('update-ref', "$ns/$b", $sha.Out.Trim())
            if ($u.Ok) { $made.Add([pscustomobject]@{ Branch = $b; Ref = "$ns/$b"; Sha = $sha.Out.Trim() }) }
        }
    }
    return [pscustomobject]@{ Namespace = $ns; Stamp = $Stamp; Refs = $made.ToArray() }
}

# Keep only the last $Keep backup snapshots (by timestamp segment).
function Remove-OldBackups {
    # See New-BackupRefs: -WhatIf must not delete refs; Medium impact keeps unattended runs silent.
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$RepoPath, [int]$Keep = 10)
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('for-each-ref', '--format=%(refname)', 'refs/fork-sync/pre-sync/')
    if (-not $r.Ok -or -not $r.Out) { return }
    $refs = @($r.Out -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
    $stamps = @($refs | ForEach-Object { ($_ -split '/')[3] } | Select-Object -Unique | Sort-Object -Descending)
    foreach ($s in @($stamps | Select-Object -Skip $Keep)) {
        if (-not $PSCmdlet.ShouldProcess($RepoPath, "delete backup snapshot $s")) { continue }
        foreach ($ref in @($refs | Where-Object { ($_ -split '/')[3] -eq $s })) {
            Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('update-ref', '-d', $ref) | Out-Null
        }
    }
}

function Confirm-Step {
    param([string]$Question, [switch]$Yes)
    if ($Yes) { return $true }
    # Coerce to a string BEFORE Trim: at EOF (an unattended run with no console) Read-Host returns
    # $null, and $null.Trim() throws under ErrorActionPreference='Stop', aborting the whole run before
    # the status is written. "$(...)" turns $null into '' → returns $false, a safe 'no'. Covers every
    # call site (incl. the MERGED-PR delete path reachable unattended without -Yes).
    $a = "$(Read-Host "    $Question [y/N]")".Trim()
    return ($a -match '^(y|yes|д|да)$')
}

# Fast-forward the local default branch to upstream, then sync the fork's remote
# default via `gh repo sync` (no --force). Only when ffSafe.
function Invoke-FfDefault {
    param([string]$RepoPath, [pscustomobject]$Rep, [switch]$DryRun)
    $default = $Rep.defaultBranch
    $upstreamRef = "$($Rep.upstreamRemote)/$default"
    if ($null -eq $Rep.behindBy -or $Rep.behindBy -eq 0) { return "$default уже синхронизирован" }
    if (-not $Rep.ffSafe) { return "$default разошёлся — ff невозможен, пропуск" }
    if ($DryRun) {
        $m = "БУДЕТ: ff локального '$default' до $upstreamRef (+$($Rep.behindBy))"
        if ($Rep.forkOwnerRepo -and -not $Rep.isOwn) { $m += "; gh repo sync $($Rep.forkOwnerRepo) -b $default" }
        return $m
    }
    $ffDone = $false
    if ($Rep.currentBranch -eq $default) {
        # git itself refuses a non-fast-forward here, so this path is TOCTOU-safe.
        $g = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('merge', '--ff-only', $upstreamRef)
        $ffDone = $g.Ok
        $res = if ($g.Ok) { "ff '$default' (+$($Rep.behindBy))" } else { "ОШИБКА ff: $(($g.Out -split "`n")[0])" }
    } else {
        # Off-branch: raw update-ref does NOT check fast-forwardness, and $Rep.ffSafe was
        # computed in the read phase (TOCTOU). Re-verify ancestry + resolve the SHA now,
        # and use update-ref's compare-and-swap (<newvalue> <oldvalue>) so a concurrent
        # move of the local default aborts the update instead of silently rewinding it.
        $oldRef = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', "refs/heads/$default")
        $newRef = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', "$upstreamRef^{commit}")
        $anc    = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('merge-base', '--is-ancestor', $default, $upstreamRef)
        if (-not $oldRef.Ok -or -not $newRef.Ok) {
            $res = "ОШИБКА ff: не удалось разрешить '$default'/$upstreamRef — пропуск"
        } elseif (-not $anc.Ok) {
            $res = "$default больше не предок $upstreamRef (изменился после анализа) — ff отменён"
        } else {
            $sha = $newRef.Out.Trim(); $old = $oldRef.Out.Trim()
            $g = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('update-ref', "refs/heads/$default", $sha, $old)
            $ffDone = $g.Ok
            $res = if ($g.Ok) { "ff '$default' до $upstreamRef (+$($Rep.behindBy))" } else { "ОШИБКА update-ref: $(($g.Out -split "`n")[0])" }
        }
    }
    # Don't sync the fork's remote default off a ff that did not actually happen.
    if ($ffDone -and $Rep.forkOwnerRepo -and -not $Rep.isOwn) {
        $s = Invoke-TimedCommand -FilePath 'gh' -TimeoutSec 60 -ArgList @('repo', 'sync', $Rep.forkOwnerRepo, '-b', $default)
        $res += if ($s.Ok) { "; форк-remote синхронизирован" } else { "; форк-remote: $(($s.Out -split "`n")[0])" }
    }
    return $res
}

# Delete a merged topic branch locally (+ on the fork, outward → push --delete).
function Remove-MergedBranch {
    # -DryRun is this module's own preview path (it RETURNS the plan text the caller prints);
    # SupportsShouldProcess additionally makes the standard -WhatIf real. Impact stays Medium so
    # the unattended `-Yes -Unattended` runs Castellyn launches are never prompted — the human
    # gate for this destructive path is Confirm-Step in Invoke-RepoActions, not $ConfirmPreference.
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$RepoPath, [string]$Branch, [string]$ForkRemote, [switch]$DeleteRemote, [switch]$DryRun)
    if ($DryRun) {
        $m = "БУДЕТ: удалить ветку '$Branch' локально"
        if ($DeleteRemote -and $ForkRemote) { $m += " и на форке ($ForkRemote)" }
        return $m
    }
    $target = if ($DeleteRemote -and $ForkRemote) { "'$Branch' (локально + форк $ForkRemote)" } else { "'$Branch' (локально)" }
    if (-not $PSCmdlet.ShouldProcess($RepoPath, "delete branch $target")) { return "пропущено '$Branch' (-WhatIf)" }
    # `--` ends option parsing so a (pathological) branch name starting with '-' is
    # treated as a ref, not a flag (argv option-injection hardening; git supports `--` here).
    $d = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('branch', '-D', '--', $Branch)
    $res = if ($d.Ok) { "удалена локально '$Branch'" } else { "ОШИБКА удаления '$Branch': $(($d.Out -split "`n")[0])" }
    if ($DeleteRemote -and $ForkRemote) {
        $p = Invoke-TimedCommand -FilePath 'git' -TimeoutSec 60 -ArgList @('-C', $RepoPath, 'push', $ForkRemote, '--delete', '--', $Branch)
        $res += if ($p.Ok) { "; удалена на форке" } else { "; форк: $(($p.Out -split "`n")[0])" }
    }
    return $res
}

# Rename remotes to canon (origin=fork, upstream=original) via temp names to
# avoid collisions; retrack default→upstream. Idempotent (no-op if already canon).
function Invoke-NormalizeRemotes {
    param([string]$RepoPath, [pscustomobject]$Rep, [switch]$DryRun)
    if ($Rep.isOwn) { return "свой репозиторий — выравнивание remote не нужно" }
    $forkName = $Rep.forkRemote; $upName = $Rep.upstreamRemote; $default = $Rep.defaultBranch
    if (-not $forkName -or -not $upName) { return "нет пары fork/upstream — пропуск" }
    if ($forkName -eq 'origin' -and $upName -eq 'upstream') { return "уже канон (origin=форк, upstream=оригинал)" }
    if ($DryRun) { return "БУДЕТ: '$upName'→upstream, '$forkName'→origin; default отслеживает upstream/$default" }
    # The branch-head backup (New-BackupRefs) does NOT cover the remote config, so
    # snapshot the prior remote set to the log → reconstructable if rename misbehaves.
    $remBefore = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', '-v')
    if ($remBefore.Ok -and $remBefore.Out) {
        Write-SkLog -Msg "normalize $($Rep.Name): remote ДО выравнивания:`n$($remBefore.Out)" -Level 'INFO' -NoConsole
    }
    # Stage 1: park both remotes under temp names (collision-free).
    $r1 = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', $upName, '__fs_up')
    if (-not $r1.Ok) { return "ОШИБКА rename ${upName}: $(($r1.Out -split "`n")[0])" }
    $r2 = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', $forkName, '__fs_fork')
    if (-not $r2.Ok) { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_up', $upName) | Out-Null; return "ОШИБКА rename ${forkName}: $(($r2.Out -split "`n")[0])" }
    # Stage 2: temp → canon. Check each rename; on a collision (e.g. a stray
    # 'upstream'/'origin' already exists) roll the temp names back to their
    # originals so the repo is never left with __fs_* remotes.
    $r3 = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_up', 'upstream')
    if (-not $r3.Ok) {
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_fork', $forkName) | Out-Null
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_up', $upName) | Out-Null
        Write-Status "$($Rep.Name): не удалось переименовать в 'upstream' (занято?) — remote откатан к ($upName,$forkName)" 'WARN'
        return "ОШИБКА выравнивания: $(($r3.Out -split "`n")[0]) — remote возвращён как был"
    }
    $r4 = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_fork', 'origin')
    if (-not $r4.Ok) {
        # 'upstream' is already applied; undo it, then restore both originals.
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', 'upstream', '__fs_up') | Out-Null
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_fork', $forkName) | Out-Null
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('remote', 'rename', '__fs_up', $upName) | Out-Null
        Write-Status "$($Rep.Name): не удалось переименовать в 'origin' (занято?) — remote откатан к ($upName,$forkName)" 'WARN'
        return "ОШИБКА выравнивания: $(($r4.Out -split "`n")[0]) — remote возвращён как был"
    }
    $dchk = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse', '--verify', '--quiet', "refs/heads/$default")
    if ($dchk.Ok) { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('branch', "--set-upstream-to=upstream/$default", $default) | Out-Null }
    return "выровнено: origin=форк, upstream=оригинал; default → upstream/$default"
}

# Rebase a topic branch onto the upstream default. Conflict → abort (branch left
# exactly as-was). Restores the original checkout afterward. Never force-pushes
# unless -Push (then with --force-with-lease + confirmation) — rewriting an
# already-pushed PR branch needs a force-push, which we keep explicit.
function Invoke-RebaseBranch {
    param([string]$RepoPath, [string]$Branch, [string]$UpstreamRef, [string]$ForkRemote, [switch]$Push, [switch]$DryRun, [switch]$Yes)
    if ($DryRun) {
        return "БУДЕТ: перебазировать '$Branch' на $UpstreamRef (локально; для обновления PR — push --force-with-lease)"
    }
    $orig = (Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('symbolic-ref', '--quiet', '--short', 'HEAD')).Out.Trim()
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rebase', $UpstreamRef, $Branch)
    if (-not $r.Ok) {
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rebase', '--abort') | Out-Null
        if ($orig) { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('checkout', '--quiet', $orig) | Out-Null }
        return "конфликт при перебазировании '$Branch' → отменено, ветка как была (нужны руки)"
    }
    if ($orig -and $orig -ne $Branch) { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('checkout', '--quiet', $orig) | Out-Null }
    $res = "перебазирована '$Branch' на $UpstreamRef"
    if ($Push -and $ForkRemote) {
        if (Confirm-Step "Обновить PR ветки '$Branch' через force-with-lease ($ForkRemote)?" -Yes:$Yes) {
            $p = Invoke-TimedCommand -FilePath 'git' -TimeoutSec 60 -ArgList @('-C', $RepoPath, 'push', $ForkRemote, '--force-with-lease', '--', $Branch)
            $res += if ($p.Ok) { "; PR обновлён (force-with-lease)" } else { "; push: $(($p.Out -split "`n")[0])" }
        } else { $res += "; PR не обновлён (по твоему выбору)" }
    } else {
        $res += "; локально — для обновления PR нужен -PushRebased"
    }
    return $res
}

# Rebase the personal wip-local integration branch onto the fresh upstream default.
# Conflict → abort (branch left exactly as-was) + restore the original checkout. Never
# pushes (wip-local is a personal branch, not a PR). Backed up by the caller. Mirrors the
# safe pattern of Invoke-RebaseBranch but targets the literal 'wip-local' branch.
function Invoke-SyncWipLocal {
    param([string]$RepoPath, [pscustomobject]$Rep, [switch]$DryRun)
    if (-not $Rep.wipLocal) { return "нет ветки wip-local — пропуск" }
    if (-not $Rep.upstreamRemote -or -not $Rep.defaultBranch) { return "нет upstream/default — пропуск" }
    $behind = $Rep.wipLocal.behindBy
    if ($null -eq $behind -or $behind -eq 0) { return "wip-local уже синхронизирован" }
    $upstreamRef = "$($Rep.upstreamRemote)/$($Rep.defaultBranch)"
    if ($DryRun) { return "БУДЕТ: перебазировать 'wip-local' на $upstreamRef (+$behind; локально, без push)" }
    $orig = (Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('symbolic-ref', '--quiet', '--short', 'HEAD')).Out.Trim()
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rebase', $upstreamRef, 'wip-local')
    if (-not $r.Ok) {
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rebase', '--abort') | Out-Null
        if ($orig) { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('checkout', '--quiet', $orig) | Out-Null }
        return "конфликт при синхронизации 'wip-local' → отменено, ветка как была (нужны руки)"
    }
    if ($orig -and $orig -ne 'wip-local') { Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('checkout', '--quiet', $orig) | Out-Null }
    return "wip-local синхронизирована с $upstreamRef (+$behind, без push)"
}

# Run the requested actions for one repo. Backs up first (unless dry-run).
# Prune stale local branches whose upstream tracking is gone (the remote branch was deleted, usually
# after its PR merged). Local-only, backed up by the caller, never pushed. NEVER touches the default
# branch, wip-local, or the currently checked-out branch. Non-dry first refreshes + prunes the fork
# remote so the ': gone' status is accurate.
function Invoke-PruneStale {
    param([string]$RepoPath, [pscustomobject]$Rep, [switch]$DryRun)
    if (-not $DryRun -and $Rep.forkRemote) {
        # A NETWORK fetch — route it through the timed helper (like the delete/force-push calls) so a
        # hung remote can't wedge the prune indefinitely (Invoke-GitLocal has no timeout).
        Invoke-TimedCommand -FilePath 'git' -TimeoutSec 60 -ArgList @('-C',$RepoPath,'fetch','--prune','--quiet',$Rep.forkRemote) | Out-Null
    }
    $vv = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('for-each-ref','--format=%(refname:short) %(upstream:track)','refs/heads')
    if (-not $vv.Ok) { return 'prune: не удалось перечислить ветки' }
    $cur = (Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse','--abbrev-ref','HEAD')).Out
    $protected = @($Rep.defaultBranch, 'wip-local', $cur) | Where-Object { $_ }
    $gone = @()
    foreach ($line in @($vv.Out -split "`n")) {
        if ($line -match '^(\S+)\s+\[gone\]') { if ($Matches[1] -notin $protected) { $gone += $Matches[1] } }
    }
    if (-not $gone.Count) { return 'prune: устаревших веток нет' }
    if ($DryRun) { return "БУДЕТ: удалить устаревшие ветки (upstream удалён): $($gone -join ', ')" }
    $deleted = @()
    foreach ($b in $gone) {
        if ((Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('branch','-D',$b)).Ok) { $deleted += $b }
    }
    if ($deleted.Count) { return "prune: удалены устаревшие ветки ($($deleted -join ', '))" }
    return 'prune: не удалось удалить ветки'
}

# Delete the personal wip-local integration branch when it holds NO unique commits (everything in it
# already landed upstream — uniquePatches == 0). Local-only, backed up by the caller, never pushed.
# Hard guard: refuses if it still has unique work, so we never drop un-merged commits.
function Invoke-DeleteWipLocal {
    param([string]$RepoPath, [pscustomobject]$Rep, [switch]$DryRun)
    if (-not $Rep.wipLocal) { return "нет ветки wip-local — пропуск" }
    $unique = $Rep.wipLocal.uniquePatches
    if ($null -eq $unique) { return "wip-local: не удалось определить уникальные коммиты — пропуск (удали вручную)" }
    if ($unique -gt 0) { return "wip-local: есть $unique своих коммит(ов) — НЕ удаляю (сначала влей/перенеси их)" }
    if ($DryRun) { return "БУДЕТ: удалить локальную 'wip-local' (нет своих коммитов; бэкап создан)" }
    # Never delete the branch we're standing on — hop to the default branch first.
    $cur = (Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('rev-parse','--abbrev-ref','HEAD')).Out
    if ($cur -eq 'wip-local' -and $Rep.defaultBranch) {
        Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('checkout','--quiet',$Rep.defaultBranch) | Out-Null
    }
    $r = Invoke-GitLocal -RepoPath $RepoPath -GitArgs @('branch','-D','wip-local')
    if ($r.Ok) { return "wip-local удалена (локально, без push; нет своих коммитов)" }
    return "ОШИБКА удаления wip-local: $(($r.Out -split "`n")[0])"
}

function Invoke-RepoActions {
    param([pscustomobject]$Rep, [bool]$Ff, [bool]$Del, [bool]$Norm, [bool]$Reb, [bool]$Wip, [bool]$DelWip, [bool]$Prune, [switch]$PushReb, [switch]$DryRun, [switch]$Yes, [switch]$Unattended)
    if ($Rep.Skipped -or $Rep.detached -or $Rep.midOp) { return @() }
    $did = New-Object System.Collections.Generic.List[string]
    if (-not $DryRun -and ($Ff -or $Del -or $Norm -or $Reb -or $Wip -or $DelWip -or $Prune)) {
        New-BackupRefs -RepoPath $Rep.Path | Out-Null
        Remove-OldBackups -RepoPath $Rep.Path
    }
    if ($Ff) { $did.Add((Invoke-FfDefault -RepoPath $Rep.Path -Rep $Rep -DryRun:$DryRun)) }
    if ($Del) {
        foreach ($b in @($Rep.branches | Where-Object { $_.outcome -eq 'merged' })) {
            # A MERGED PR is hard evidence → fork-side delete is allowed and -Yes may
            # auto-confirm. A 'merged' classification WITHOUT a MERGED PR rests only on
            # the cherry heuristic (git cherry is blind to patch-rewriting squashes),
            # so: never push --delete the fork branch on that evidence, and require an
            # explicit confirmation even under -Yes (cherry-only must not be unattended).
            $prConfirmed = ($b.prState -eq 'MERGED')
            if ($DryRun) {
                $did.Add((Remove-MergedBranch -RepoPath $Rep.Path -Branch $b.name -ForkRemote $Rep.forkRemote -DeleteRemote:$prConfirmed -DryRun))
            } elseif ($prConfirmed) {
                if (Confirm-Step "Удалить влитую '$($b.name)' в $($Rep.Name) (локально+форк)?" -Yes:$Yes) {
                    $did.Add((Remove-MergedBranch -RepoPath $Rep.Path -Branch $b.name -ForkRemote $Rep.forkRemote -DeleteRemote))
                } else { $did.Add("пропущено '$($b.name)'") }
            } elseif ($Unattended) {
                # M6: a cherry-heuristic-only delete needs interactive confirmation (never auto, even
                # under -Yes). In an unattended run there's no console — Confirm-Step's Read-Host gets
                # $null at EOF and ($null).Trim() THROWS, aborting the whole run before the status is
                # written (ErrorActionPreference='Stop', no try/catch here). Skip it safely instead.
                $did.Add("пропущено '$($b.name)' (эвристика cherry без влитого PR — требует интерактивного подтверждения)")
            } else {
                # Cherry-heuristic-only: local delete, no fork push, ALWAYS prompt (ignore -Yes).
                if (Confirm-Step "Удалить '$($b.name)' в $($Rep.Name) ЛОКАЛЬНО? (нет влитого PR — только эвристика cherry; форк не трогаем)") {
                    $did.Add((Remove-MergedBranch -RepoPath $Rep.Path -Branch $b.name -ForkRemote $Rep.forkRemote))
                } else { $did.Add("пропущено '$($b.name)' (эвристика cherry без влитого PR — требует подтверждения)") }
            }
        }
    }
    if ($Reb) {
        $upstreamRef = "$($Rep.upstreamRemote)/$($Rep.defaultBranch)"
        if ($Rep.dirty -and -not $DryRun) {
            $did.Add("rebase пропущен: грязное дерево (tracked) — сначала закоммить/спрячь")
        } else {
            foreach ($b in @($Rep.branches | Where-Object { $_.outcome -eq 'clean' })) {
                if ($DryRun) { $did.Add((Invoke-RebaseBranch -RepoPath $Rep.Path -Branch $b.name -UpstreamRef $upstreamRef -ForkRemote $Rep.forkRemote -DryRun)) }
                else { $did.Add((Invoke-RebaseBranch -RepoPath $Rep.Path -Branch $b.name -UpstreamRef $upstreamRef -ForkRemote $Rep.forkRemote -Push:$PushReb -Yes:$Yes)) }
            }
        }
    }
    if ($Wip) {
        if ($Rep.dirty -and -not $DryRun) {
            $did.Add("sync wip-local пропущен: грязное дерево (tracked) — сначала закоммить/спрячь")
        } else {
            $did.Add((Invoke-SyncWipLocal -RepoPath $Rep.Path -Rep $Rep -DryRun:$DryRun))
        }
    }
    if ($DelWip) { $did.Add((Invoke-DeleteWipLocal -RepoPath $Rep.Path -Rep $Rep -DryRun:$DryRun)) }
    if ($Prune) { $did.Add((Invoke-PruneStale -RepoPath $Rep.Path -Rep $Rep -DryRun:$DryRun)) }
    if ($Norm) { $did.Add((Invoke-NormalizeRemotes -RepoPath $Rep.Path -Rep $Rep -DryRun:$DryRun)) }
    return $did.ToArray()
}

# Summary counters over a report set. Called twice: for the console box right after the analysis,
# and again after the action phase — so the JSON envelope's counts describe the repos as they are
# NOW, not as they were before fork-sync mutated them.
function Get-ForkSyncCounts {
    param($Reports)
    # .Where() intrinsics + explicit flatten instead of @(...) subexpressions: the @() binder throws
    # "Argument types do not match" on a generic List in PS7 interpreted mode (cf. PowerShell #8661).
    $managed = @($Reports.Where({ -not $_.Skipped }))
    $branches = [System.Collections.Generic.List[object]]::new()
    foreach ($m in $managed) { foreach ($br in $m.branches) { if ($br) { $branches.Add($br) } } }
    $conflict = $branches.Where({ $_.outcome -eq 'conflict' }).Count
    return [pscustomobject]@{
        Managed   = $managed
        Merged    = $branches.Where({ $_.outcome -eq 'merged' }).Count
        Open      = $branches.Where({ $_.outcome -eq 'clean' }).Count
        Conflict  = $conflict
        NeedHands = $managed.Where({ $_.midOp -or $_.detached }).Count + $conflict
    }
}

# Re-analyze ONE repo after it was mutated and swap the fresh report into $Reports, which is what
# the envelope is written from. Without this the UI keeps the "needs sync" recommendation for a
# repo that was JUST synced. Shared by the flag path and the interactive menu — the menu used to
# mutate without any refresh and overwrite the shared fork-sync.last.json with the stale snapshot.
# -NoFetch: the actions ff/rebase onto the already-fetched upstream, so no new network fetch.
function Sync-ReportAfterActions {
    param($Rep, $Acts, $Config, [bool]$GhAvailable, $Reports)
    try {
        $fresh = Get-RepoReport -RepoPath $Rep.Path -Config $Config -NoFetch -GhAvailable:$GhAvailable -IsOwn:([bool]$Rep.isOwn)
    } catch {
        # The mutation already happened on disk; a re-analysis failure must NOT abort the run
        # ($ErrorActionPreference='Stop') and discard every repo's status. Fall back to the
        # pre-action report + actionsTaken (mirrors the guarded initial analysis pass).
        Write-Status "$($Rep.Path): повторный анализ после действий не удался — $($_.Exception.Message)" 'FAIL'
        $fresh = $null
    }
    if (-not $fresh -or $fresh.Skipped) {
        $Rep | Add-Member -NotePropertyName 'actionsTaken' -NotePropertyValue $Acts -Force
        return
    }
    $fresh | Add-Member -NotePropertyName 'actionsTaken' -NotePropertyValue $Acts -Force
    $ri = $Reports.IndexOf($Rep)
    if ($ri -ge 0) { $Reports[$ri] = $fresh } else { $Rep | Add-Member -NotePropertyName 'actionsTaken' -NotePropertyValue $Acts -Force }
}

function Invoke-ForkSync {
    [CmdletBinding()]
    param(
        [string]$Root = $PSScriptRoot,
        [switch]$Unattended,
        [switch]$NoFetch,
        [string[]]$Roots,
        [string[]]$Paths,
        [int]$FetchTimeoutSec,
        [int]$GhTimeoutSec,
        [switch]$Apply,
        [switch]$FfMain,
        [switch]$DeleteMerged,
        [switch]$NormalizeRemotes,
        [switch]$Rebase,
        [switch]$SyncWipLocal,
        [switch]$DeleteWip,
        [switch]$Prune,
        [switch]$PushRebased,
        [switch]$DryRun,
        [switch]$Yes,
        # Strict single-repo mode: process ONLY this path (ignore roots/own config) and write the
        # result to -OutFile. Lets Castellyn run repos concurrently (per-repo lock + per-repo JSON).
        [string]$Single,
        [string]$OutFile,
        # Durable fork config (Castellyn %APPDATA%\castellyn\forks.json); overrides vendored repos.json.
        [string]$ConfigPath
    )
    $start = Get-Date
    $log = Initialize-Logging -Root $Root -Prefix 'fork-sync'
    $acting = ($Apply -or $FfMain -or $DeleteMerged -or $NormalizeRemotes -or $Rebase -or $SyncWipLocal -or $DeleteWip -or $Prune)
    $modeLabel = if ($acting -and $DryRun) { 'dry-run: показ плана' } elseif ($acting) { 'APPLY: внесение изменений' } elseif ($NoFetch) { 'read-only (no fetch)' } elseif ($Unattended) { 'read-only unattended' } else { 'read-only' }
    Write-Banner "fork-sync — статус форков  ($modeLabel)" "log: $log" -Width 64

    # Pre-flight
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Write-Status 'git не найден на PATH.' 'FAIL'; return 2 }
    $ghCmd = Get-Command gh -ErrorAction SilentlyContinue
    $ghAvailable = $false
    if ($ghCmd) {
        $auth = Invoke-TimedCommand -FilePath 'gh' -TimeoutSec 20 -ArgList @('auth', 'status')
        $ghAvailable = $auth.Ok
    }
    if ($ghAvailable) { Write-Status 'gh авторизован — статусы PR будут точными.' 'OK' }
    else { Write-Status 'gh недоступен/без авторизации — роли по эвристике, статусы PR = unknown.' 'WARN' }

    $cfg = Get-ForkSyncConfig -Root $Root -Roots $Roots -Paths $Paths -FetchTimeoutSec $FetchTimeoutSec -GhTimeoutSec $GhTimeoutSec -ConfigPath $ConfigPath
    $repos = Find-ManagedRepos -Roots $cfg.Roots -Paths $cfg.Paths
    $ownRepos = @(Find-ManagedRepos -Paths $cfg.OwnPaths | Where-Object { $_ -notin $repos })
    if ($Single) {
        # Override discovery: only the one requested repo. Route it to fork/own based on OwnPaths so
        # its role (and the -IsOwn report) stays correct.
        $rp = if ((Test-Path -LiteralPath $Single) -and (Test-Path -LiteralPath (Join-Path $Single '.git'))) { (Resolve-Path -LiteralPath $Single).Path } else { $null }
        $isOwnSingle = $false
        foreach ($o in $cfg.OwnPaths) { if ((Test-Path -LiteralPath $o) -and ((Resolve-Path -LiteralPath $o).Path -eq $rp)) { $isOwnSingle = $true } }
        if ($rp -and $isOwnSingle) { $repos = @(); $ownRepos = @($rp) }
        elseif ($rp) { $repos = @($rp); $ownRepos = @() }
        else { $repos = @(); $ownRepos = @() }
    }
    Write-Status ("найдено: форков {0}, своих репо {1}" -f @($repos).Count, @($ownRepos).Count) 'INFO'

    $reports = New-Object System.Collections.Generic.List[object]
    foreach ($rp in $repos) {
        try { $reports.Add((Get-RepoReport -RepoPath $rp -Config $cfg -NoFetch:$NoFetch -GhAvailable:$ghAvailable)) }
        catch { Write-Status "$rp : ошибка анализа — $($_.Exception.Message)" 'FAIL'; $reports.Add([pscustomobject]@{ Name = (Split-Path $rp -Leaf); Path = $rp; Skipped = 'error' }) }
    }
    foreach ($rp in $ownRepos) {
        try { $reports.Add((Get-RepoReport -RepoPath $rp -Config $cfg -NoFetch:$NoFetch -GhAvailable:$ghAvailable -IsOwn)) }
        catch { Write-Status "$rp : ошибка анализа — $($_.Exception.Message)" 'FAIL'; $reports.Add([pscustomobject]@{ Name = (Split-Path $rp -Leaf); Path = $rp; Skipped = 'error' }) }
    }

    $repoTotal = $reports.Count; $repoIdx = 0
    foreach ($rep in $reports) { $repoIdx++; Write-RepoHuman -Rep $rep -Index $repoIdx -Total $repoTotal }

    $sum = Get-ForkSyncCounts -Reports $reports
    $managed = $sum.Managed
    $cMerged = $sum.Merged; $cConf = $sum.Conflict; $cOpen = $sum.Open; $needHands = $sum.NeedHands
    $dur = (Get-Date) - $start
    $durStr = '{0:m\:ss}' -f $dur

    # Rich framed summary with tee dividers (mirrors build_portable.ps1 final block).
    $sumColor = if ($cConf -gt 0) { 'Red' } else { 'Green' }
    $sumBar   = $script:SK_H * 64
    Write-Host ''
    Write-Host "  $($script:SK_TL)$sumBar$($script:SK_TR)" -ForegroundColor $sumColor
    Write-Host "  $($script:SK_V)  Итог  --  $($managed.Count) репо  --  $durStr" -ForegroundColor $sumColor
    Write-Host "  $($script:SK_TM)$sumBar" -ForegroundColor $sumColor
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'репозиториев', $managed.Count) -ForegroundColor White
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'влито',        $cMerged)       -ForegroundColor White
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'открыто',      $cOpen)         -ForegroundColor White
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'конфликты',    $cConf)         -ForegroundColor $(if ($cConf -gt 0) { 'Red' } else { 'White' })
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'нужны руки',   $needHands)     -ForegroundColor $(if ($needHands -gt 0) { 'Yellow' } else { 'White' })
    Write-Host ("  $($script:SK_V)  {0,-12} {1}" -f 'время',        $durStr)        -ForegroundColor White
    Write-Host "  $($script:SK_BL)$sumBar$($script:SK_BR)" -ForegroundColor $sumColor
    Write-SkLog -Msg ("Итог: {0} репо | влито {1}, открыто {2}, конфликтов {3}, нужны руки {4} | {5}" -f $managed.Count, $cMerged, $cOpen, $cConf, $needHands, $durStr) -Level 'INFO' -NoConsole

    # --- Action phase (Phase 2: safe mutations) ---
    $mutated = $false   # set by either action path; drives the post-phase counter refresh
    $doFf = [bool]($Apply -or $FfMain); $doDel = [bool]($Apply -or $DeleteMerged); $doNorm = [bool]$NormalizeRemotes; $doReb = [bool]$Rebase; $doWip = [bool]$SyncWipLocal; $doDelWip = [bool]$DeleteWip; $doPrune = [bool]$Prune
    if ($doFf -or $doDel -or $doNorm -or $doReb -or $doWip -or $doDelWip -or $doPrune) {
        $dry = [bool]$DryRun
        Write-Section $(if ($dry) { 'ПЛАН действий (dry-run — ничего не меняется)' } else { 'Выполнение действий' }) '' $(if ($dry) { 'Magenta' } else { 'Green' })
        foreach ($rep in $managed) {
            $acts = Invoke-RepoActions -Rep $rep -Ff:$doFf -Del:$doDel -Norm:$doNorm -Reb:$doReb -Wip:$doWip -DelWip:$doDelWip -Prune:$doPrune -PushReb:$PushRebased -DryRun:$dry -Yes:$Yes -Unattended:$Unattended
            if (@($acts).Count) {
                Write-Host "    $($rep.Name):" -ForegroundColor White
                foreach ($a in $acts) { Write-Status $a $(if ($dry) { 'INFO' } else { 'OK' }) }
                if ($dry) {
                    $rep | Add-Member -NotePropertyName 'actionsPlanned' -NotePropertyValue $acts -Force
                } else {
                    $mutated = $true
                    Sync-ReportAfterActions -Rep $rep -Acts $acts -Config $cfg -GhAvailable $ghAvailable -Reports $reports
                }
            }
        }
    } elseif (-not $Unattended) {
        $menu = $true
        while ($menu) {
            Write-Host ''
            Write-Host '  Что сделать?' -ForegroundColor Cyan
            Write-Host '    [P] Показать план (ничего не меняя)'
            Write-Host '    [F] Обновить main у отставших (fast-forward)'
            Write-Host '    [D] Удалить влитые ветки (спросит по каждой)'
            Write-Host '    [N] Выровнять имена remote к канону'
            Write-Host '    [R] Перебазировать открытые ветки на свежий оригинал (локально)'
            Write-Host '    [W] Синхронизировать wip-local с оригиналом (локально, без push)'
            Write-Host '    [S] Выход'
            $c = (Read-Host '    Выбор [P/F/D/N/R/W/S]').Trim().ToUpper()
            if ($c -eq 'S') { $menu = $false; continue }
            $plan = ($c -eq 'P')
            $ff = ($c -eq 'F' -or $plan); $del = ($c -eq 'D' -or $plan); $norm = ($c -eq 'N' -or $plan); $reb = ($c -eq 'R' -or $plan); $wip = ($c -eq 'W' -or $plan)
            if (-not ($ff -or $del -or $norm -or $reb -or $wip)) { Write-Host '    ? неизвестный выбор' -ForegroundColor Yellow; continue }
            foreach ($rep in $managed) {
                $acts = Invoke-RepoActions -Rep $rep -Ff:$ff -Del:$del -Norm:$norm -Reb:$reb -Wip:$wip -PushReb:$false -DryRun:$plan -Yes:$Yes
                if (@($acts).Count) {
                    Write-Host "    $($rep.Name):" -ForegroundColor White
                    foreach ($a in $acts) { Write-Status $a $(if ($plan) { 'INFO' } else { 'OK' }) }
                    if (-not $plan) {
                        $mutated = $true
                        Sync-ReportAfterActions -Rep $rep -Acts $acts -Config $cfg -GhAvailable $ghAvailable -Reports $reports
                    }
                }
            }
            # Refresh the working set so the NEXT menu round acts on the post-mutation reports
            # instead of re-offering work that the previous round already did.
            if ($mutated) { $managed = (Get-ForkSyncCounts -Reports $reports).Managed }
        }
    }

    # Recompute after the action phase: the envelope below must report the repos as they are now.
    # Both action paths mutate, so counts taken before the phase would describe the pre-action world.
    if ($mutated) {
        $sum = Get-ForkSyncCounts -Reports $reports
        $managed = $sum.Managed
        $cMerged = $sum.Merged; $cConf = $sum.Conflict; $cOpen = $sum.Open; $needHands = $sum.NeedHands
    }

    # Unified status envelope (schemaVersion/component/status/counts/durationSec) is
    # ADDITIVE -- the legacy generatedAt/mode/summary/repos fields below are kept so
    # the max.fork-sync skill still parses. counts.failed = "needs hands" (conflicts
    # + mid-operation), counts.changed = merged branches.
    $skSkipErr = $reports.Where({ $_.Skipped -eq 'error' }).Count
    $skEnvStatus = if ($skSkipErr -gt 0) { 'error' } elseif ($needHands -gt 0 -or $cMerged -gt 0) { 'changes' } else { 'ok' }
    $payload = [ordered]@{
        schemaVersion = 1; component = 'forks'; status = $skEnvStatus
        counts = [ordered]@{ changed = $cMerged; failed = $needHands; total = $managed.Count }
        timestamp = (Get-Date -Format 'o'); durationSec = [math]::Round($dur.TotalSeconds, 1)
        generatedAt = (Get-Date -Format 'o'); mode = $modeLabel
        gitVersion = ((Invoke-GitLocal -RepoPath $Root -GitArgs @('--version')).Out)
        ghAvailable = $ghAvailable
        summary = [ordered]@{ repos = $managed.Count; merged = $cMerged; open = $cOpen; conflict = $cConf; needHands = $needHands }
        repos = $reports.ToArray()
    }
    $jsonPath = Write-ForkSyncJson -Root $Root -Payload $payload -OutFile $OutFile
    Write-Status "JSON: $jsonPath" 'INFO'
    if (-not $Unattended) { Show-Notification -Title 'fork-sync' -Body ("Влито {0}, открыто {1}, конфликтов {2}" -f $cMerged, $cOpen, $cConf) -IsError:($cConf -gt 0) }

    if ($skSkipErr -gt 0) { return 1 } # reuse the count computed above; $reports is unchanged since
    return 0
}
# endregion

Export-ModuleMember -Function *-* -Variable @()
