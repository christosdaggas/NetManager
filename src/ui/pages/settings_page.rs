// Network Manager - Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Application settings page.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use tracing::info;

use crate::models::config::ThemePreference;
use crate::models::{
    HotkeyEntry, Profile, SandboxMode, ScheduleEntry, WatchdogAction, CONFIG_DIR_NAME,
};
use crate::tr;
use crate::ui::dialog_window;

/// The application that owns the window a widget is shown in.
fn app_for(widget: &impl IsA<gtk::Widget>) -> Option<crate::application::Application> {
    widget
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.application())
        .and_downcast::<crate::application::Application>()
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SettingsPage {
        pub theme_row: RefCell<Option<adw::ComboRow>>,
        pub auto_switch_row: RefCell<Option<adw::SwitchRow>>,
        pub auto_switch_interval: RefCell<Option<adw::SpinRow>>,
        pub auto_switch_allow_scripts_row: RefCell<Option<adw::SwitchRow>>,
        pub notifications_row: RefCell<Option<adw::SwitchRow>>,
        pub start_minimized_row: RefCell<Option<adw::SwitchRow>>,
        // Scheduling
        pub scheduling_enabled_row: RefCell<Option<adw::SwitchRow>>,
        // Watchdog
        pub watchdog_enabled_row: RefCell<Option<adw::SwitchRow>>,
        pub watchdog_interval_row: RefCell<Option<adw::SpinRow>>,
        pub watchdog_target_row: RefCell<Option<adw::EntryRow>>,
        pub watchdog_threshold_row: RefCell<Option<adw::SpinRow>>,
        pub watchdog_action_row: RefCell<Option<adw::ComboRow>>,
        pub watchdog_fallback_row: RefCell<Option<adw::ComboRow>>,
        /// Profiles available for the fallback selector.
        pub profiles: RefCell<Vec<Profile>>,
        // Security
        pub sandbox_row: RefCell<Option<adw::ComboRow>>,
        pub encryption_row: RefCell<Option<adw::SwitchRow>>,
        pub encryption_change_row: RefCell<Option<adw::ButtonRow>>,
        pub encryption_unlock_row: RefCell<Option<adw::ButtonRow>>,
        // Hotkeys
        pub hotkeys_enabled_row: RefCell<Option<adw::SwitchRow>>,
        /// Set while load_from_config fills the rows, so their handlers don't save.
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsPage {
        const NAME: &'static str = "CdNetworkManagerSettingsPage";
        type Type = super::SettingsPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for SettingsPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for SettingsPage {}
    impl gtk::subclass::prelude::BoxImpl for SettingsPage {}
}

glib::wrapper! {
    pub struct SettingsPage(ObjectSubclass<imp::SettingsPage>)
        @extends gtk::Widget, gtk::Box;
}

impl SettingsPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Scrolled container
        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        self.append(&scrolled);

        // Content box with margins
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .hexpand(true)
            .build();
        scrolled.set_child(Some(&content));

        // Appearance group
        let appearance_group = adw::PreferencesGroup::builder()
            .title(tr("Appearance"))
            .build();

        let theme_labels = [
            // TRANSLATORS: Color scheme option that follows the system setting
            tr("System"),
            // TRANSLATORS: Color scheme option
            tr("Light"),
            // TRANSLATORS: Color scheme option
            tr("Dark"),
        ];
        let theme_label_refs: Vec<&str> = theme_labels.iter().map(String::as_str).collect();
        let theme_model = gtk::StringList::new(&theme_label_refs);
        let theme_row = adw::ComboRow::builder()
            .title(tr("Color Scheme"))
            .subtitle(tr("Select the application color scheme"))
            .model(&theme_model)
            .build();
        appearance_group.add(&theme_row);

        // Apply and remember the chosen color scheme
        let this_for_theme = self.downgrade();
        theme_row.connect_selected_notify(move |row| {
            let Some(this) = this_for_theme.upgrade() else {
                return;
            };
            if this.imp().loading.get() {
                return;
            }
            let theme = match row.selected() {
                1 => ThemePreference::Light,
                2 => ThemePreference::Dark,
                _ => ThemePreference::System,
            };
            if let Some(app) = this.app() {
                app.apply_theme(theme);
            }
            this.update_app_config(|config| config.theme = theme);
        });

        // Follow changes made from the main menu or the Preferences dialog
        let theme_row_weak = theme_row.downgrade();
        let this_for_scheme = self.downgrade();
        adw::StyleManager::default().connect_color_scheme_notify(move |manager| {
            let (Some(row), Some(this)) = (theme_row_weak.upgrade(), this_for_scheme.upgrade())
            else {
                return;
            };
            let index = match manager.color_scheme() {
                adw::ColorScheme::ForceLight => 1,
                adw::ColorScheme::ForceDark => 2,
                _ => 0,
            };
            let was_loading = this.imp().loading.replace(true);
            row.set_selected(index);
            this.imp().loading.set(was_loading);
        });
        *imp.theme_row.borrow_mut() = Some(theme_row);

        content.append(&appearance_group);

        // Auto-Switch group
        let auto_switch_group = adw::PreferencesGroup::builder()
            .title(tr("Auto-Switch"))
            .description(tr("Automatically activate profiles based on conditions"))
            .build();

        let auto_switch_row = adw::SwitchRow::builder()
            .title(tr("Enable Auto-Switch"))
            .subtitle(tr("Activate profiles when conditions match"))
            .active(true)
            .build();
        auto_switch_group.add(&auto_switch_row);
        let this_for_as = self.downgrade();
        auto_switch_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_as.upgrade() {
                let enabled = row.is_active();
                this.update_app_config(|config| config.auto_switch_enabled = enabled);
            }
        });
        *imp.auto_switch_row.borrow_mut() = Some(auto_switch_row);

        let auto_switch_interval = adw::SpinRow::builder()
            .title(tr("Check Interval"))
            .subtitle(tr("Seconds between condition checks"))
            .adjustment(&gtk::Adjustment::new(30.0, 5.0, 300.0, 5.0, 30.0, 0.0))
            .build();
        auto_switch_group.add(&auto_switch_interval);
        let this_for_interval = self.downgrade();
        auto_switch_interval.connect_value_notify(move |row| {
            if let Some(this) = this_for_interval.upgrade() {
                let secs = row.value() as u32;
                this.update_app_config(|config| config.auto_switch_interval_secs = secs);
            }
        });
        *imp.auto_switch_interval.borrow_mut() = Some(auto_switch_interval);

        let auto_switch_allow_scripts_row = adw::SwitchRow::builder()
            .title(tr("Allow Scripts During Auto-Switch"))
            .subtitle(tr("Run automation actions when auto-switching profiles"))
            .active(false)
            .build();
        auto_switch_group.add(&auto_switch_allow_scripts_row);

        // Connect auto-switch allow scripts save
        let this_for_as_scripts = self.downgrade();
        auto_switch_allow_scripts_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_as_scripts.upgrade() {
                this.save_auto_switch_allow_scripts(row.is_active());
            }
        });

        *imp.auto_switch_allow_scripts_row.borrow_mut() = Some(auto_switch_allow_scripts_row);

        // Manage rules row
        let manage_rules_row = adw::ActionRow::builder()
            .title(tr("Auto-Switch Rules"))
            .subtitle(tr("Configure conditions for automatic profile switching"))
            .activatable(true)
            .build();
        let rules_icon = gtk::Image::from_icon_name("view-list-symbolic");
        manage_rules_row.add_suffix(&rules_icon);
        let rules_arrow = gtk::Image::from_icon_name("go-next-symbolic");
        manage_rules_row.add_suffix(&rules_arrow);
        auto_switch_group.add(&manage_rules_row);

        // Connect manage rules
        let this_for_rules = self.downgrade();
        manage_rules_row.connect_activated(move |_| {
            if let Some(this) = this_for_rules.upgrade() {
                this.show_manage_rules_dialog();
            }
        });

        content.append(&auto_switch_group);

        // Behavior group
        let behavior_group = adw::PreferencesGroup::builder()
            .title(tr("Behavior"))
            .build();

        let notifications_row = adw::SwitchRow::builder()
            .title(tr("Desktop Notifications"))
            .subtitle(tr("Show notifications for profile changes"))
            .active(true)
            .build();
        behavior_group.add(&notifications_row);
        let this_for_notify = self.downgrade();
        notifications_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_notify.upgrade() {
                let enabled = row.is_active();
                this.update_app_config(|config| config.show_notifications = enabled);
            }
        });
        *imp.notifications_row.borrow_mut() = Some(notifications_row);

        let start_minimized_row = adw::SwitchRow::builder()
            .title(tr("Start Minimized"))
            .subtitle(tr("Minimize to system tray on startup"))
            .active(false)
            .build();
        behavior_group.add(&start_minimized_row);
        let this_for_minimized = self.downgrade();
        start_minimized_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_minimized.upgrade() {
                let enabled = row.is_active();
                this.update_app_config(|config| config.start_minimized = enabled);
            }
        });
        *imp.start_minimized_row.borrow_mut() = Some(start_minimized_row);

        content.append(&behavior_group);

        // Storage group
        let storage_group = adw::PreferencesGroup::builder()
            .title(tr("Storage"))
            .build();

        let profiles_dir = Self::get_profiles_dir();
        let profiles_dir_row = adw::ActionRow::builder()
            .title(tr("Profiles Directory"))
            .subtitle(profiles_dir.to_string_lossy().as_ref())
            .build();
        let open_folder_btn = gtk::Button::from_icon_name("folder-symbolic");
        open_folder_btn.set_valign(gtk::Align::Center);
        open_folder_btn.set_tooltip_text(Some(&tr("Open folder")));
        profiles_dir_row.add_suffix(&open_folder_btn);
        storage_group.add(&profiles_dir_row);

        // Connect profiles folder button
        let profiles_dir_clone = profiles_dir.clone();
        open_folder_btn.connect_clicked(move |_| {
            // Ensure directory exists before opening
            if let Err(e) = std::fs::create_dir_all(&profiles_dir_clone) {
                tracing::error!("Failed to create profiles directory: {}", e);
                return;
            }
            let uri = format!("file://{}", profiles_dir_clone.display());
            if let Err(e) =
                gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
            {
                tracing::error!("Failed to open profiles directory: {}", e);
            }
        });

        let logs_dir = Self::get_logs_dir();
        let logs_file = logs_dir.join("logs.json");
        let logs_dir_row = adw::ActionRow::builder()
            .title(tr("Logs File"))
            .subtitle(logs_file.to_string_lossy().as_ref())
            .build();
        let open_logs_btn = gtk::Button::from_icon_name("folder-symbolic");
        open_logs_btn.set_valign(gtk::Align::Center);
        open_logs_btn.set_tooltip_text(Some(&tr("Open folder")));
        logs_dir_row.add_suffix(&open_logs_btn);
        storage_group.add(&logs_dir_row);

        // Connect logs folder button
        let logs_dir_clone = logs_dir.clone();
        open_logs_btn.connect_clicked(move |_| {
            // Ensure directory exists before opening
            if let Err(e) = std::fs::create_dir_all(&logs_dir_clone) {
                tracing::error!("Failed to create logs directory: {}", e);
                return;
            }
            let uri = format!("file://{}", logs_dir_clone.display());
            if let Err(e) =
                gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
            {
                tracing::error!("Failed to open logs directory: {}", e);
            }
        });

        content.append(&storage_group);

        // Backup & Restore group
        let backup_group = adw::PreferencesGroup::builder()
            // TRANSLATORS: Shown as Pango markup, so write "&" as "&amp;"
            .title(tr("Backup &amp; Restore"))
            .build();

        let export_row = adw::ActionRow::builder()
            .title(tr("Export Profiles"))
            .subtitle(tr("Save all profiles to a backup file"))
            .build();
        let export_icon = gtk::Image::from_icon_name("document-save-symbolic");
        export_row.add_suffix(&export_icon);
        let export_btn = gtk::Button::from_icon_name("go-next-symbolic");
        export_btn.set_valign(gtk::Align::Center);
        export_btn.add_css_class("flat");
        export_row.add_suffix(&export_btn);
        backup_group.add(&export_row);

        let import_row = adw::ActionRow::builder()
            .title(tr("Import Profiles"))
            .subtitle(tr("Restore profiles from a backup file"))
            .build();
        let import_icon = gtk::Image::from_icon_name("document-open-symbolic");
        import_row.add_suffix(&import_icon);
        let import_btn = gtk::Button::from_icon_name("go-next-symbolic");
        import_btn.set_valign(gtk::Align::Center);
        import_btn.add_css_class("flat");
        import_row.add_suffix(&import_btn);
        backup_group.add(&import_row);

        // Use the window's export/import so the file format, the script
        // warning and the save path are the same as on the Profiles page
        export_btn.set_action_name(Some("win.export-profiles"));
        export_btn.set_tooltip_text(Some(&tr("Export Profiles")));
        import_btn.set_action_name(Some("win.import-profiles"));
        import_btn.set_tooltip_text(Some(&tr("Import Profiles")));

        content.append(&backup_group);

        // Scheduling group
        let scheduling_group = adw::PreferencesGroup::builder()
            .title(tr("Scheduling"))
            .description(tr("Automatically activate profiles at specific times"))
            .build();

        let scheduling_enabled_row = adw::SwitchRow::builder()
            .title(tr("Enable Scheduled Activation"))
            .subtitle(tr("Automatically switch profiles based on schedule"))
            .active(false)
            .build();
        scheduling_group.add(&scheduling_enabled_row);
        *imp.scheduling_enabled_row.borrow_mut() = Some(scheduling_enabled_row.clone());

        // Connect scheduling enabled change
        let this_for_sched_save = self.downgrade();
        scheduling_enabled_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_sched_save.upgrade() {
                this.save_scheduling_enabled(row.is_active());
            }
        });

        let manage_schedules_row = adw::ActionRow::builder()
            .title(tr("Manage Schedules"))
            .subtitle(tr("Configure time-based profile activations"))
            .activatable(true)
            .build();
        let schedule_icon = gtk::Image::from_icon_name("alarm-symbolic");
        manage_schedules_row.add_suffix(&schedule_icon);
        let schedule_arrow = gtk::Image::from_icon_name("go-next-symbolic");
        manage_schedules_row.add_suffix(&schedule_arrow);
        scheduling_group.add(&manage_schedules_row);

        // Connect manage schedules click - will show schedule management dialog
        let this_for_schedules = self.downgrade();
        manage_schedules_row.connect_activated(move |_| {
            if let Some(this) = this_for_schedules.upgrade() {
                this.show_manage_schedules_dialog();
            }
        });

        content.append(&scheduling_group);

        // Watchdog group
        let watchdog_group = adw::PreferencesGroup::builder()
            .title(tr("Connection Watchdog"))
            .description(tr(
                "Monitor network connectivity and take action on failures",
            ))
            .build();

        let watchdog_enabled_row = adw::SwitchRow::builder()
            .title(tr("Enable Watchdog"))
            .subtitle(tr("Periodically check network connectivity"))
            .active(false)
            .build();
        watchdog_group.add(&watchdog_enabled_row);
        *imp.watchdog_enabled_row.borrow_mut() = Some(watchdog_enabled_row.clone());

        let watchdog_interval_row = adw::SpinRow::builder()
            .title(tr("Check Interval"))
            .subtitle(tr("Seconds between connectivity checks"))
            .adjustment(&gtk::Adjustment::new(30.0, 5.0, 300.0, 5.0, 30.0, 0.0))
            .build();
        watchdog_group.add(&watchdog_interval_row);
        *imp.watchdog_interval_row.borrow_mut() = Some(watchdog_interval_row.clone());

        let watchdog_target_row = adw::EntryRow::builder()
            .title(tr("Ping Target"))
            .text("8.8.8.8")
            .build();
        watchdog_group.add(&watchdog_target_row);
        *imp.watchdog_target_row.borrow_mut() = Some(watchdog_target_row.clone());

        let watchdog_threshold_row = adw::SpinRow::builder()
            .title(tr("Failure Threshold"))
            .subtitle(tr("Failed checks before taking action"))
            .adjustment(&gtk::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0))
            .build();
        watchdog_group.add(&watchdog_threshold_row);
        *imp.watchdog_threshold_row.borrow_mut() = Some(watchdog_threshold_row.clone());

        // The rows map to WatchdogAction by position, never by their text
        let action_labels = [
            // TRANSLATORS: Watchdog failure action: show a notification
            tr("Notify"),
            // TRANSLATORS: Watchdog failure action: reconnect the network
            tr("Reconnect"),
            // TRANSLATORS: Watchdog failure action: switch to the fallback profile
            tr("Switch Profile"),
            // TRANSLATORS: Watchdog failure action; "NetworkManager" is the
            // name of the system service
            tr("Restart NetworkManager"),
        ];
        let action_label_refs: Vec<&str> = action_labels.iter().map(String::as_str).collect();
        let action_model = gtk::StringList::new(&action_label_refs);
        let watchdog_action_row = adw::ComboRow::builder()
            .title(tr("Failure Action"))
            .subtitle(tr("Action when connectivity is lost"))
            .model(&action_model)
            .build();
        watchdog_group.add(&watchdog_action_row);
        *imp.watchdog_action_row.borrow_mut() = Some(watchdog_action_row.clone());

        // TRANSLATORS: No fallback profile selected
        let fallback_model = gtk::StringList::new(&[tr("None").as_str()]);
        let watchdog_fallback_row = adw::ComboRow::builder()
            .title(tr("Fallback Profile"))
            .subtitle(tr("Profile to switch to when connectivity fails"))
            .model(&fallback_model)
            .build();
        watchdog_group.add(&watchdog_fallback_row);
        *imp.watchdog_fallback_row.borrow_mut() = Some(watchdog_fallback_row.clone());

        // Connect watchdog save handlers; each one saves only its own field
        let this_for_wd = self.downgrade();
        watchdog_enabled_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_wd.upgrade() {
                let enabled = row.is_active();
                this.update_app_config(|config| config.watchdog.enabled = enabled);
            }
        });
        let this_for_wd2 = self.downgrade();
        watchdog_interval_row.connect_value_notify(move |row| {
            if let Some(this) = this_for_wd2.upgrade() {
                let secs = row.value() as u32;
                this.update_app_config(|config| config.watchdog.check_interval_secs = secs);
            }
        });
        let this_for_wd3 = self.downgrade();
        watchdog_threshold_row.connect_value_notify(move |row| {
            if let Some(this) = this_for_wd3.upgrade() {
                let threshold = row.value() as u32;
                this.update_app_config(|config| config.watchdog.failure_threshold = threshold);
            }
        });
        let this_for_wd4 = self.downgrade();
        watchdog_action_row.connect_selected_notify(move |row| {
            if let Some(this) = this_for_wd4.upgrade() {
                let action = match row.selected() {
                    1 => WatchdogAction::Reconnect,
                    2 => WatchdogAction::SwitchProfile,
                    3 => WatchdogAction::RestartNetworkManager,
                    _ => WatchdogAction::Notify,
                };
                this.update_app_config(|config| config.watchdog.failure_action = action);
                this.update_watchdog_fallback_sensitivity();
            }
        });
        let this_for_wd5 = self.downgrade();
        watchdog_target_row.connect_changed(move |row| {
            if let Some(this) = this_for_wd5.upgrade() {
                let target = row.text().to_string();
                this.update_app_config(|config| config.watchdog.ping_target = target);
            }
        });
        let this_for_wd6 = self.downgrade();
        watchdog_fallback_row.connect_selected_notify(move |row| {
            if let Some(this) = this_for_wd6.upgrade() {
                let selected = row.selected();
                let profile_id = if selected == 0 {
                    None
                } else {
                    this.imp()
                        .profiles
                        .borrow()
                        .get((selected - 1) as usize)
                        .map(|profile| profile.id().to_string())
                };
                this.update_app_config(|config| config.watchdog.fallback_profile_id = profile_id);
            }
        });

        content.append(&watchdog_group);

        // Security group
        let security_group = adw::PreferencesGroup::builder()
            .title(tr("Security"))
            .description(tr("Script execution and profile security settings"))
            .build();

        // The rows map to SandboxMode by position. Bubblewrap and Firejail
        // are program names and stay untranslated.
        let no_sandboxing = tr("No Sandboxing");
        let sandbox_model =
            gtk::StringList::new(&[no_sandboxing.as_str(), "Bubblewrap", "Firejail"]);
        let sandbox_row = adw::ComboRow::builder()
            .title(tr("Script Sandboxing"))
            .subtitle(tr("Method for isolating script execution"))
            .model(&sandbox_model)
            .build();
        security_group.add(&sandbox_row);
        *imp.sandbox_row.borrow_mut() = Some(sandbox_row.clone());

        // Profile encryption: the passphrase is never written to disk. The
        // user unlocks it at startup (or from this page) to load profiles.
        let encryption_row = adw::SwitchRow::builder()
            .title(tr("Profile Encryption"))
            .subtitle(tr("Encrypt saved profiles with a passphrase"))
            .active(false)
            .build();
        security_group.add(&encryption_row);
        *imp.encryption_row.borrow_mut() = Some(encryption_row.clone());

        let encryption_change_row = adw::ButtonRow::builder()
            .title(tr("Change Passphrase…"))
            .build();
        encryption_change_row.set_visible(false);
        security_group.add(&encryption_change_row);
        *imp.encryption_change_row.borrow_mut() = Some(encryption_change_row.clone());

        let encryption_unlock_row = adw::ButtonRow::builder()
            .title(tr("Unlock Profiles…"))
            .build();
        encryption_unlock_row.set_visible(false);
        security_group.add(&encryption_unlock_row);
        *imp.encryption_unlock_row.borrow_mut() = Some(encryption_unlock_row.clone());

        // Connect security save handler
        let this_for_sec = self.downgrade();
        sandbox_row.connect_selected_notify(move |row| {
            if let Some(this) = this_for_sec.upgrade() {
                let sandbox_mode = match row.selected() {
                    1 => SandboxMode::Bubblewrap,
                    2 => SandboxMode::Firejail,
                    _ => SandboxMode::None,
                };
                this.update_app_config(|config| config.sandbox_mode = sandbox_mode);
            }
        });

        let this_for_enc = self.downgrade();
        encryption_row.connect_active_notify(move |row| {
            let Some(this) = this_for_enc.upgrade() else {
                return;
            };
            if this.imp().loading.get() {
                return;
            }

            if row.is_active() {
                // Reset the switch until a passphrase has actually been set.
                this.imp().loading.set(true);
                row.set_active(false);
                this.imp().loading.set(false);

                let this_for_dialog = this.clone();
                crate::ui::encryption_dialog::show_passphrase_dialog(
                    this.app().and_then(|app| app.active_window()).as_ref(),
                    &tr("Enable Profile Encryption"),
                    &tr("Choose a passphrase. It is never saved to disk, so you will need to enter it each time you start the app."),
                    true,
                    &tr("Enable"),
                    move |passphrase| {
                        let app = this_for_dialog
                            .app()
                            .ok_or_else(|| tr("Could not access the application."))?;
                        let token = crate::services::make_check_token(passphrase)
                            .map_err(|e| e.user_message())?;

                        let mut config = app.config();
                        config.encrypt_profiles = true;
                        config.encryption_key = Some(passphrase.to_string());
                        config.encryption_check = Some(token);
                        app.update_config(config);

                        if let Some(store) = app.data_store() {
                            store.save_profiles_cache_now();
                        }

                        this_for_dialog.update_encryption_rows();
                        Ok(())
                    },
                );
            } else {
                // Ask before writing the profiles back in plain text.
                let dialog = adw::AlertDialog::new(
                    Some(&tr("Disable Profile Encryption?")),
                    Some(&tr("Saved profiles will be written to disk without encryption.")),
                );
                dialog.add_response("cancel", &tr("Cancel"));
                dialog.add_response("disable", &tr("Disable"));
                dialog.set_response_appearance(
                    "disable",
                    adw::ResponseAppearance::Destructive,
                );
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let this_for_dialog = this.clone();
                let row_weak = row.downgrade();
                dialog.connect_response(None, move |_dialog, response| {
                    if response == "disable" {
                        if let Some(app) = this_for_dialog.app() {
                            let mut config = app.config();
                            config.encrypt_profiles = false;
                            config.encryption_key = None;
                            config.encryption_check = None;
                            app.update_config(config);
                            if let Some(store) = app.data_store() {
                                store.save_profiles_cache_now();
                            }
                            if let Some(win) = app
                                .active_window()
                                .and_downcast::<crate::ui::MainWindow>()
                            {
                                win.reload_profiles_from_cache();
                            }
                        }
                        this_for_dialog.update_encryption_rows();
                    } else if let Some(row) = row_weak.upgrade() {
                        this_for_dialog.imp().loading.set(true);
                        row.set_active(true);
                        this_for_dialog.imp().loading.set(false);
                    }
                });

                if let Some(app) = this.app() {
                    dialog.present(app.active_window().as_ref());
                }
            }
        });

        let this_for_change = self.downgrade();
        encryption_change_row.connect_activated(move |_| {
            let Some(this) = this_for_change.upgrade() else {
                return;
            };
            let this_for_dialog = this.clone();
            crate::ui::encryption_dialog::show_passphrase_dialog(
                this.app().and_then(|app| app.active_window()).as_ref(),
                &tr("Change Encryption Passphrase"),
                &tr("Choose a new passphrase for your saved profiles."),
                true,
                &tr("Change"),
                move |passphrase| {
                    let app = this_for_dialog
                        .app()
                        .ok_or_else(|| tr("Could not access the application."))?;
                    let token = crate::services::make_check_token(passphrase)
                        .map_err(|e| e.user_message())?;

                    let mut config = app.config();
                    config.encrypt_profiles = true;
                    config.encryption_key = Some(passphrase.to_string());
                    config.encryption_check = Some(token);
                    app.update_config(config);

                    if let Some(store) = app.data_store() {
                        store.save_profiles_cache_now();
                    }
                    this_for_dialog.update_encryption_rows();
                    Ok(())
                },
            );
        });

        let this_for_unlock = self.downgrade();
        encryption_unlock_row.connect_activated(move |_| {
            let Some(this) = this_for_unlock.upgrade() else {
                return;
            };
            let this_for_dialog = this.clone();
            crate::ui::encryption_dialog::show_passphrase_dialog(
                this.app().and_then(|app| app.active_window()).as_ref(),
                &tr("Unlock Profiles"),
                &tr("Enter the passphrase you chose when encryption was enabled."),
                false,
                &tr("Unlock"),
                move |passphrase| {
                    let app = this_for_dialog
                        .app()
                        .ok_or_else(|| tr("Could not access the application."))?;
                    let mut config = app.config();

                    if let Some(store) = app.data_store() {
                        if !store.verify_encryption_passphrase(
                            passphrase,
                            config.encryption_check.as_deref(),
                        ) {
                            return Err(tr("That passphrase is not correct."));
                        }
                    }

                    config.encryption_key = Some(passphrase.to_string());
                    app.update_config(config);

                    if let Some(store) = app.data_store() {
                        store.reload_profiles_cache();
                    }
                    if let Some(win) = app.active_window().and_downcast::<crate::ui::MainWindow>() {
                        win.reload_profiles_from_cache();
                    }
                    this_for_dialog.update_encryption_rows();
                    Ok(())
                },
            );
        });

        content.append(&security_group);

        // Hotkeys group
        let hotkeys_group = adw::PreferencesGroup::builder()
            .title(tr("Keyboard Shortcuts"))
            .description(tr("Quick access shortcuts for profile switching"))
            .build();

        let hotkeys_enabled_row = adw::SwitchRow::builder()
            .title(tr("Enable Profile Hotkeys"))
            .subtitle(tr("Use keyboard shortcuts to switch profiles"))
            .active(false)
            .build();
        hotkeys_group.add(&hotkeys_enabled_row);
        *imp.hotkeys_enabled_row.borrow_mut() = Some(hotkeys_enabled_row.clone());

        // Connect hotkeys enabled save
        let this_for_hk = self.downgrade();
        hotkeys_enabled_row.connect_active_notify(move |row| {
            if let Some(this) = this_for_hk.upgrade() {
                this.save_hotkeys_enabled(row.is_active());
            }
        });

        let manage_hotkeys_row = adw::ActionRow::builder()
            .title(tr("Configure Hotkeys"))
            .subtitle(tr("Assign keyboard shortcuts to profiles"))
            .activatable(true)
            .build();
        let hotkey_icon = gtk::Image::from_icon_name("input-keyboard-symbolic");
        manage_hotkeys_row.add_suffix(&hotkey_icon);
        let hotkey_arrow = gtk::Image::from_icon_name("go-next-symbolic");
        manage_hotkeys_row.add_suffix(&hotkey_arrow);
        hotkeys_group.add(&manage_hotkeys_row);

        // Connect manage hotkeys
        let this_for_hotkeys = self.downgrade();
        manage_hotkeys_row.connect_activated(move |_| {
            if let Some(this) = this_for_hotkeys.upgrade() {
                this.show_manage_hotkeys_dialog();
            }
        });

        content.append(&hotkeys_group);

        // About group
        let about_group = adw::PreferencesGroup::builder().title(tr("About")).build();

        let version_row = adw::ActionRow::builder()
            // TRANSLATORS: Application version row
            .title(tr("Version"))
            .subtitle(env!("CARGO_PKG_VERSION"))
            .build();
        about_group.add(&version_row);

        let project_row = adw::ActionRow::builder()
            .title(tr("Project Website"))
            .subtitle("https://chrisdaggas.com")
            .activatable(true)
            .build();
        let link_icon = gtk::Image::from_icon_name("web-browser-symbolic");
        project_row.add_suffix(&link_icon);
        let project_arrow = gtk::Image::from_icon_name("go-next-symbolic");
        project_row.add_suffix(&project_arrow);
        about_group.add(&project_row);

        // Connect project website click
        project_row.connect_activated(|_| {
            if let Err(e) = gio::AppInfo::launch_default_for_uri(
                "https://chrisdaggas.com",
                None::<&gio::AppLaunchContext>,
            ) {
                tracing::error!("Failed to open project website: {}", e);
            }
        });

        let report_issue_row = adw::ActionRow::builder()
            .title(tr("Report an Issue"))
            .subtitle("https://github.com/christosdaggas/Network-Manager/issues")
            .activatable(true)
            .build();
        let issue_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        report_issue_row.add_suffix(&issue_icon);
        let issue_arrow = gtk::Image::from_icon_name("go-next-symbolic");
        report_issue_row.add_suffix(&issue_arrow);
        about_group.add(&report_issue_row);

        // Connect report issue click
        report_issue_row.connect_activated(|_| {
            if let Err(e) = gio::AppInfo::launch_default_for_uri(
                "https://github.com/christosdaggas/Network-Manager/issues",
                None::<&gio::AppLaunchContext>,
            ) {
                tracing::error!("Failed to open issues page: {}", e);
            }
        });

        content.append(&about_group);
    }

    /// Get the profiles directory path (display only - actual profiles are in cache)
    fn get_profiles_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/daggas")
            .join(CONFIG_DIR_NAME)
    }

    /// Get the profiles cache file path
    fn get_profiles_cache_file() -> PathBuf {
        Self::get_profiles_dir().join("profiles_cache.json")
    }

    /// Get the logs file path (logs are stored in config directory as JSON)
    fn get_logs_dir() -> PathBuf {
        Self::get_profiles_dir()
    }

    /// Get profile names and IDs from the profiles cache.
    /// Returns (display_names, profile_ids) vectors.
    fn get_profile_names_and_ids() -> (Vec<String>, Vec<String>) {
        let cache_file = Self::get_profiles_cache_file();
        if let Ok(content) = std::fs::read_to_string(&cache_file) {
            if let Ok(profiles) = serde_json::from_str::<Vec<Profile>>(&content) {
                let names: Vec<String> = profiles.iter().map(|p| p.name().to_string()).collect();
                let ids: Vec<String> = profiles.iter().map(|p| p.id().to_string()).collect();
                if !names.is_empty() {
                    return (names, ids);
                }
            }
        }
        // Return default placeholder if no profiles found
        (vec![tr("(No profiles available)")], vec![String::new()])
    }

    /// Get the currently selected theme.
    pub fn selected_theme(&self) -> ThemePreference {
        let imp = self.imp();
        if let Some(theme_row) = imp.theme_row.borrow().as_ref() {
            match theme_row.selected() {
                0 => ThemePreference::System,
                1 => ThemePreference::Light,
                2 => ThemePreference::Dark,
                _ => ThemePreference::System,
            }
        } else {
            ThemePreference::System
        }
    }

    /// Set the theme selection.
    pub fn set_theme(&self, theme: ThemePreference) {
        let imp = self.imp();
        if let Some(theme_row) = imp.theme_row.borrow().as_ref() {
            let position = match theme {
                ThemePreference::System => 0,
                ThemePreference::Light => 1,
                ThemePreference::Dark => 2,
            };
            theme_row.set_selected(position);
        }
    }

    /// Check if auto-switch is enabled.
    pub fn auto_switch_enabled(&self) -> bool {
        let imp = self.imp();
        imp.auto_switch_row
            .borrow()
            .as_ref()
            .map_or(false, |r| r.is_active())
    }

    /// Get the auto-switch interval in seconds.
    pub fn auto_switch_interval(&self) -> u32 {
        let imp = self.imp();
        imp.auto_switch_interval
            .borrow()
            .as_ref()
            .map_or(30, |r| r.value() as u32)
    }

    /// Check if auto-switch is allowed to run scripts.
    pub fn auto_switch_allow_scripts(&self) -> bool {
        let imp = self.imp();
        imp.auto_switch_allow_scripts_row
            .borrow()
            .as_ref()
            .map_or(false, |r| r.is_active())
    }

    /// Check if notifications are enabled.
    pub fn notifications_enabled(&self) -> bool {
        let imp = self.imp();
        imp.notifications_row
            .borrow()
            .as_ref()
            .map_or(true, |r| r.is_active())
    }

    /// Show the manage schedules dialog.
    fn show_manage_schedules_dialog(&self) {
        let Some(root) = self.root() else { return };
        let Some(window) = root.downcast_ref::<gtk::Window>() else {
            return;
        };
        let Some(app) = self.app() else { return };

        let dialog = dialog_window::new(&tr("Manage Schedules"), 500, 400);

        // Header bar with close and add buttons
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some(&tr("Add Schedule")));
        add_btn.add_css_class("flat");
        header.pack_start(&add_btn);

        // Main content
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);

        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let schedules_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        // Saved schedules
        let schedules_list = gtk::ListBox::new();
        schedules_list.set_selection_mode(gtk::SelectionMode::None);
        schedules_list.add_css_class("boxed-list");
        schedules_list.set_valign(gtk::Align::Start);
        schedules_box.append(&schedules_list);

        // Status page for empty state
        let empty_page = adw::StatusPage::builder()
            .icon_name("alarm-symbolic")
            .title(tr("No Schedules"))
            .description(tr("Add a schedule to automatically activate profiles at specific times.\n\nClick the + button to create your first schedule."))
            .build();
        schedules_box.append(&empty_page);

        Self::populate_schedules(&schedules_list, &empty_page, &app);

        scrolled.set_child(Some(&schedules_box));
        content.append(&scrolled);

        // Help text
        let help_label = gtk::Label::builder()
            .label(tr("Schedules use cron format: minute hour day-of-month month day-of-week\nExample: \"30 9 * * 1-5\" = 9:30 AM on weekdays"))
            .wrap(true)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        help_label.add_css_class("dim-label");
        help_label.add_css_class("caption");
        content.append(&help_label);

        dialog.set_content(Some(&content));

        // Connect add button
        let dialog_weak = glib::WeakRef::new();
        dialog_weak.set(Some(&dialog));
        let list_weak = schedules_list.downgrade();
        let empty_weak = empty_page.downgrade();
        add_btn.connect_clicked(move |_| {
            if let (Some(dlg), Some(list), Some(empty_page)) = (
                dialog_weak.upgrade(),
                list_weak.upgrade(),
                empty_weak.upgrade(),
            ) {
                Self::show_add_schedule_dialog(&dlg, &list, &empty_page);
            }
        });

        dialog_window::present(&dialog, Some(window));
    }

    /// Rebuild the schedule list from the saved configuration.
    fn populate_schedules(
        list: &gtk::ListBox,
        empty_page: &adw::StatusPage,
        app: &crate::application::Application,
    ) {
        list.remove_all();
        let schedules = app.config().schedules;
        let (profile_names, profile_ids) = Self::get_profile_names_and_ids();

        for schedule in &schedules {
            let profile_name = profile_ids
                .iter()
                .position(|id| *id == schedule.profile_id)
                .and_then(|i| profile_names.get(i).cloned())
                .unwrap_or_else(|| tr("Unknown profile"));
            let state = if schedule.enabled {
                String::new()
            } else {
                // TRANSLATORS: Appended to a schedule or hotkey that is turned off
                format!(" · {}", tr("disabled"))
            };
            let row = adw::ActionRow::builder()
                .title(
                    schedule
                        .description
                        .as_deref()
                        .unwrap_or(&schedule.cron_expression),
                )
                .subtitle(format!(
                    "{} · {}{}",
                    schedule.cron_expression, profile_name, state
                ))
                .use_markup(false)
                .build();

            let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
            delete_btn.set_valign(gtk::Align::Center);
            delete_btn.add_css_class("flat");
            delete_btn.set_tooltip_text(Some(&tr("Delete Schedule")));
            let schedule_id = schedule.id.clone();
            let list_weak = list.downgrade();
            let empty_weak = empty_page.downgrade();
            let app_weak = app.downgrade();
            delete_btn.connect_clicked(move |_| {
                let (Some(list), Some(empty_page), Some(app)) = (
                    list_weak.upgrade(),
                    empty_weak.upgrade(),
                    app_weak.upgrade(),
                ) else {
                    return;
                };
                let mut config = app.config();
                config.schedules.retain(|s| s.id != schedule_id);
                app.update_config(config);
                Self::populate_schedules(&list, &empty_page, &app);
            });
            row.add_suffix(&delete_btn);
            list.append(&row);
        }

        list.set_visible(!schedules.is_empty());
        empty_page.set_visible(schedules.is_empty());
    }

    /// Show the add schedule dialog with form fields.
    fn show_add_schedule_dialog(
        parent: &adw::Window,
        list: &gtk::ListBox,
        empty_page: &adw::StatusPage,
    ) {
        let dialog = dialog_window::new(&tr("Add Schedule"), 400, 450);

        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        let save_btn = gtk::Button::with_label(&tr("Save"));
        save_btn.add_css_class("suggested-action");
        header.pack_end(&save_btn);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);

        let prefs_page = adw::PreferencesPage::new();

        // Schedule info group
        let info_group = adw::PreferencesGroup::builder()
            .title(tr("Schedule Information"))
            .build();

        // TRANSLATORS: Schedule description entry
        let name_row = adw::EntryRow::builder().title(tr("Description")).build();
        // TRANSLATORS: Default description for a new schedule
        name_row.set_text(&tr("Daily Work Profile"));
        info_group.add(&name_row);

        // Profile selection - load from application's profiles
        let (profile_names, profile_ids) = Self::get_profile_names_and_ids();
        let profile_model =
            gtk::StringList::new(&profile_names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let profile_row = adw::ComboRow::builder()
            .title(tr("Profile"))
            .subtitle(tr("Profile to activate on schedule"))
            .model(&profile_model)
            .build();
        info_group.add(&profile_row);

        prefs_page.add(&info_group);

        // Time group
        let time_group = adw::PreferencesGroup::builder()
            .title(tr("Activation Time"))
            .build();

        let hour_row = adw::SpinRow::builder()
            .title(tr("Hour (0-23)"))
            .adjustment(&gtk::Adjustment::new(9.0, 0.0, 23.0, 1.0, 1.0, 0.0))
            .build();
        time_group.add(&hour_row);

        let minute_row = adw::SpinRow::builder()
            .title(tr("Minute (0-59)"))
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 59.0, 1.0, 5.0, 0.0))
            .build();
        time_group.add(&minute_row);

        prefs_page.add(&time_group);

        // Days group
        let days_group = adw::PreferencesGroup::builder()
            .title(tr("Days of Week"))
            .description(tr("Select days when the schedule should run"))
            .build();

        let mon_row = adw::SwitchRow::builder()
            .title(tr("Monday"))
            .active(true)
            .build();
        let tue_row = adw::SwitchRow::builder()
            .title(tr("Tuesday"))
            .active(true)
            .build();
        let wed_row = adw::SwitchRow::builder()
            .title(tr("Wednesday"))
            .active(true)
            .build();
        let thu_row = adw::SwitchRow::builder()
            .title(tr("Thursday"))
            .active(true)
            .build();
        let fri_row = adw::SwitchRow::builder()
            .title(tr("Friday"))
            .active(true)
            .build();
        let sat_row = adw::SwitchRow::builder()
            .title(tr("Saturday"))
            .active(false)
            .build();
        let sun_row = adw::SwitchRow::builder()
            .title(tr("Sunday"))
            .active(false)
            .build();

        days_group.add(&mon_row);
        days_group.add(&tue_row);
        days_group.add(&wed_row);
        days_group.add(&thu_row);
        days_group.add(&fri_row);
        days_group.add(&sat_row);
        days_group.add(&sun_row);

        prefs_page.add(&days_group);

        // Options group
        let options_group = adw::PreferencesGroup::builder()
            .title(tr("Options"))
            .build();

        let enabled_row = adw::SwitchRow::builder()
            .title(tr("Enabled"))
            .subtitle(tr("Schedule will trigger when enabled"))
            .active(true)
            .build();
        options_group.add(&enabled_row);

        let one_shot_row = adw::SwitchRow::builder()
            // TRANSLATORS: A schedule that runs only once
            .title(tr("One-shot"))
            .subtitle(tr("Run once then disable"))
            .active(false)
            .build();
        options_group.add(&one_shot_row);

        prefs_page.add(&options_group);

        content.append(&prefs_page);
        dialog.set_content(Some(&content));

        // Connect save button
        let dialog_weak = glib::WeakRef::new();
        dialog_weak.set(Some(&dialog));
        let parent_weak = glib::WeakRef::new();
        parent_weak.set(Some(parent));
        // Clone profile_ids for the closure
        let profile_ids_clone = profile_ids.clone();
        let profile_row_weak = profile_row.downgrade();
        let list_weak = list.downgrade();
        let empty_weak = empty_page.downgrade();
        save_btn.connect_clicked(move |_| {
            let hour = hour_row.value() as u32;
            let minute = minute_row.value() as u32;
            let description = name_row.text().to_string();
            let is_enabled = enabled_row.is_active();
            let is_one_shot = one_shot_row.is_active();

            // Get selected profile ID
            let selected_profile_id = profile_row_weak
                .upgrade()
                .map(|row| row.selected() as usize)
                .and_then(|idx| profile_ids_clone.get(idx).cloned())
                .unwrap_or_default();

            // Build days string
            let mut days = Vec::new();
            if mon_row.is_active() {
                days.push("1");
            }
            if tue_row.is_active() {
                days.push("2");
            }
            if wed_row.is_active() {
                days.push("3");
            }
            if thu_row.is_active() {
                days.push("4");
            }
            if fri_row.is_active() {
                days.push("5");
            }
            if sat_row.is_active() {
                days.push("6");
            }
            if sun_row.is_active() {
                days.push("0");
            }

            let days_str = if days.len() == 7 {
                "*".to_string()
            } else {
                days.join(",")
            };
            let cron = format!("{} {} * * {}", minute, hour, days_str);

            // Create schedule entry with actual profile ID
            let schedule = ScheduleEntry {
                id: uuid::Uuid::new_v4().to_string(),
                profile_id: selected_profile_id,
                cron_expression: cron.clone(),
                enabled: is_enabled,
                one_shot: is_one_shot,
                description: if description.is_empty() {
                    None
                } else {
                    Some(description.clone())
                },
            };

            // Save to config via parent dialog -> window -> app
            if let Some(parent_dlg) = parent_weak.upgrade() {
                if let Some(root) = parent_dlg.root() {
                    if let Some(window) = root.downcast_ref::<gtk::Window>() {
                        if let Some(app) = window.application() {
                            if let Some(network_app) =
                                app.downcast_ref::<crate::application::Application>()
                            {
                                let mut config = network_app.config();
                                config.schedules.push(schedule);
                                network_app.update_config(config);
                                info!("Schedule saved: {}", cron);
                                if let (Some(list), Some(empty_page)) =
                                    (list_weak.upgrade(), empty_weak.upgrade())
                                {
                                    Self::populate_schedules(&list, &empty_page, network_app);
                                }
                            }
                        }
                    }
                }
            }

            // Close dialog
            if let Some(dlg) = dialog_weak.upgrade() {
                dlg.close();
            }
        });

        dialog_window::present(&dialog, Some(parent));
    }

    /// Show the manage hotkeys dialog.
    fn show_manage_hotkeys_dialog(&self) {
        let Some(root) = self.root() else { return };
        let Some(window) = root.downcast_ref::<gtk::Window>() else {
            return;
        };
        let Some(app) = self.app() else { return };

        let dialog = dialog_window::new(&tr("Configure Hotkeys"), 500, 450);

        // Header bar
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some(&tr("Add Hotkey")));
        add_btn.add_css_class("flat");
        header.pack_start(&add_btn);

        // Main content
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);

        let prefs_page = adw::PreferencesPage::new();

        // Info group
        let info_group = adw::PreferencesGroup::builder()
            .title(tr("Profile Hotkeys"))
            .description(tr("Global keyboard shortcuts for quick profile switching"))
            .build();

        // Saved hotkeys
        let hotkeys_list = gtk::ListBox::new();
        hotkeys_list.set_selection_mode(gtk::SelectionMode::None);
        hotkeys_list.add_css_class("boxed-list");
        info_group.add(&hotkeys_list);

        let empty_label = gtk::Label::new(Some(&tr(
            "No hotkeys configured. Click the + button to add one.",
        )));
        empty_label.add_css_class("dim-label");
        empty_label.set_wrap(true);
        empty_label.set_margin_top(12);
        empty_label.set_margin_bottom(12);
        info_group.add(&empty_label);

        Self::populate_hotkeys(&hotkeys_list, &empty_label, &app);

        prefs_page.add(&info_group);

        // Help group
        let help_group = adw::PreferencesGroup::builder()
            .title(tr("How to Use"))
            .build();

        let help_row = adw::ActionRow::builder()
            .title(tr("About Hotkeys"))
            .subtitle(tr("Hotkeys work globally when the app is running. Press the shortcut to instantly switch to the assigned profile."))
            .build();
        help_group.add(&help_row);

        let note_row = adw::ActionRow::builder()
            // TRANSLATORS: Heading of a warning about conflicting shortcuts
            .title(tr("Note"))
            .subtitle(tr(
                "Some shortcuts may conflict with system or other application shortcuts.",
            ))
            .build();
        let warning_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        note_row.add_prefix(&warning_icon);
        help_group.add(&note_row);

        prefs_page.add(&help_group);

        content.append(&prefs_page);
        dialog.set_content(Some(&content));

        // Connect add button
        let dialog_weak = glib::WeakRef::new();
        dialog_weak.set(Some(&dialog));
        let list_weak = hotkeys_list.downgrade();
        let empty_weak = empty_label.downgrade();
        add_btn.connect_clicked(move |_| {
            if let (Some(dlg), Some(list), Some(empty_label)) = (
                dialog_weak.upgrade(),
                list_weak.upgrade(),
                empty_weak.upgrade(),
            ) {
                Self::show_add_hotkey_dialog(&dlg, &list, &empty_label);
            }
        });

        dialog_window::present(&dialog, Some(window));
    }

    /// Rebuild the hotkey list from the saved configuration.
    fn populate_hotkeys(
        list: &gtk::ListBox,
        empty_label: &gtk::Label,
        app: &crate::application::Application,
    ) {
        list.remove_all();
        let hotkeys = app.config().hotkeys;

        for hotkey in &hotkeys {
            let state = if hotkey.enabled {
                String::new()
            } else {
                // TRANSLATORS: Appended to a schedule or hotkey that is turned off
                format!(" · {}", tr("disabled"))
            };
            let row = adw::ActionRow::builder()
                .title(&hotkey.profile_name)
                .subtitle(format!("{}{}", hotkey.shortcut_string(), state))
                .use_markup(false)
                .build();

            let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
            delete_btn.set_valign(gtk::Align::Center);
            delete_btn.add_css_class("flat");
            delete_btn.set_tooltip_text(Some(&tr("Delete Hotkey")));
            let hotkey_id = hotkey.id.clone();
            let list_weak = list.downgrade();
            let empty_weak = empty_label.downgrade();
            let app_weak = app.downgrade();
            delete_btn.connect_clicked(move |_| {
                let (Some(list), Some(empty_label), Some(app)) = (
                    list_weak.upgrade(),
                    empty_weak.upgrade(),
                    app_weak.upgrade(),
                ) else {
                    return;
                };
                let mut config = app.config();
                config.hotkeys.retain(|h| h.id != hotkey_id);
                app.update_config(config);
                Self::populate_hotkeys(&list, &empty_label, &app);
            });
            row.add_suffix(&delete_btn);
            list.append(&row);
        }

        list.set_visible(!hotkeys.is_empty());
        empty_label.set_visible(hotkeys.is_empty());
    }

    /// Show dialog to add a new hotkey.
    fn show_add_hotkey_dialog(parent: &adw::Window, list: &gtk::ListBox, empty_label: &gtk::Label) {
        let dialog = dialog_window::new(&tr("Add Hotkey"), 350, 300);

        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        let save_btn = gtk::Button::with_label(&tr("Save"));
        save_btn.add_css_class("suggested-action");
        header.pack_end(&save_btn);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);

        let prefs_page = adw::PreferencesPage::new();

        let group = adw::PreferencesGroup::builder()
            .title(tr("Hotkey Configuration"))
            .build();

        // Profile selection - load from actual profiles
        let (profile_names, profile_ids) = Self::get_profile_names_and_ids();
        let profile_model =
            gtk::StringList::new(&profile_names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let profile_row = adw::ComboRow::builder()
            .title(tr("Profile"))
            .subtitle(tr("Profile to activate with this hotkey"))
            .model(&profile_model)
            .build();
        group.add(&profile_row);

        // Modifier keys. Only the row titles are translated; the modifier
        // names saved with the hotkey below stay in English.
        let ctrl_row = adw::SwitchRow::builder()
            // TRANSLATORS: Keyboard modifier key name
            .title(tr("Ctrl"))
            .active(true)
            .build();
        group.add(&ctrl_row);

        let alt_row = adw::SwitchRow::builder()
            // TRANSLATORS: Keyboard modifier key name
            .title(tr("Alt"))
            .active(true)
            .build();
        group.add(&alt_row);

        let shift_row = adw::SwitchRow::builder()
            // TRANSLATORS: Keyboard modifier key name
            .title(tr("Shift"))
            .active(false)
            .build();
        group.add(&shift_row);

        let super_row = adw::SwitchRow::builder()
            // TRANSLATORS: Keyboard modifier key name (the Windows/logo key)
            .title(tr("Super"))
            .active(false)
            .build();
        group.add(&super_row);

        // Key entry
        // TRANSLATORS: The key pressed together with the modifiers
        let key_row = adw::EntryRow::builder().title(tr("Key")).build();
        key_row.set_text("1");
        group.add(&key_row);

        prefs_page.add(&group);
        content.append(&prefs_page);

        dialog.set_content(Some(&content));

        // Connect save
        let dialog_weak = glib::WeakRef::new();
        dialog_weak.set(Some(&dialog));
        let parent_weak = glib::WeakRef::new();
        parent_weak.set(Some(parent));
        let profile_names_clone = profile_names.clone();
        let profile_ids_clone = profile_ids.clone();
        let list_weak = list.downgrade();
        let empty_weak = empty_label.downgrade();
        save_btn.connect_clicked(move |_| {
            let mut modifiers = Vec::new();
            if ctrl_row.is_active() {
                modifiers.push("Ctrl".to_string());
            }
            if alt_row.is_active() {
                modifiers.push("Alt".to_string());
            }
            if shift_row.is_active() {
                modifiers.push("Shift".to_string());
            }
            if super_row.is_active() {
                modifiers.push("Super".to_string());
            }

            let key = key_row.text().to_string();
            let profile_idx = profile_row.selected() as usize;

            // Get profile name and ID from loaded profiles
            let profile_name = profile_names_clone
                .get(profile_idx)
                .cloned()
                // TRANSLATORS: Profile name shown when the hotkey's profile is missing
                .unwrap_or_else(|| tr("Unknown"));
            let profile_id = profile_ids_clone
                .get(profile_idx)
                .cloned()
                .unwrap_or_default();

            // Create hotkey entry with actual profile ID
            let hotkey = HotkeyEntry {
                id: uuid::Uuid::new_v4().to_string(),
                profile_id,
                profile_name: profile_name.clone(),
                modifiers,
                key: key.clone(),
                enabled: true,
            };

            let shortcut = hotkey.shortcut_string();

            // Save to config via parent dialog -> window -> app
            if let Some(parent_dlg) = parent_weak.upgrade() {
                if let Some(root) = parent_dlg.root() {
                    if let Some(window) = root.downcast_ref::<gtk::Window>() {
                        if let Some(app) = window.application() {
                            if let Some(network_app) =
                                app.downcast_ref::<crate::application::Application>()
                            {
                                let mut config = network_app.config();
                                config.hotkeys.push(hotkey);
                                network_app.update_config(config);
                                info!("Hotkey saved: {} -> {}", shortcut, profile_name);
                                if let (Some(list), Some(empty_label)) =
                                    (list_weak.upgrade(), empty_weak.upgrade())
                                {
                                    Self::populate_hotkeys(&list, &empty_label, network_app);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(dlg) = dialog_weak.upgrade() {
                dlg.close();
            }
        });

        dialog_window::present(&dialog, Some(parent));
    }

    /// Show the auto-switch rules management dialog.
    fn show_manage_rules_dialog(&self) {
        let Some(root) = self.root() else { return };
        let Some(window) = root.downcast_ref::<gtk::Window>() else {
            return;
        };

        let dialog = dialog_window::new(&tr("Auto-Switch Rules"), 550, 450);

        // Header bar
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        // Main content
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);

        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let rules_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        // Info about rule types
        let rules_group = adw::PreferencesGroup::builder()
            .title(tr("Available Rule Types"))
            .description(tr("Rules are stored with each profile"))
            .build();

        let ssid_row = adw::ActionRow::builder()
            .title(tr("Wi-Fi SSID"))
            .subtitle(tr("Activate when connected to a specific network"))
            .build();
        let ssid_icon = gtk::Image::from_icon_name("network-wireless-symbolic");
        ssid_row.add_prefix(&ssid_icon);
        rules_group.add(&ssid_row);

        let gateway_row = adw::ActionRow::builder()
            .title(tr("Gateway MAC"))
            .subtitle(tr("Match by router MAC address"))
            .build();
        let gw_icon = gtk::Image::from_icon_name("network-wired-symbolic");
        gateway_row.add_prefix(&gw_icon);
        rules_group.add(&gateway_row);

        let ping_row = adw::ActionRow::builder()
            .title(tr("Ping Target"))
            .subtitle(tr("Activate when a host is reachable"))
            .build();
        let ping_icon = gtk::Image::from_icon_name("network-server-symbolic");
        ping_row.add_prefix(&ping_icon);
        rules_group.add(&ping_row);

        let time_row = adw::ActionRow::builder()
            .title(tr("Time Window"))
            .subtitle(tr("Activate during specific hours/days"))
            .build();
        let time_icon = gtk::Image::from_icon_name("appointment-soon-symbolic");
        time_row.add_prefix(&time_icon);
        rules_group.add(&time_row);

        let interface_row = adw::ActionRow::builder()
            .title(tr("Interface State"))
            .subtitle(tr("Match network interface up/down state"))
            .build();
        let if_icon = gtk::Image::from_icon_name("preferences-system-network-symbolic");
        interface_row.add_prefix(&if_icon);
        rules_group.add(&interface_row);

        rules_box.append(&rules_group);

        // Help text
        let help_box = adw::Clamp::builder().maximum_size(500).build();
        let help_label = gtk::Label::builder()
            .label(tr("Rules cannot be edited in the app yet. Rules included in imported profiles are checked automatically while Auto-Switch is enabled.\nRules can be combined with AND/OR logic for complex matching."))
            .wrap(true)
            .margin_top(16)
            .build();
        help_label.add_css_class("dim-label");
        help_box.set_child(Some(&help_label));
        rules_box.append(&help_box);

        scrolled.set_child(Some(&rules_box));
        content.append(&scrolled);

        dialog.set_content(Some(&content));

        dialog_window::present(&dialog, Some(window));
    }

    /// Get the Application and update config.
    fn update_app_config<F: FnOnce(&mut crate::models::AppConfig)>(&self, f: F) {
        // Rows being filled from the config must not write it back
        if self.imp().loading.get() {
            return;
        }
        // Start from the live config so only the caller's fields change
        if let Some(network_app) = self.app() {
            let mut config = network_app.config();
            f(&mut config);
            network_app.update_config(config);
            info!("Settings saved to config");
        }
    }

    /// The application this page's window belongs to.
    fn app(&self) -> Option<crate::application::Application> {
        app_for(self)
    }

    /// Save scheduling enabled setting.
    fn save_scheduling_enabled(&self, enabled: bool) {
        self.update_app_config(|config| {
            config.scheduling_enabled = enabled;
        });
    }

    /// Save auto-switch allow scripts setting.
    fn save_auto_switch_allow_scripts(&self, enabled: bool) {
        self.update_app_config(|config| {
            config.auto_switch_allow_scripts = enabled;
        });
    }

    /// Save hotkeys enabled setting.
    fn save_hotkeys_enabled(&self, enabled: bool) {
        self.update_app_config(|config| {
            config.hotkeys_enabled = enabled;
        });
    }

    /// Show or hide the encryption actions based on whether a passphrase is
    /// currently unlocked.
    fn update_encryption_rows(&self) {
        let imp = self.imp();
        let config = self.app().map(|app| app.config()).unwrap_or_default();

        if let Some(row) = imp.encryption_row.borrow().as_ref() {
            let was_loading = imp.loading.replace(true);
            row.set_active(config.encrypt_profiles);
            imp.loading.set(was_loading);
        }
        if let Some(row) = imp.encryption_change_row.borrow().as_ref() {
            row.set_visible(config.encrypt_profiles && config.encryption_key.is_some());
        }
        if let Some(row) = imp.encryption_unlock_row.borrow().as_ref() {
            row.set_visible(config.encrypt_profiles && config.encryption_key.is_none());
        }
    }

    /// The fallback profile chooser only matters for the "Switch Profile"
    /// watchdog action.
    fn update_watchdog_fallback_sensitivity(&self) {
        let imp = self.imp();
        let is_switch = self
            .app()
            .map(|app| app.config().watchdog.failure_action == WatchdogAction::SwitchProfile)
            .unwrap_or(false);
        if let Some(row) = imp.watchdog_fallback_row.borrow().as_ref() {
            row.set_sensitive(is_switch);
        }
    }

    /// Rebuild the fallback profile list and restore the saved selection.
    fn rebuild_fallback_model(&self, selected_id: Option<&str>) {
        let imp = self.imp();
        let Some(row) = imp.watchdog_fallback_row.borrow().clone() else {
            return;
        };

        let profiles = imp.profiles.borrow();
        let mut labels: Vec<String> = Vec::with_capacity(profiles.len() + 1);
        // TRANSLATORS: No fallback profile selected
        labels.push(tr("None"));
        labels.extend(profiles.iter().map(|profile| profile.name().to_string()));
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&label_refs);

        let selected = selected_id
            .and_then(|id| {
                profiles
                    .iter()
                    .position(|profile| profile.id().to_string() == id)
            })
            .map(|index| index as u32 + 1)
            .unwrap_or(0);

        let was_loading = imp.loading.replace(true);
        row.set_model(Some(&model));
        row.set_selected(selected);
        imp.loading.set(was_loading);
    }

    /// Update the profile list used by the watchdog fallback chooser.
    pub fn set_profiles(&self, profiles: Vec<Profile>) {
        *self.imp().profiles.borrow_mut() = profiles;
        let selected = self
            .app()
            .and_then(|app| app.config().watchdog.fallback_profile_id);
        self.rebuild_fallback_model(selected.as_deref());
    }

    /// Load config values into UI widgets.
    pub fn load_from_config(&self, config: &crate::models::AppConfig) {
        let imp = self.imp();
        imp.loading.set(true);

        // Theme
        if let Some(row) = imp.theme_row.borrow().as_ref() {
            let idx = match config.theme {
                ThemePreference::System => 0,
                ThemePreference::Light => 1,
                ThemePreference::Dark => 2,
            };
            row.set_selected(idx);
        }

        // Auto-switch
        if let Some(row) = imp.auto_switch_row.borrow().as_ref() {
            row.set_active(config.auto_switch_enabled);
        }
        if let Some(row) = imp.auto_switch_interval.borrow().as_ref() {
            row.set_value(config.auto_switch_interval_secs as f64);
        }
        if let Some(row) = imp.auto_switch_allow_scripts_row.borrow().as_ref() {
            row.set_active(config.auto_switch_allow_scripts);
        }

        // Notifications
        if let Some(row) = imp.notifications_row.borrow().as_ref() {
            row.set_active(config.show_notifications);
        }

        // Start minimized
        if let Some(row) = imp.start_minimized_row.borrow().as_ref() {
            row.set_active(config.start_minimized);
        }

        // Scheduling
        if let Some(row) = imp.scheduling_enabled_row.borrow().as_ref() {
            row.set_active(config.scheduling_enabled);
        }

        // Watchdog
        if let Some(row) = imp.watchdog_enabled_row.borrow().as_ref() {
            row.set_active(config.watchdog.enabled);
        }
        if let Some(row) = imp.watchdog_interval_row.borrow().as_ref() {
            row.set_value(config.watchdog.check_interval_secs as f64);
        }
        if let Some(row) = imp.watchdog_target_row.borrow().as_ref() {
            row.set_text(&config.watchdog.ping_target);
        }
        if let Some(row) = imp.watchdog_threshold_row.borrow().as_ref() {
            row.set_value(config.watchdog.failure_threshold as f64);
        }
        if let Some(row) = imp.watchdog_action_row.borrow().as_ref() {
            let idx = match config.watchdog.failure_action {
                WatchdogAction::Notify => 0,
                WatchdogAction::Reconnect => 1,
                WatchdogAction::SwitchProfile => 2,
                WatchdogAction::RestartNetworkManager => 3,
            };
            row.set_selected(idx);
        }

        // Fill the fallback profile chooser from the app's profile cache.
        if let Some(app) = self.app() {
            if let Some(store) = app.data_store() {
                *imp.profiles.borrow_mut() = store.profiles();
            }
        }
        self.rebuild_fallback_model(config.watchdog.fallback_profile_id.as_deref());
        if let Some(row) = imp.watchdog_fallback_row.borrow().as_ref() {
            row.set_sensitive(config.watchdog.failure_action == WatchdogAction::SwitchProfile);
        }

        // Security
        if let Some(row) = imp.sandbox_row.borrow().as_ref() {
            let idx = match config.sandbox_mode {
                SandboxMode::None => 0,
                SandboxMode::Bubblewrap => 1,
                SandboxMode::Firejail => 2,
            };
            row.set_selected(idx);
        }
        if let Some(row) = imp.encryption_row.borrow().as_ref() {
            row.set_active(config.encrypt_profiles);
        }
        if let Some(row) = imp.encryption_change_row.borrow().as_ref() {
            row.set_visible(config.encrypt_profiles && config.encryption_key.is_some());
        }
        if let Some(row) = imp.encryption_unlock_row.borrow().as_ref() {
            row.set_visible(config.encrypt_profiles && config.encryption_key.is_none());
        }

        // Hotkeys
        if let Some(row) = imp.hotkeys_enabled_row.borrow().as_ref() {
            row.set_active(config.hotkeys_enabled);
        }

        imp.loading.set(false);
        info!("Loaded settings from config");
    }
}

impl Default for SettingsPage {
    fn default() -> Self {
        Self::new()
    }
}
