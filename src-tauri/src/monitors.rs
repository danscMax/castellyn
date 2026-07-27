// ===================== Multi-monitor windows (pop a live pane onto another monitor) =====================
// A pane can be "popped out" to its own frameless window on a chosen monitor. The window renders the
// SAME live session by attaching an extra output channel (session_attach) — no respawn. Monitors are
// enumerated and windows created/positioned from Rust (PhysicalPosition → correct across mixed DPI;
// no JS window perms needed). Child window labels (mon-* / pane-*) get core:default via the capability.
// A small handoff registry passes the pane's display spec to the new window.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::cur_lang;
use crate::i18n::tr;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    index: usize,
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    primary: bool,
}

#[tauri::command]
pub fn list_monitors(app: AppHandle) -> Vec<MonitorInfo> {
    let prim_pos = app.primary_monitor().ok().flatten().map(|m| {
        let p = m.position();
        (p.x, p.y)
    });
    match app.available_monitors() {
        Ok(mons) => mons
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let p = m.position();
                let s = m.size();
                MonitorInfo {
                    index: i,
                    name: m
                        .name()
                        .cloned()
                        .unwrap_or_else(|| format!("Monitor {}", i + 1)),
                    x: p.x,
                    y: p.y,
                    width: s.width,
                    height: s.height,
                    scale: m.scale_factor(),
                    primary: prim_pos == Some((p.x, p.y)),
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

// Handoff: the main window stashes a pane's display spec under the new window's label; the child reads
// (and clears) it on mount. Lazy-init map (HashMap::new isn't const).
static DETACH_REGISTRY: Mutex<Option<std::collections::HashMap<String, serde_json::Value>>> =
    Mutex::new(None);

#[tauri::command]
pub fn prepare_detach(label: String, spec: serde_json::Value) {
    let mut g = DETACH_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(std::collections::HashMap::new)
        .insert(label, spec);
}

#[tauri::command]
pub fn take_detach(label: String) -> Option<serde_json::Value> {
    let mut g = DETACH_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().and_then(|m| m.remove(&label))
}

/// Open (or focus) a frameless window filling the given monitor. Positioned/sized in PHYSICAL pixels
/// so it lands correctly across mixed-DPI monitors. The window loads the app; its label drives the
/// detached view on the frontend.
#[tauri::command]
pub fn open_monitor_window(
    app: AppHandle,
    label: String,
    monitor_index: usize,
) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        // Ok() here would be a lie: the caller stashed a spec via prepare_detach and reads Ok as
        // "the panes moved", removing them from the grid — but this window mounted long ago and
        // will never consume it, so they end up rendered nowhere. Drop the orphaned spec (it would
        // otherwise be replayed by the next window under this label) and report the failure so the
        // caller leaves the panes where they are.
        let _ = take_detach(label);
        return Err(tr("err.monitor_window_open", cur_lang()).into());
    }
    let mons = app.available_monitors().map_err(|e| e.to_string())?;
    let m = mons
        .get(monitor_index)
        .ok_or_else(|| tr("err.monitor_out_of_range", cur_lang()).to_string())?;
    let pos = *m.position();
    let size = *m.size();
    // Build OFF the main thread. The command runs on the main (event-loop) thread, and a synchronous
    // `WebviewWindowBuilder::build()` there DEADLOCKS: WebView2 creation is async and needs the event
    // loop to pump, but build() blocks that very loop. From a worker thread, build() dispatches the
    // creation to the (now free) main loop and returns once the webview is ready.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let built = tauri::WebviewWindowBuilder::new(
            &app2,
            &label,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Castellyn")
        .decorations(false)
        // Dark background so the frame never flashes white while the webview boots.
        .background_color(tauri::webview::Color(8, 12, 24, 255))
        .build();
        // Physical position/size — correct across mixed-DPI monitors (the window-state plugin is
        // denylisted for mon-* so it can't override these with a stale restored rect).
        match built {
            Ok(win) => {
                let _ = win.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                let _ = win.set_size(tauri::PhysicalSize::new(size.width, size.height));
                let _ = win.set_focus();
            }
            Err(e) => {
                // If a window with this label already exists, ANOTHER open_monitor_window won the race
                // (a duplicate-label build error). Don't clear the registry or report a failure — the
                // winner owns the stashed spec and the live window.
                if app2.get_webview_window(&label).is_some() {
                    return;
                }
                // Genuine build failure — don't fail silently. The frontend stashed the pane spec under
                // this label (prepare_detach) before calling us; clear it so it can't leak, and tell the
                // UI so it can re-home the pane / toast instead of "losing" the detached session.
                let _ = take_detach(label.clone());
                let _ = app2.emit(
                    "monitor-window-failed",
                    serde_json::json!({ "label": label, "error": e.to_string() }),
                );
            }
        }
    });
    Ok(())
}
