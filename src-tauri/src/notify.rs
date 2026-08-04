//! The single exit point for every OS notification.
//!
//! Before this module there were three independent senders — agent status transitions, the usage
//! limit alert and `notify_important` (stack down / run failed / schedule failed). Each re-read the
//! config, each called the plugin itself, and none knew about the others, so three events in a row
//! produced three separate toasts and one wrong AppUserModelID mis-signed all of them at once. This
//! is to notifications what `pump_and_wait` is to streaming: the one path everything goes through.
//!
//! **Why WinRT directly instead of `tauri-plugin-notification`.** The plugin exposes only
//! title/body, and the crate beneath it cannot set a notification's TAG or GROUP at all (its single
//! `SetTag` call is reachable only for progress bars). Without a tag a second notice about the same
//! session stacks on top of the first instead of replacing it. Going at WinRT directly also buys
//! the click callback, action buttons and the reminder scenario. See `docs/adr/0004`.
//!
//! The plugin stays in the project as the FALLBACK: if this channel cannot show a toast (an
//! unregistered AppUserModelID is the classic cause — Windows then silently drops the toast) we
//! fall back rather than going quiet, and say so in the UI.

use tauri::AppHandle;

/// What a notification is about. Drives the tag (so same-subject notices replace each other), the
/// scenario (only "someone is waiting on you" earns a toast that will not fade), and whether the
/// toast carries a jump-to button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// An agent stopped and needs a human answer.
    Blocked,
    /// SEVERAL agents are waiting — one notice standing in for all of them. Sticky like `Blocked`,
    /// but it must be free to update as the count moves, so it follows the plain cooldown instead
    /// of the escalation window (which is about not re-nagging for ONE pane).
    BlockedMany,
    /// An agent finished its turn.
    Done,
    /// A session is parked until its usage window resets.
    Limited,
    /// Background maintenance: stack down, run failed, schedule failed, long run finished.
    Important,
}

impl Kind {
    /// A reminder-scenario toast stays on screen until dismissed. Reserved for the one state that
    /// actually blocks the user's work — everything else may fade on its own.
    fn sticky(self) -> bool {
        matches!(self, Kind::Blocked | Kind::BlockedMany)
    }
    /// Toasts are grouped per kind so a burst of "finished" notices cannot bury a "waiting" one.
    fn group(self) -> &'static str {
        match self {
            Kind::Blocked | Kind::BlockedMany => "blocked",
            Kind::Done => "done",
            Kind::Limited => "limited",
            Kind::Important => "important",
        }
    }
}

/// One notification to show.
pub struct Notice {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// What this notice is ABOUT — a session id, a profile name. Used as the toast tag, so a newer
    /// notice on the same subject REPLACES the older one instead of stacking. Kept separate from
    /// `session` on purpose: a usage-limit alert is per-profile and wants replacement, but there is
    /// no pane to jump to, and conflating the two silently left those alerts untagged.
    pub tag: Option<String>,
    /// The pane to focus when the toast is clicked, when there is one. Drives the button and the
    /// activation payload; `None` means the notice carries neither.
    pub session: Option<String>,
}

