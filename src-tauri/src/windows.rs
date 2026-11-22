use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Only local bundled content is used for auxiliary windows. Initial data is
/// serialized rather than interpolated as script or appended to a URL.
#[tauri::command]
pub async fn open_utility(app: AppHandle, kind: String, data: Option<Value>) -> Result<(), String> {
    let (title, w, h) = match kind.as_str() {
        "new" => ("Create Profile", 700., 850.),
        "edit" => ("Edit Profile", 700., 850.),
        "switch" => ("Switch Profile", 460., 480.),
        "preferences" => ("Preferences", 600., 640.),
        "diagnostics" => ("Network Diagnostics", 700., 600.),
        "device" => ("Device Details", 560., 600.),
        "schedules" => ("Manage Schedules", 560., 520.),
        "hotkeys" => ("Configure Hotkeys", 560., 520.),
        "rules" => ("Auto-Switch Rules", 550., 450.),
        _ => return Err("Unknown window.".into()),
    };
    let parent = app
        .get_webview_window("main")
        .ok_or("Main window closed.")?;
    let label = format!("utility-{}", uuid::Uuid::new_v4());
    WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(w, h)
        .min_inner_size(350., 260.)
        .prevent_overflow()
        .transient_for(&parent)
        .map_err(|e| e.to_string())?
        .initialization_script(format!(
            "window.__NM_UTILITY__={};",
            json!({"kind":kind,"data":data})
        ))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub fn workspace_changed(app: AppHandle, profiles_page: Option<bool>) {
    let _ = app.emit_to("main", "workspace-changed", profiles_page.unwrap_or(false));
}
#[tauri::command]
pub fn inspect_device(app: AppHandle, ip: std::net::Ipv4Addr) {
    let _ = app.emit_to("main", "inspect-device", ip.to_string());
}
