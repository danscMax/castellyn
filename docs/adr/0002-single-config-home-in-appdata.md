# One config home: everything Castellyn owns lives in %APPDATA%\castellyn

> **STATUS: ON HOLD — do not implement from this document.** 2026-07-20, same day it was written.
> Two later findings changed the ground under it: (1) `Backup-ClaudeSetup.ps1:91-132` is a fourth
> writer to the config directory and treats it as the sync channel; (2) `Manage-Engine`,
> `Manage-Provider`, `Connect-Router` and `Manage-OpenCode-Provider` are already ported to native
> Rust (`lib.rs:4725, 4995, 5476, 7962`), so the settled direction is to finish porting the profile
> scripts first — after which PowerShell stops reading `profiles.json` and this decision becomes
> nearly free. The owner chose that sequencing explicitly. Revisit this document only after the
> port lands. Rationale below still holds; the timing and the exact target path do not.

Every configuration file Castellyn reads or writes moves to `%APPDATA%\castellyn\` —
`profiles.json`, `.mcp.json`, `engines.json`, `myproviders.json`, `sync-config.json`
and their siblings. Files that must reach the user's other machines go one level down,
in `%APPDATA%\castellyn\shared\`, and that subfolder — not the whole home — is what
Syncthing replicates; machine-local state (`config.json`: theme, window, autostart)
stays beside it and never travels. No file Castellyn owns is addressed relative to
`SCRIPTS_ROOT` any more.

## Why

The files used to live under the shared settings folder (`<SCRIPTS_ROOT>\SettingsMCP\
ClaudeProfiles\config\`) because they have three readers, not one: Castellyn, the
PowerShell scripts, and the second machine via Syncthing (which replicates
`E:\Scripts`, not `%APPDATA%`). That location is a consequence of Castellyn having
grown as a GUI over an existing script collection, not of a deliberate design.

It breaks the moment a user has no such folder — i.e. every user who is not the
author. `profiles.json` is addressed as `{{PROFILES}}\config\profiles.json`
(`lib.rs:2367`) and `read_profiles()` (`lib.rs:2063`) reads a status envelope only a
PowerShell script writes, so on a bare machine the profile surface is permanently
empty and the profile a user creates cannot be declared anywhere.

Keeping the old location and adding a fallback for newcomers was considered and
rejected: it produces two homes, a merge question the day a newcomer installs a
settings folder, and merge code that runs once in a user's lifetime and would
therefore be the least-exercised path in the product.

## Considered alternatives

- **Fallback address for newcomers only** (`%APPDATA%` when no settings folder
  exists, the shared folder otherwise). Preserves the author's setup untouched, but
  makes "where does this file live" a runtime question forever, and requires a
  one-time move-or-ask flow when a settings folder later appears. Rejected: the
  complexity is permanent, the benefit is one user's convenience.
- **Castellyn keeps its own registry alongside the scripts' one.** Two lists of the
  same thing, with a merge on first contact. Rejected for the same reason the five
  hand-rolled confirm gates were consolidated in v0.7.2 — divergent copies of one
  concept are the defect class this codebase has already paid for once.
- **Leave it and accept the newcomer gap.** Rejected: it makes the whole
  newcomer-facing stage of the roadmap rest on a surface that cannot work without
  the author's private folder tree.

## Consequences

- The move is a **coordinated change across two repositories** — Castellyn and the
  maintenance scripts. Shipping it in halves reintroduces exactly the two-homes state
  it removes, so it lands as one step, before any newcomer-facing work.
- Syncthing needs `%APPDATA%\castellyn` configured as a synced folder. Until it is,
  the second machine stops receiving configuration changes. This is a manual step on
  each machine and must be part of the step's definition of done, not assumed.
- A one-time migration reads the old location and writes the new one; the old files
  are left in place, not deleted, so a rollback to a previous Castellyn build still
  finds them.
- Castellyn's own data survives the loss or rename of `SCRIPTS_ROOT`. Losing the
  drive now costs access to the maintenance scripts only, not the profile, MCP and
  provider configuration.
- `HubConfig.scriptsRoot` keeps its meaning — where the maintenance scripts are — and
  stops being the root that every owned file hangs off.