/// XML escaping for text that reaches the toast payload. Session labels are user-controlled (a pane
/// can be renamed to anything, a project folder can contain `&`), and an unescaped `&` makes the
/// whole XML document fail to parse — which would silently kill the notification.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Build the toast XML. Kept pure and separate from the WinRT calls so the escaping and the
/// scenario/button decisions are unit-testable without a Windows notification platform.
fn toast_xml(kind: Kind, title: &str, body: &str, session: Option<&str>, goto_label: &str) -> String {
    let scenario = if kind.sticky() {
        r#" scenario="reminder""#
    } else {
        ""
    };
    // `launch` is what the click hands back; the jump only makes sense when we know the session.
    let launch = match session {
        Some(id) => format!(r#" launch="goto:{}""#, esc(id)),
        None => String::new(),
    };
    let actions = match session {
        Some(id) => format!(
            r#"<actions><action content="{}" arguments="goto:{}" activationType="foreground"/></actions>"#,
            esc(goto_label),
            esc(id)
        ),
        None => String::new(),
    };
    format!(
        r#"<toast{scenario}{launch}><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>{actions}</toast>"#,
        esc(title),
        esc(body)
    )
}

/// The AppUserModelID the toasts are signed with. Same string as the bundle identifier, so an
/// installed build reuses the shortcut the installer already registered.
#[cfg(windows)]
const AUMID: &str = "com.danscmax.castellyn";

/// Is the rich channel usable at all? Set once at startup from the registration result.
///
/// This exists because `Show()` is NOT a reliable signal: with an unregistered identity Windows
/// drops the toast and still returns `Ok`, so an error-triggered fallback would never fire and the
/// user would get nothing — no toast, no error. Registration, on the other hand, is knowable, and
/// it is exactly the precondition the platform enforces. If it failed, every notification goes
/// through the plugin from the start.
#[cfg(windows)]
static CHANNEL_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Make Windows aware of our AppUserModelID by planting a Start Menu shortcut that carries it.
///
/// This is not cosmetic plumbing — it is the precondition for the entire channel. Windows only
/// shows a toast for an identity it knows, and it learns one from a shortcut with
/// `PKEY_AppUserModel_ID`; without it `CreateToastNotifierWithId` fails with ERROR_NOT_FOUND
/// (0x80070490) — measured live, not assumed. An installed build already has such a shortcut from
/// the installer; a standalone `castellyn.exe` (which is how this project is normally run — see
/// `build_all.ps1`) does not, so we plant one ourselves on first start.
///
/// Deliberately best-effort: any failure just leaves the channel unavailable, and `notify()` then
/// falls back to the plugin. Never blocks startup, never reports an error to the user.
#[cfg(windows)]
pub fn ensure_registered() {
    // Report the outcome instead of swallowing it: a missing shortcut means Windows silently drops
    // every toast (Show() still returns Ok), which is the hardest possible failure to notice.
    let ok = match ensure_registered_inner() {
        Ok(msg) => {
            eprintln!("notify: AUMID registration — {msg}");
            true
        }
        Err(e) => {
            eprintln!("notify: AUMID registration FAILED — {e}");
            false
        }
    };
    CHANNEL_READY.store(ok, std::sync::atomic::Ordering::Relaxed);
}

/// The AppUserModelID already stored on a shortcut, if any. `None` covers "no such property" just
/// as much as "could not read the file" — both mean the same thing to the caller: not ours yet.
#[cfg(windows)]
fn shortcut_aumid(path: &std::path::Path) -> Option<String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};
    use std::os::windows::ffi::OsStrExt;

    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // Read-only: we only inspect the property.
        file.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let store: IPropertyStore = link.cast().ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID as *const _).ok()?;
        let s = value.to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

#[cfg(windows)]
fn ensure_registered_inner() -> Result<String, String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, IPersistFile,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_CREATE};
    use windows::core::{HSTRING, Interface, PCWSTR};
    use std::os::windows::ffi::OsStrExt;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    unsafe {
        // The main thread is already an STA under Tauri; a second init returns RPC_E_CHANGED_MODE,
        // which is harmless here — we only need SOME apartment, so the result is ignored.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        // KF_FLAG_CREATE, not DEFAULT: the folder is only guaranteed to EXIST on a normal profile.
        // Measured live — with a redirected APPDATA (the isolated test world) DEFAULT fails with
        // 0x80070003 "path not found", and the whole channel silently degrades from there.
        let dir = match SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_CREATE, None) {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                // The shell allocates this buffer with CoTaskMemAlloc and hands ownership over;
                // `PWSTR` is a bare pointer with no Drop, so without this it leaks every call.
                CoTaskMemFree(Some(p.0 as *const std::ffi::c_void));
                s
            }
            Err(_) => String::new(),
        };
        // Last resort: compose the path from APPDATA ourselves. Shell folder lookup can fail on an
        // unusual profile, and a notification identity is too load-bearing to give up on that.
        let dir = if dir.is_empty() {
            let base = std::env::var("APPDATA").map_err(|_| "no APPDATA and no shell folder")?;
            let p = std::path::Path::new(&base)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            std::fs::create_dir_all(&p).map_err(|e| format!("create Programs dir: {e}"))?;
            p.to_string_lossy().to_string()
        } else {
            dir
        };
        let lnk = std::path::Path::new(&dir).join("Castellyn.lnk");
        // A shortcut being PRESENT is not the same as it carrying our identity. The installer
        // creates one too, and an NSIS shortcut has no AppUserModelID — trusting mere existence
        // would skip registration on exactly the machines that install the app properly, and every
        // toast would be dropped with nothing to explain why. So read the property and only leave
        // the file alone when it already says what we need.
        if lnk.exists() && shortcut_aumid(&lnk).as_deref() == Some(AUMID) {
            return Ok(format!("already registered: {}", lnk.display()));
        }

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("CoCreateInstance(ShellLink): {e}"))?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))
            .map_err(|e| format!("SetPath: {e}"))?;
        if let Some(parent) = exe.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(parent.as_os_str()));
        }
        // The identity itself: this property on the shortcut is what teaches Windows the AUMID.
        let store: IPropertyStore = link.cast().map_err(|e| format!("cast IPropertyStore: {e}"))?;
        let pv = PROPVARIANT::from(AUMID);
        // Raw pointers: this binding takes the key and value as `*const`.
        store
            .SetValue(&PKEY_AppUserModel_ID as *const _, &pv as *const _)
            .map_err(|e| format!("SetValue(PKEY_AppUserModel_ID): {e}"))?;
        store.Commit().map_err(|e| format!("property Commit: {e}"))?;

        let file: IPersistFile = link.cast().map_err(|e| format!("cast IPersistFile: {e}"))?;
        let wide: Vec<u16> = lnk
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        file.Save(PCWSTR(wide.as_ptr()), true)
            .map_err(|e| format!("IPersistFile::Save: {e}"))?;
        Ok(format!("registered {AUMID} via {}", lnk.display()))
    }
}

