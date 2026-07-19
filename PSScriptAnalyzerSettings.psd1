@{
    # Single source of truth for the PowerShell static-analysis gate. Read by verify.ps1 (local) and
    # by ci.yml / release.yml (CI) so local and CI can never disagree.
    #
    # Both severities on purpose: the previous CI invocation passed `-Severity Warning`, which filters
    # to EXACTLY that level — a PSScriptAnalyzer *Error* would have sailed through the gate.
    Severity = @('Error', 'Warning')

    ExcludeRules = @(
        # This repo's PowerShell layer is a set of interactive maintenance scripts whose whole job is
        # printing progress to a console the user is watching. Write-Host is the correct tool there;
        # Write-Output would pollute the pipeline the callers actually consume.
        'PSAvoidUsingWriteHost'

        # UTF-8 without BOM is the project convention (see CLAUDE.md). It would only corrupt Cyrillic
        # literals under Windows PowerShell 5.1, and every repo script the backend spawns goes through
        # `pwsh` 7.x, which decodes BOM-less UTF-8 correctly. The two `powershell.exe` (5.1) call sites
        # in lib.rs run scripts from SCRIPTS_ROOT, not from this repo.
        'PSUseBOMForUnicodeEncodedFile'

        # Plural nouns in function names (Get-ForkRepos, Sync-Profiles) read better for commands that
        # genuinely act on collections, and renaming them now would break every call site + the docs.
        'PSUseSingularNouns'

        # False positive on THIS runtime, verified rather than assumed: the rule flags ScriptKit's
        # `Write-Log` as shadowing a built-in, but `Get-Command Write-Log` is empty in a clean
        # pwsh 7.6.2 session and Microsoft.PowerShell.Utility ships no such cmdlet — the rule matches
        # an inventory compiled into PSScriptAnalyzer ("core-6.1.0-windows"), not the live shell.
        # Renaming would break 40 call sites across three repositories that vendor ScriptKit.ps1
        # (Backup-ClaudeSetup.ps1, update-plugins.ps1, fork-updater/ForkSync.psm1) to fix nothing.
        # Excluded because the reported defect does not exist here — NOT to make a red gate go green.
        # Re-evaluate if a future PowerShell actually ships Write-Log.
        'PSAvoidOverwritingBuiltInCmdlets'
    )
}
