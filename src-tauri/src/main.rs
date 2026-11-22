mod library;
mod network;
use tauri::Manager;

#[tauri::command]
fn open_classic() -> Result<(), String> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        paths.push(exe.with_file_name("network-manager"));
    }
    #[cfg(debug_assertions)]
    paths.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/network-manager"),
    );
    paths.push("network-manager".into());
    for path in paths {
        match std::process::Command::new(path).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(
        "Install the classic Network Manager RPM, or run cargo build in the repository root."
            .into(),
    )
}

fn main() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(library::Library::default())
        .invoke_handler(tauri::generate_handler![
            network::network_snapshot,
            network::traffic_counters,
            library::library_view,
            library::save_profile,
            library::apply_profile,
            library::delete_profile,
            library::unlock_profiles,
            open_classic
        ])
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .run(tauri::generate_context!())
}
