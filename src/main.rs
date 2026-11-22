// Network Manager - Main Entry Point
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! # Network Manager
//!
//! A GTK4/libadwaita network and system profile manager for Linux.
//!
//! This is the main entry point for the GUI application.

#[cfg(feature = "gresource")]
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use std::env;

mod application;
mod autostart;
mod models;
mod network_utils;
mod profile_apply;
mod scheduler;
mod services;
mod storage;
mod tray;
mod ui;
mod version_check;

use application::Application;

/// Application ID for GNOME/Freedesktop.
pub const APP_ID: &str = models::APP_ID;

/// Human-readable application name.
pub const APP_NAME: &str = "Network Manager";

/// Application version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Gettext domain for translations.
pub const GETTEXT_DOMAIN: &str = "network-manager";

/// Print version information and exit.
fn print_version() {
    println!("{} {}", APP_NAME, VERSION);
    println!("Copyright (C) 2026 Christos A. Daggas");
    println!("License: MIT");
    println!();
    println!("A GTK4/libadwaita network and system profile manager for Linux.");
}

/// Print help information and exit.
fn print_help() {
    println!(
        "Usage: {} [OPTIONS]",
        env::args()
            .next()
            .unwrap_or_else(|| "network-manager".to_string())
    );
    println!();
    println!("A GTK4/libadwaita network and system profile manager for Linux.");
    println!();
    println!("Options:");
    println!("  -h, --help       Show this help message and exit");
    println!("  -v, --version    Show version information and exit");
    println!("  -m, --minimized  Start in the background without opening the window");
    println!("  -d, --debug      Enable debug logging");
    println!();
    println!("Environment variables:");
    println!("  RUST_LOG         Set log level (trace, debug, info, warn, error)");
    println!();
    println!("Report bugs to: https://github.com/christosdaggas/Network-Manager/issues");
}

/// Translate a message using the application's gettext domain.
///
/// Call it as a bare `tr("...")` (import it with `use crate::tr;`):
/// xgettext does not extract path-qualified calls such as `crate::tr(...)`.
/// Values go in named placeholders, e.g.
/// `tr("Apply '{name}'?").replace("{name}", name)`.
pub fn tr(message: &str) -> String {
    gettextrs::gettext(message)
}

/// Translate a message with singular and plural forms, chosen by `n`.
///
/// Same calling rules as [`tr`]; put the number in a `{n}` placeholder.
#[allow(dead_code)]
pub fn ntr(singular: &str, plural: &str, n: u32) -> String {
    gettextrs::ngettext(singular, plural, n)
}

/// Initialize internationalization (gettext).
fn setup_i18n() {
    use gettextrs::{bindtextdomain, setlocale, textdomain, LocaleCategory};

    // Set locale from environment
    setlocale(LocaleCategory::LcAll, "");

    // Development builds compile catalogs into target/locale; installed
    // builds use the system locale directory (AppImages keep it next to the
    // executable under usr/share/locale).
    let mut locale_dirs = vec![
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/locale"),
        std::path::PathBuf::from("/usr/share/locale"),
        std::path::PathBuf::from("/usr/local/share/locale"),
    ];
    if let Ok(executable) = std::env::current_exe() {
        if let Some(prefix) = executable.parent().and_then(|dir| dir.parent()) {
            locale_dirs.insert(0, prefix.join("share/locale"));
        }
    }

    for dir in &locale_dirs {
        if dir.exists() {
            if let Err(e) = bindtextdomain(GETTEXT_DOMAIN, dir) {
                tracing::warn!("Failed to bind textdomain to {}: {}", dir.display(), e);
            } else {
                tracing::debug!("Bound textdomain to {}", dir.display());
                break;
            }
        }
    }

    if let Err(e) = textdomain(GETTEXT_DOMAIN) {
        tracing::warn!("Failed to set textdomain: {}", e);
    }
}