#[cfg(not(windows))]
pub fn ensure_registered() {}

/// Whether Windows currently allows this app to show toasts, as the platform sees it — the only
/// honest answer to "why did no notification appear". Surfaced in Settings so a user who muted
/// Castellyn in Windows sees why it went quiet instead of assuming the feature is broken.
#[cfg(windows)]
pub fn platform_enabled() -> Result<bool, String> {
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    use windows::core::HSTRING;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .map_err(|e| e.to_string())?;
    let setting = notifier.Setting().map_err(|e| e.to_string())?;
    Ok(setting == NotificationSetting::Enabled)
}

#[cfg(not(windows))]
pub fn platform_enabled() -> Result<bool, String> {
    Ok(true)
}

/// Show through WinRT. `Err` means the caller should fall back to the plugin.
#[cfg(windows)]
fn winrt_show(app: &AppHandle, n: &Notice, goto_label: &str) -> Result<(), String> {
    use tauri::Emitter;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};
    use windows::core::{HSTRING, IInspectable, Interface};

    let xml = XmlDocument::new().map_err(|e| e.to_string())?;
    xml.LoadXml(&HSTRING::from(toast_xml(
        n.kind,
        &n.title,
        &n.body,
        n.session.as_deref(),
        goto_label,
    )))
    .map_err(|e| e.to_string())?;

    let toast = ToastNotification::CreateToastNotification(&xml).map_err(|e| e.to_string())?;
    // Tag = the subject. Windows replaces a notification with the same (tag, group) pair, which is
    // exactly "one live notice per session" instead of a growing stack. Tags are limited in length
    // and character set, and our session ids are short hex — but truncate defensively anyway.
    if let Some(subject) = n.tag.as_deref() {
        let tag: String = subject.chars().filter(|c| c.is_ascii_alphanumeric()).take(60).collect();
        if !tag.is_empty() {
            let _ = toast.SetTag(&HSTRING::from(tag));
        }
    }
    let _ = toast.SetGroup(&HSTRING::from(n.kind.group()));

    // Clicking the toast (or its button) brings the app forward and asks the UI to focus that pane.
    // The handler runs on a system thread, so it only clones an AppHandle and emits.
    let app2 = app.clone();
    let _ = toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, args| {
            let arg = args
                .as_ref()
                .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|a| a.Arguments().ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            crate::reveal(&app2);
            if let Some(id) = arg.strip_prefix("goto:") {
                let _ = app2.emit("notify-activate", id.to_string());
            }
            Ok(())
        },
    ));

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .map_err(|e| e.to_string())?;
    notifier.Show(&toast).map_err(|e| e.to_string())?;
    // The reference implementation (tauri-winrt-notification) sleeps here too: Show() returns
    // before the platform has taken ownership, and dropping everything immediately can lose the
    // toast. Cheap insurance on a path that fires at most a few times a minute.
    std::thread::sleep(std::time::Duration::from_millis(10));
    Ok(())
}

