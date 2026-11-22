// Network Manager - System Tray
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! StatusNotifierItem (AppIndicator) system tray integration.
//!
//! The tray uses the freedesktop StatusNotifierItem protocol, which GNOME
//! exposes through the AppIndicator extension and which KDE Plasma, Xfce,
//! Cinnamon and others support natively. On a desktop without a
//! StatusNotifierWatcher, no tray icon can be shown; the application keeps
//! working normally and logs why the tray is unavailable.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use ksni::menu::{StandardItem, SubMenu};
use ksni::{MenuItem, Tray, TrayMethods};
use tracing::info;

use crate::tr;

/// Commands that can be sent from the tray to the application.
#[derive(Debug, Clone)]
pub enum TrayCommand {
    /// Show the main window.
    ShowWindow,
    /// Quit the application.
    Quit,
    /// Activate the profile with the given ID.
    ActivateProfile(String),
}

/// Data shared between the GTK thread and the tray service.
struct TrayState {
    /// Profile ID and display name.
    profiles: Vec<(String, String)>,
    /// Currently active profile ID.
    active_profile: Option<String>,
    command_tx: Sender<TrayCommand>,
}

impl TrayState {
    fn send(&self, command: TrayCommand) {
        let _ = self.command_tx.send(command);
    }
}

/// StatusNotifierItem implementation.
struct NetworkManagerTray {
    state: Arc<Mutex<TrayState>>,
}

impl NetworkManagerTray {
    fn send(&self, command: TrayCommand) {
        if let Ok(state) = self.state.lock() {
            state.send(command);
        }
    }

    fn active_label(state: &TrayState) -> String {
        state
            .active_profile
            .as_ref()
            .and_then(|id| {
                state
                    .profiles
                    .iter()
                    .find(|(profile_id, _)| profile_id == id)
                    .map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| tr("No profile active"))
    }
}

impl Tray for NetworkManagerTray {
    /// Open the menu on left click; many applet hosts only show a menu.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "com.chrisdaggas.network-manager".to_string()
    }

    fn title(&self) -> String {
        tr("Network Manager")
    }

    fn icon_name(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: tr("Open Network Manager"),
                icon_name: "window-new-symbolic".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::ShowWindow)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                // TRANSLATORS: Tray menu entry naming the active profile
                label: tr("Active: {name}").replace("{name}", &Self::active_label(&state)),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];

        if !state.profiles.is_empty() {
            let submenu: Vec<MenuItem<Self>> = state
                .profiles
                .iter()
                .map(|(id, name)| {
                    let profile_id = id.clone();
                    StandardItem {
                        label: name.clone(),
                        activate: Box::new(move |tray: &mut Self| {
                            tray.send(TrayCommand::ActivateProfile(profile_id.clone()));
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();

            items.push(
                SubMenu {
                    label: tr("Switch Profile"),
                    submenu,
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: tr("Quit"),
                icon_name: "application-exit-symbolic".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// Handle for the running tray service.
pub struct TrayHandle {
    runtime: &'static tokio::runtime::Runtime,
    handle: ksni::Handle<NetworkManagerTray>,
    state: Arc<Mutex<TrayState>>,
}

impl TrayHandle {
    /// Update the active profile shown in the menu.
    pub fn set_active_profile(&self, name: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.active_profile = name;
        }
        self.refresh();
    }

    /// Update the list of profiles in the menu.
    pub fn set_profiles(&self, profiles: Vec<(String, String)>) {
        if let Ok(mut state) = self.state.lock() {
            state.profiles = profiles;
        }
        self.refresh();
    }

    fn refresh(&self) {
        // The menu is rebuilt by the tray service; ask it to republish the
        // layout and properties after a state change.
        let _ = self.runtime.block_on(self.handle.update(|_| ()));
    }

    /// Stop the tray service.
    pub fn shutdown(&self) -> ksni::ShutdownAwaiter {
        self.handle.shutdown()
    }
}

/// Start the StatusNotifierItem tray service.
///
/// Returns `None` when the desktop has no StatusNotifierWatcher (for example
/// stock GNOME without the AppIndicator extension).
pub fn start_tray(runtime: &'static tokio::runtime::Runtime) -> Option<(TrayHandle, Receiver<TrayCommand>)> {
    let (tx, rx) = channel();
    let state = Arc::new(Mutex::new(TrayState {
        profiles: Vec::new(),
        active_profile: None,
        command_tx: tx,
    }));

    let tray = NetworkManagerTray {
        state: Arc::clone(&state),
    };

    let result = runtime.block_on(tray.spawn());
    match result {
        Ok(handle) => {
            info!("System tray icon registered");
            Some((TrayHandle { handle, state, runtime }, rx))
        }
        Err(error) => {
            info!("System tray unavailable: {}", error);
            None
        }
    }
}