fn main() -> glib::ExitCode {
    // Parse command-line arguments before GTK initialization
    let args: Vec<String> = env::args().collect();
    let mut start_minimized = false;
    let mut debug_mode = false;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return glib::ExitCode::SUCCESS;
            }
            "-v" | "--version" => {
                print_version();
                return glib::ExitCode::SUCCESS;
            }
            "-m" | "--minimized" => {
                start_minimized = true;
            }
            "-d" | "--debug" => {
                debug_mode = true;
            }
            _ => {
                if arg.starts_with('-') {
                    eprintln!("Unknown option: {}", arg);
                    eprintln!("Try '--help' for more information.");
                    return glib::ExitCode::FAILURE;
                }
            }
        }
    }

    // Set the program name to match StartupWMClass in the .desktop file
    glib::set_prgname(Some(APP_ID));

    // Initialize logging with appropriate level
    let log_level = if debug_mode {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(log_level.into()),
        )
        .init();

    tracing::info!("Starting {} v{}", APP_NAME, VERSION);
    if start_minimized {
        tracing::info!("Starting in minimized mode");
    }

    // Initialize internationalization
    setup_i18n();
    glib::set_application_name(&tr(APP_NAME));

    // Initialize GTK and Libadwaita
    if let Err(e) = libadwaita::init() {
        eprintln!("Failed to initialize libadwaita: {}", e);
        eprintln!("This application requires a graphical environment.");
        return glib::ExitCode::FAILURE;
    }

    // Add the data/icons directory to the icon theme search path for development
    // Handle headless environments gracefully (e.g., SSH sessions without display)
    let display = match gtk4::gdk::Display::default() {
        Some(d) => d,
        None => {
            eprintln!("No display found. This application requires a graphical environment.");
            eprintln!("If running via SSH, ensure X11 forwarding or Wayland socket is available.");
            return glib::ExitCode::FAILURE;
        }
    };
    let icon_theme = gtk4::IconTheme::for_display(&display);

    // Try to find the icons directory relative to the executable or in common locations
    let possible_paths = [
        // Development: relative to current working directory
        std::path::PathBuf::from("data/icons"),
        // Development: relative to cargo manifest
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/icons"),
        // Installed: standard locations
        std::path::PathBuf::from("/usr/share/icons"),
        std::path::PathBuf::from("/usr/local/share/icons"),
        dirs::data_dir().unwrap_or_default().join("icons"),
    ];

    for path in &possible_paths {
        if path.exists() {
            icon_theme.add_search_path(path);
            tracing::debug!("Added icon search path: {:?}", path);
        }
    }

    // Register resources (if gresource available)
    #[cfg(feature = "gresource")]
    {
        let resource_bytes = include_bytes!(concat!(
            env!("OUT_DIR"),
            "/com.chrisdaggas.network-manager.gresource"
        ));
        let resource_data = glib::Bytes::from_static(resource_bytes);
        if let Ok(resource) = gio::Resource::from_data(&resource_data) {
            gio::resources_register(&resource);
        }
    }

    // Create and run the application.
    // The command line was parsed above. GApplication registers no options of
    // its own and would reject ours ("Unknown option --minimized"), so only
    // argv[0] is forwarded to it.
    let app = Application::new();
    app.set_start_minimized(start_minimized);
    let program_name = args
        .first()
        .cloned()
        .unwrap_or_else(|| "network-manager".to_string());
    app.run_with_args(&[program_name])
}

/// Helper macro for gettext translations.
#[macro_export]
macro_rules! i18n {
    ($s:expr) => {
        gettext_rs::gettext($s)
    };
}

/// Helper macro for ngettext (plurals).
#[macro_export]
macro_rules! ni18n {
    ($singular:expr, $plural:expr, $n:expr) => {
        gettext_rs::ngettext($singular, $plural, $n as u32)
    };
}

/// Helper macro for pgettext (context-aware translations).
#[macro_export]
macro_rules! pi18n {
    ($context:expr, $msgid:expr) => {
        gettext_rs::pgettext($context, $msgid)
    };
}