#[cfg(not(windows))]
fn winrt_show(_app: &AppHandle, _n: &Notice, _goto_label: &str) -> Result<(), String> {
    Err("winrt: not windows".into())
}

/// The fallback that was the only path before: the official plugin. Title/body only — no tag, no
/// button, no click — but a plain toast beats silence.
fn plugin_show(app: &AppHandle, n: &Notice) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app
        .notification()
        .builder()
        .title(&n.title)
        .body(&n.body)
        .show()
    {
        eprintln!("notify: fallback failed: {e}");
    }
}

// ── policy ───────────────────────────────────────────────────────────────────────────────────────
// Everything above delivers a notification; this decides whether it should exist at all. Kept as a
// pure decision plus one small map, so the rules are unit-testable without a notification platform
// (the same reason the frontend keeps its logic in modules like `menuContinue.ts`).

/// Two notices about the SAME subject closer together than this are a repeat, not news.
const COOLDOWN_MS: u64 = 90_000;
/// …except "someone is still waiting on you", which earns exactly one reminder after this long.
/// Silence would be worse than a second toast when a human is the blocker.
const ESCALATE_MS: u64 = 10 * 60_000;

/// When we last notified about a given subject. Small and bounded by the number of live sessions
/// plus profiles; entries are dropped when a subject is quiet for a full escalation window.
static LAST_SENT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, u64>>,
> = std::sync::LazyLock::new(Default::default);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Should this notice go out, given how long ago we last said something about the same subject?
///
/// `None` = never notified about it. "Waiting" repeats only as an escalation, because a pane that
/// flips blocked→working→blocked while the user works through it would otherwise toast every time;
/// everything else just respects a plain cooldown.
fn allow_send(kind: Kind, since_last_ms: Option<u64>) -> bool {
    match since_last_ms {
        None => true,
        Some(ms) => match kind {
            // One pane: do not re-nag; only escalate. The aggregate is the opposite case — its whole
            // job is to stay current, so it updates on the ordinary cooldown.
            Kind::Blocked => ms >= ESCALATE_MS,
            _ => ms >= COOLDOWN_MS,
        },
    }
}

