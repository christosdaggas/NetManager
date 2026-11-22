use crate::library::{self, Library};
use netmanager_core::{
    models::Profile,
    port_scanner::is_valid_host,
    storage::{write_private_atomic, DataStore},
};
use std::process::Command;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn catalog(language: String) -> ValueMap {
    let code = language.split(['-', '_', '.']).next().unwrap_or("en");
    if code == "en" {
        return ValueMap::new();
    }
    serde_json::from_str::<std::collections::HashMap<String, ValueMap>>(include_str!(concat!(
        env!("OUT_DIR"),
        "/catalogs.json"
    )))
    .unwrap_or_default()
    .remove(code)
    .unwrap_or_default()
}
type ValueMap = std::collections::HashMap<String, String>;

// GTK is the Linux Tauri host toolkit. Writing via the display clipboard also
// works when the webview Clipboard API is unavailable in a custom scheme.
#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    let display = gtk::gdk::Display::default().ok_or("No graphical display.")?;
    let clipboard = gtk::Clipboard::default(&display).ok_or("Clipboard unavailable.")?;
    clipboard.set_text(&text);
    Ok(())
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "{program}: {} {}",
            String::from_utf8_lossy(&out.stderr).trim(),
            text
        ))
    }
}
#[tauri::command]
pub async fn diagnostic(kind: String, target: String) -> Result<String, String> {
    match kind.as_str() {
        "ping" | "dns" if !is_valid_host(&target) => {
            Err("Enter a valid address or host name.".into())
        }
        "ping" => output("ping", &["-c", "4", "-W", "2", "--", &target]),
        "dns" => output("dig", &["+time=3", "+tries=1", &target])
            .or_else(|_| output("host", &[&target]))
            .or_else(|_| output("getent", &["ahosts", &target])),
        "speed" => output("speedtest-cli", &["--simple", "--timeout", "10"]).or_else(|_| {
            output(
                "curl",
                &[
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--max-time",
                    "25",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{speed_download}",
                    "https://speed.cloudflare.com/__down?bytes=10000000",
                ],
            )
            .and_then(|s| {
                s.parse::<f64>()
                    .map(|n| format!("Download: {:.2} Mbps", n * 8. / 1e6))
                    .map_err(|e| e.to_string())
            })
        }),
        "start-service" => output("systemctl", &["start", "NetworkManager.service"]),
        _ => Err("Unknown diagnostic.".into()),
    }
}
#[tauri::command]
pub async fn open_location(kind: String) -> Result<(), String> {
    let target = match kind.as_str() {
        "storage" => DataStore::new().config_dir().display().to_string(),
        "website" => "https://chrisdaggas.com".into(),
        "issues" => "https://github.com/christosdaggas/Network-Manager/issues".into(),
        "releases" => "https://github.com/christosdaggas/Network-Manager/releases".into(),
        _ => return Err("Unknown location.".into()),
    };
    output("xdg-open", &[&target]).map(|_| ())
}
#[tauri::command]
pub async fn export_file(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, Library>,
    kind: String,
    contents: String,
) -> Result<bool, String> {
    let (name, ext, data) = match kind.as_str() {
        "profiles" => {
            let key = state.guard()?;
            let (_, _, profiles) = library::open(key.as_deref())?;
            (
                "network-profiles.json",
                "json",
                serde_json::to_vec_pretty(&profiles).map_err(|e| e.to_string())?,
            )
        }
        "logs" => ("network-manager-logs.txt", "txt", contents.into_bytes()),
        "scanner" => ("network-scan.csv", "csv", contents.into_bytes()),
        "ports" => ("port-scan.csv", "csv", contents.into_bytes()),
        _ => return Err("Unknown export format.".into()),
    };
    let file = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(name)
        .add_filter(ext, &[ext])
        .blocking_save_file();
    if let Some(file) = file {
        write_private_atomic(&file.into_path().map_err(|e| e.to_string())?, &data)
            .map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn import_profiles(
    state: State<'_, Library>,
    contents: String,
    include_scripts: bool,
) -> Result<usize, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|e| e.to_string())?;
    if let Some(wrapped) = value.get_mut("profiles") {
        value = wrapped.take();
    }
    let mut added: Vec<Profile> =
        serde_json::from_value(value).map_err(|e| format!("Invalid profile backup: {e}"))?;
    if added.is_empty() {
        return Err("No profiles found in file.".into());
    }
    let key = state.guard()?;
    let (store, cfg, mut profiles) = library::open(key.as_deref())?;
    for p in &mut added {
        p.metadata.id = uuid::Uuid::new_v4();
        p.metadata.name.push_str(" (Imported)");
        p.metadata.created_at = chrono::Utc::now();
        p.metadata.updated_at = p.metadata.created_at;
        p.metadata.last_applied_at = None;
        p.approved_for_users = false;
        if !include_scripts {
            p.automation_actions.clear();
        }
    }
    let count = added.len();
    profiles.append(&mut added);
    store.persist_profiles(&profiles, &cfg)?;
    store.append_log("INFO", &format!("Imported {count} profiles."));
    Ok(count)
}
#[tauri::command]
pub async fn check_update() -> Option<String> {
    netmanager_core::version_check::check_for_update(env!("CARGO_PKG_VERSION"))
        .await
        .map(|v| v.latest_version)
}
