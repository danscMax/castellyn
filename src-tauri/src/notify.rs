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
        self == Kind::Blocked
    }
    /// Toasts are grouped per kind so a burst of "finished" notices cannot bury a "waiting" one.
    fn group(self) -> &'static str {
        match self {
            Kind::Blocked => "blocked",
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
    /// The session this is about, when there is one. Used as the toast TAG — a newer notice about
    /// the same session replaces the older one instead of stacking — and as the payload of the
    /// click, so activating the toast can jump straight to that pane.
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
    match ensure_registered_inner() {
        Ok(msg) => eprintln!("notify: AUMID registration — {msg}"),
        Err(e) => eprintln!("notify: AUMID registration FAILED — {e}"),
    }
}

#[cfg(windows)]
fn ensure_registered_inner() -> Result<String, String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
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
            Ok(p) => p.to_string().unwrap_or_default(),
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
        // Already registered (by us on an earlier run, or by the installer) — leave it alone.
        if lnk.exists() {
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
    if let Some(id) = n.session.as_deref() {
        let tag: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(60).collect();
        let _ = toast.SetTag(&HSTRING::from(tag));
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

/// Send one notification. Honours the `status_notify` switch; everything past that is the channel's
/// business. Never returns an error to the caller — a monitor thread must not die over a toast.
pub fn notify(app: &AppHandle, n: Notice) {
    if !crate::read_config_file().status_notify.unwrap_or(true) {
        return;
    }
    let goto_label = crate::i18n::tr("notify.action_goto", crate::cur_lang());
    if let Err(e) = winrt_show(app, &n, goto_label) {
        // One line per failure, not a silent swap: if the rich channel is unavailable (typically an
        // unregistered AppUserModelID) the user still gets the toast, and the reason is on record.
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
    fn kinds_group_separately_so_a_burst_cannot_bury_a_waiting_notice() {
        assert_ne!(Kind::Blocked.group(), Kind::Done.group());
        assert_ne!(Kind::Blocked.group(), Kind::Limited.group());
        assert!(Kind::Blocked.sticky());
        assert!(!Kind::Done.sticky() && !Kind::Limited.sticky() && !Kind::Important.sticky());
    }
}
