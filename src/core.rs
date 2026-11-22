//! UI-independent pieces of the Linux profile manager used by the Tauri shell.
pub mod models;
pub mod network_utils;
pub mod profile_apply;
pub mod storage;
pub mod autostart;
pub mod scheduler;
pub mod tray;
pub mod version_check;
pub const APP_ID: &str = models::APP_ID;
pub const APP_NAME: &str = "Network Manager";
#[path = "services/autoswitch.rs"]
pub mod autoswitch;
#[path = "services/watchdog.rs"]
pub mod watchdog;
#[path = "services/scanner.rs"]
pub mod scanner;
#[path = "services/port_scanner.rs"]
pub mod port_scanner;

#[path = "services/encryption.rs"]
pub mod encryption;
#[path = "services/sandbox.rs"]
pub mod sandbox;

pub mod services {
    pub use super::encryption::{make_check_token, verify_check_token, ProfileEncryption};
    pub use super::sandbox::SandboxRunner;
}

pub fn tr(message: &str) -> String {
    gettextrs::gettext(message)
}
