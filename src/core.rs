//! UI-independent pieces of the Linux profile manager used by the Tauri shell.
pub mod models;
pub mod network_utils;
pub mod profile_apply;
pub mod storage;

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
