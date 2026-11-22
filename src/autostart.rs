// Network Manager - Autostart Management
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Manages the autostart .desktop file for starting the application on login.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::{APP_ID, APP_NAME};

/// Get the path to the autostart directory.
fn autostart_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("autostart"))
}

/// Get the path to the autostart .desktop file.
fn autostart_file_path() -> Option<PathBuf> {
    autostart_dir().map(|p| p.join(format!("{}.desktop", APP_ID)))
}

/// Check if autostart is currently enabled.
pub fn is_autostart_enabled() -> bool {
    if let Some(path) = autostart_file_path() {
        path.exists()
    } else {
        false
    }
}

/// Enable autostart by creating the .desktop file.
pub fn enable_autostart() -> Result<(), String> {
    enable_autostart_for("network-manager")
}

pub fn enable_autostart_for(binary: &str) -> Result<(), String> {
    let dir = autostart_dir().ok_or("Could not determine autostart directory")?;
    let path = autostart_file_path().ok_or("Could not determine autostart file path")?;

    // Create autostart directory if it doesn't exist
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create autostart directory: {}", e))?;
    }

    // Create the .desktop file content
    let desktop_content = format!(
        r#"[Desktop Entry]
Name={}
Comment=Manage network and system profiles
Exec={} --minimized
Icon={}
Terminal=false
Type=Application
Categories=System;Network;Settings;GTK;GNOME;
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
NoDisplay=false
"#,
        APP_NAME, binary, APP_ID
    );

    let mut file = fs::File::create(&path)
        .map_err(|e| format!("Failed to create autostart file: {}", e))?;
    file.write_all(desktop_content.as_bytes())
        .map_err(|e| format!("Failed to write autostart file: {}", e))?;

    tracing::info!("Autostart enabled: {}", path.display());
    Ok(())
}

/// Disable autostart by removing the .desktop file.
pub fn disable_autostart() -> Result<(), String> {
    if let Some(path) = autostart_file_path() {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove autostart file: {}", e))?;
            tracing::info!("Autostart disabled: {}", path.display());
        }
    }
    Ok(())
}

/// Set autostart state.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        enable_autostart()
    } else {
        disable_autostart()
    }
}