/// Is the user in a state where Windows itself says "do not disturb"? Presentation mode, a
/// full-screen game, Focus assist / quiet hours. Honouring this is the difference between a helpful
/// notification and one that lands in the middle of a screen share.
#[cfg(windows)]
fn os_wants_quiet() -> bool {
    use windows::Win32::UI::Shell::{
        QUNS_ACCEPTS_NOTIFICATIONS, QUNS_APP, SHQueryUserNotificationState,
    };
    unsafe {
        match SHQueryUserNotificationState() {
            // ACCEPTS = normal desktop; APP = a foreground app is running but notifications are fine.
            Ok(state) => !(state == QUNS_ACCEPTS_NOTIFICATIONS || state == QUNS_APP),
            // Unknown → assume it is fine; going silent on an API hiccup is the worse failure.
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
fn os_wants_quiet() -> bool {
    false
}

/// Per-kind switch on top of the master `status_notify`.
fn kind_enabled(cfg: &crate::HubConfig, kind: Kind) -> bool {
    match kind {
        Kind::Blocked | Kind::BlockedMany => cfg.notify_blocked.unwrap_or(true),
        Kind::Done => cfg.notify_done.unwrap_or(true),
        Kind::Limited => cfg.notify_limited.unwrap_or(true),
        // Maintenance follows the master switch only — it reports things the user cannot foresee.
        Kind::Important => true,
    }
}

/// Send one notification. Honours the `status_notify` switch; everything past that is the channel's
/// business. Never returns an error to the caller — a monitor thread must not die over a toast.
pub fn notify(app: &AppHandle, n: Notice) {
    let cfg = crate::read_config_file();
    if !cfg.status_notify.unwrap_or(true) || !kind_enabled(&cfg, n.kind) {
        return;
    }
    // Windows' own "do not disturb" outranks us — except for the one thing the user is actively
    // blocked on, which is exactly what they would want to hear about even in focus mode.
    if n.kind != Kind::Blocked && os_wants_quiet() {
        return;
    }
    // Repeat suppression, keyed on the subject. A notice with no subject (maintenance) is always a
    // distinct event, so it is never throttled.
    if let Some(subject) = n.tag.as_deref() {
        let now = now_ms();
        let mut seen = LAST_SENT.lock().unwrap_or_else(|e| e.into_inner());
        let since = seen.get(subject).map(|t| now.saturating_sub(*t));
        if !allow_send(n.kind, since) {
            return;
        }
        seen.insert(subject.to_string(), now);
        // Keep the map from growing for the lifetime of the app: anything older than an escalation
        // window can never change a decision again.
        seen.retain(|_, t| now.saturating_sub(*t) < ESCALATE_MS * 2);
    }
    let goto_label = crate::i18n::tr("notify.action_goto", crate::cur_lang());
    // Registration is checked FIRST, and deliberately not `Show()`'s return value: an unregistered
    // identity makes Windows drop the toast while still returning Ok, so waiting for an error would
    // leave the user with nothing at all — no toast and no fallback. If we are not registered, the
    // plugin is the only path that can still deliver something.
    #[cfg(windows)]
    if !CHANNEL_READY.load(std::sync::atomic::Ordering::Relaxed) {
        plugin_show(app, &n);
        return;
    }
    if let Err(e) = winrt_show(app, &n, goto_label) {
        // One line per failure, not a silent swap: if the rich channel is unavailable the user
        // still gets the toast, and the reason is on record.
        eprintln!("notify: winrt channel unavailable ({e}), using the plugin");
        plugin_show(app, &n);
    }
}

/// Does Windows currently allow this app to show toasts? The Settings tab warns when it does not —
/// otherwise a muted app looks exactly like a broken one.
#[tauri::command]
pub fn notify_enabled() -> Result<bool, String> {
    platform_enabled()
}

/// What happened when we tried to register the notification identity, in words.
///
/// A release build is a GUI subsystem binary with no console, so `eprintln!` goes nowhere — the one
/// place this can be read is here. It re-runs the registration (idempotent: an existing shortcut is
/// left alone) and reports the outcome, which is the difference between "Windows shows our toasts"
/// and "Windows silently drops every one of them while Show() reports success".
#[tauri::command]
pub fn notify_diag() -> String {
    #[cfg(windows)]
    {
        match ensure_registered_inner() {
            Ok(msg) => format!("ok: {msg}"),
            Err(e) => format!("failed: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        "n/a: not windows".to_string()
    }
}

/// Fire a harmless notification so the whole path can be checked on demand: identity, permission,
/// and whether anything shows up at all. The one gate the automated suites cannot replace.
#[tauri::command]
pub fn notify_test(app: AppHandle) {
    let lang = crate::cur_lang();
    notify(
        &app,
        Notice {
            kind: Kind::Important,
            title: crate::i18n::tr("notify.test_title", lang).to_string(),
            body: crate::i18n::tr("notify.test_body", lang).to_string(),
            // Repeated presses replace the previous test toast rather than stacking four of them.
            tag: Some("selftest".to_string()),
            session: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_that_would_break_the_xml_are_escaped() {
        // A pane can be renamed to anything and a project folder may contain '&' — unescaped, the
        // document fails to parse and Windows drops the notification with no error anywhere.
        let xml = toast_xml(
            Kind::Blocked,
            "Агент ждёт",
            r#"cc5 · Docx & pdf <обработка> "рабочая""#,
            Some("s7d28ea"),
            "Перейти",
        );
        assert!(xml.contains("Docx &amp; pdf &lt;обработка&gt;"));
        assert!(!xml.contains("& pdf"));
        assert!(xml.contains("&quot;рабочая&quot;"));
    }

    #[test]
    fn only_waiting_is_sticky_and_only_a_session_gets_a_button() {
        // "Someone is waiting on you" must survive the 7-second fade; "finished" must not nag.
        let waiting = toast_xml(Kind::Blocked, "t", "b", Some("s1"), "Перейти");
        assert!(waiting.contains(r#"scenario="reminder""#));
        assert!(waiting.contains(r#"<action content="Перейти""#));
        assert!(waiting.contains(r#"launch="goto:s1""#));

        let done = toast_xml(Kind::Done, "t", "b", Some("s1"), "Перейти");
        assert!(!done.contains("scenario"));

        // Maintenance notices have no session, so there is nothing to jump to — no button, no launch.
        let maintenance = toast_xml(Kind::Important, "t", "b", None, "Перейти");
        assert!(!maintenance.contains("<action"));
        assert!(!maintenance.contains("launch="));
    }

    #[test]
    fn a_first_notice_always_goes_out() {
        // Nothing said about this subject yet — every kind must reach the user.
        for k in [Kind::Blocked, Kind::Done, Kind::Limited, Kind::Important] {
            assert!(allow_send(k, None), "{k:?} suppressed on first notice");
        }
    }

    #[test]
    fn waiting_repeats_only_as_an_escalation_but_never_goes_silent() {
        // A pane the user is working through flips blocked→working→blocked; toasting on every flip
        // is the spam this guards. But silence is worse than one extra toast when a HUMAN is the
        // blocker, so after the escalation window it speaks again.
        assert!(!allow_send(Kind::Blocked, Some(COOLDOWN_MS)));
        assert!(!allow_send(Kind::Blocked, Some(ESCALATE_MS - 1)));
        assert!(allow_send(Kind::Blocked, Some(ESCALATE_MS)));
    }

    #[test]
    fn the_aggregate_stays_current_instead_of_waiting_for_the_escalation_window() {
        // Regression: the "N agents are waiting" notice used to inherit Blocked's escalation rule,
        // so 2→3→4 waiting was silently dropped for ten minutes while the toast claimed to track
        // the count. It must refresh on the ordinary cooldown, and still be sticky + same group.
        assert!(!allow_send(Kind::BlockedMany, Some(COOLDOWN_MS - 1)));
        assert!(allow_send(Kind::BlockedMany, Some(COOLDOWN_MS)));
        assert!(!allow_send(Kind::Blocked, Some(COOLDOWN_MS)), "one pane must NOT re-nag that fast");
        assert!(Kind::BlockedMany.sticky());
        assert_eq!(Kind::BlockedMany.group(), Kind::Blocked.group());
    }

    #[test]
    fn other_kinds_use_the_plain_cooldown() {
        // "Finished" and "hit the limit" are informational: a short cooldown is enough, and they
        // must NOT wait for the (much longer) escalation window that only waiting uses.
        assert!(!allow_send(Kind::Done, Some(COOLDOWN_MS - 1)));
        assert!(allow_send(Kind::Done, Some(COOLDOWN_MS)));
        assert!(allow_send(Kind::Limited, Some(COOLDOWN_MS)));
        assert!(COOLDOWN_MS < ESCALATE_MS, "a cooldown longer than the escalation would invert the rules");
    }

    #[test]
    fn per_kind_switches_default_to_on_and_never_gag_maintenance() {
        // An empty config must behave exactly as before the switches existed.
        let cfg = crate::HubConfig::default();
        for k in [Kind::Blocked, Kind::Done, Kind::Limited, Kind::Important] {
            assert!(kind_enabled(&cfg, k), "{k:?} off by default");
        }
        // Turning off agent chatter must not also silence "the stack is down" — those are the
        // notices the user cannot anticipate.
        let quiet = crate::HubConfig {
            notify_blocked: Some(false),
            notify_done: Some(false),
            notify_limited: Some(false),
            ..Default::default()
        };
        assert!(!kind_enabled(&quiet, Kind::Blocked));
        assert!(!kind_enabled(&quiet, Kind::Done));
        assert!(!kind_enabled(&quiet, Kind::Limited));
        assert!(kind_enabled(&quiet, Kind::Important));
    }

    #[test]
    fn kinds_group_separately_so_a_burst_cannot_bury_a_waiting_notice() {
        assert_ne!(Kind::Blocked.group(), Kind::Done.group());
        assert_ne!(Kind::Blocked.group(), Kind::Limited.group());
        assert!(Kind::Blocked.sticky());
        assert!(!Kind::Done.sticky() && !Kind::Limited.sticky() && !Kind::Important.sticky());
    }
}
