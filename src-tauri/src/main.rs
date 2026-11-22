mod background;
mod hotkeys;
mod library;
mod network;
mod operations;
mod scans;
mod windows;
use tauri::Manager;

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0)
}

fn main() -> tauri::Result<()> {
    let debug = std::env::args().any(|a| a == "--debug" || a == "-d");
    let level = if debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .init();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("Network Manager\nUsage: network-manager-workspace [--minimized] [--debug] [--version]");
                return Ok(());
            }
            "-v" | "--version" => {
                println!("Network Manager {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => (),
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .with_filter(|label| label == "main")
                .build(),
        )
        .manage(library::Library::default())
        .manage(scans::Scans::default())
        .setup(|app| {
            app.manage(hotkeys::Hotkeys::start(app.handle().clone()));
            app.manage(background::Services::start(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            network::network_snapshot,
            network::traffic_counters,
            library::library_view,
            library::profile_form,
            library::save_profile,
            library::profile_operation,
            library::apply_profile,
            library::unlock_profiles,
            library::save_setting,
            library::encryption,
            library::clear_logs,
            operations::diagnostic,
            operations::open_location,
            operations::export_file,
            operations::import_profiles,
            operations::check_update,
            operations::catalog,
            operations::copy_text,
            scans::detect_range,
            scans::scan_start,
            scans::scan_stop,
            scans::scan_events,
            scans::wake_device,
            windows::open_utility,
            windows::workspace_changed,
            windows::inspect_device,
            quit_app
        ])
        .run(tauri::generate_context!())
}
