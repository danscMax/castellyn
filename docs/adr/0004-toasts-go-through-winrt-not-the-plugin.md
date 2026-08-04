# Toasts are built on WinRT directly; the plugin becomes the fallback

Every OS notification now leaves through one module, `src-tauri/src/notify.rs`, which talks to the
Windows notification platform itself. `tauri-plugin-notification` stays registered, but only as the
path taken when the direct channel cannot show anything.

## Why not stay on the plugin

The plugin's builder exposes exactly four things: title, body, icon, sound. Three consequences made
it the wrong foundation for what Sessions needs:

| Need | Plugin | Direct WinRT |
| --- | --- | --- |
| One live notice per session (a newer one **replaces** the older) | impossible — no tag, no group | `SetTag` + `SetGroup` |
| Click the toast → jump to the pane that is waiting | no callback at all | `Activated` event |
| Buttons on the toast | none | `<actions>` in the payload |
| "Someone is waiting on you" must not fade after 7 s | no scenario control | `scenario="reminder"` |
| Know that Windows has muted us | not exposed | `ToastNotifier::Setting()` |

The crate underneath the plugin, `tauri-winrt-notification`, does not close the gap either: its only
`SetTag` call is reachable exclusively for progress bars, `SetGroup` is never called, and the
`ToastNotification` object is behind a private constructor, so the tag/group cannot be set from
outside. Replacement semantics were simply unavailable without going one layer down.

## The identity problem this also fixes

Toasts used to be signed **"Windows PowerShell"**, with PowerShell's icon, and were grouped under
PowerShell in the Action Center. That was not cosmetic: `tauri-plugin-notification` sets an
AppUserModelID only when the executable does **not** live in `target\debug` or `target\release`
(`desktop.rs:201-204`), and this project's own release binary lives exactly there
(`build_all.ps1:101`). With no AppUserModelID, `notify-rust` falls back to PowerShell's.

The direct channel always signs with `com.danscmax.castellyn`.

## The risk, and why the fallback exists

Windows shows a toast only for a **registered** AppUserModelID — normally registered by a Start Menu
shortcut. An installed build has one; a standalone `castellyn.exe` started from a hand-made desktop
shortcut may not. When the identity is unknown the platform drops the toast **without an error**:
`Show()` returns `Ok`.

So the app registers the identity itself on startup: it plants a Start Menu shortcut carrying
`PKEY_AppUserModel_ID` (idempotent — an existing shortcut, including the installer's, is left
alone). Verified live: before it, `CreateToastNotifierWithId` returned 0x80070490 and nothing was
shown; after it, Windows created its own registry entry for the identity and the toast appeared
under the name **Castellyn** with our icon.

Two safeguards rather than blind trust:

- `notify_diag` reports, in words, what the registration did — a release build is a GUI subsystem
  binary with **no console**, so `eprintln!` reaches nobody and this is the only readable answer;
- any failure on the direct path falls back to the plugin and records the reason, so the worst case
  is the notification everyone had before, never silence.

**`ToastNotifier::Setting()` is not a health check.** It reports the user's permission, and it
answered `Enabled` in a state where every toast was being dropped for an unregistered identity —
`Show()` returning `Ok` all the while. It is worth surfacing ("Windows muted this app") but it must
never be read as "the channel works".

## Consequences

- Windows-only by construction. The project is already Windows-only in several load-bearing places
  (Job Objects for PTY trees, junctions for profile links, `MessageBeep`, the credential store), so
  this adds no new platform constraint; the non-Windows build keeps a stub that reports the direct
  channel as unavailable and therefore always uses the plugin.
- No new dependency: the `windows` crate is already a direct dependency; three feature flags were
  added (`UI_Notifications`, `Data_Xml_Dom`, `Foundation`).
- Notification payloads are XML, so every interpolated string is escaped. Pane labels are
  user-controlled — a project folder containing `&` would otherwise make the document fail to parse
  and the notification vanish. This is pinned by a unit test.
