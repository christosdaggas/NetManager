//! Adwaita application root object and lifecycle wiring.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::models::AppConfig;
use crate::storage::DataStore;
use crate::ui::MainWindow;
use crate::{tr, APP_ID, APP_NAME, VERSION};

/// Global Tokio runtime for async operations.
static TOKIO_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// How often the watchdog and auto-switch timers re-read the configuration.
const SERVICE_TICK_SECS: u32 = 1;

/// Shortest auto-switch interval, whatever the configuration says.
const AUTO_SWITCH_MIN_INTERVAL_SECS: u32 = 10;

/// Shortest watchdog interval (the settings page enforces the same floor).
const WATCHDOG_MIN_INTERVAL_SECS: u32 = 5;

/// How often the scheduler checks for due entries.
const SCHEDULER_TICK_SECS: u32 = 15;

/// Event posted from the watchdog thread to the main thread.
enum WatchdogEvent {
    /// Tell the user that the connection was lost.
    Notify,
    /// Switch to the configured fallback profile.
    SwitchProfile(String),
}

/// Get the global Tokio runtime handle.
pub fn tokio_runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

mod imp {
    use super::*;
    use crate::tray::TrayHandle;

    #[derive(Default)]
    pub struct Application {
        pub data_store: RefCell<Option<Arc<DataStore>>>,
        pub config: RefCell<AppConfig>,
        pub tray_handle: RefCell<Option<TrayHandle>>,
        /// Set by `--minimized`; consumed by the first activation.
        pub start_minimized: Cell<bool>,
        /// Keeps the app running while it was started minimized and the
        /// window has not been shown yet.
        pub minimized_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "CdNetworkManagerApplication";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_actions();
            obj.set_accels_for_action("app.quit", &["<primary>q"]);
            obj.set_accels_for_action("win.show-help-overlay", &["<primary>question"]);
        }
    }

    impl ApplicationImpl for Application {
        fn activate(&self) {
            let application = self.obj();

            let window = if let Some(window) = application.active_window() {
                window
            } else {
                let window = MainWindow::new(&application);

                if let Some(store) = self.data_store.borrow().clone() {
                    window.init_with_store(store);
                }

                // The profile cache is loaded by now, so the tray menu can
                // show real profile names from the start.
                if application.config().show_tray_icon {
                    application.start_tray();
                }

                window.upcast()
            };

            if self.start_minimized.replace(false) {
                // First activation with --minimized: create the window but keep
                // it hidden, and hold the app so background services keep
                // running. Without a tray there would be no visible way back,
                // so show the window instead.
                if self.tray_handle.borrow().is_some() {
                    info!("Started minimized; the window opens on the next activation");
                    *self.minimized_hold.borrow_mut() = Some(application.hold());
                    return;
                }
                info!("Started minimized but no tray is available; showing the window");
            }

            window.present();
            // Once the window has been shown, closing it behaves normally.
            self.minimized_hold.borrow_mut().take();
        }

        fn startup(&self) {
            self.parent_startup();
            let obj = self.obj();

            info!("{} {} starting up", APP_NAME, VERSION);

            if let Some(display) = gtk::gdk::Display::default() {
                let icon_theme = gtk::IconTheme::for_display(&display);

                if let Ok(exe_path) = std::env::current_exe() {
                    if let Some(exe_dir) = exe_path.parent() {
                        let dev_icons = exe_dir.join("../../data/icons");
                        if dev_icons.exists() {
                            if let Some(path_str) = dev_icons
                                .canonicalize()
                                .ok()
                                .and_then(|p| p.to_str().map(String::from))
                            {
                                icon_theme.add_search_path(&path_str);
                            }
                        }
                    }
                }

                icon_theme.add_search_path("data/icons");
            }

            gtk::Window::set_default_icon_name(APP_ID);

            let data_store = Arc::new(DataStore::new());
            *self.data_store.borrow_mut() = Some(data_store);

            let config = DataStore::load_config().unwrap_or_default();
            *self.config.borrow_mut() = config.clone();

            obj.apply_theme(config.theme);

            obj.load_css();

            obj.start_background_services();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

/// Release notes for the About dialog.
///
/// The dialog already shows the version as the heading, so these are only the
/// list items. Each item is translated on its own, so the markup stays out of
/// the translatable strings; the translations are escaped before they are
/// placed into the markup.
fn release_notes() -> String {
    let items = [
        tr("First release, made for Fedora with the GNOME desktop"),
        tr("Save network and system settings as profiles and switch between them in one click"),
        tr("Network settings: IP addresses, DNS, routes, Wi-Fi and VPN connections"),
        tr("System settings: hostname, proxy, firewall zone, default printer and timezone"),
        tr("Automatic switching based on the Wi-Fi network, the router or the time of day"),
        tr("Scheduled profile activation and a connection watchdog"),
        tr("Scripts before and after a profile is applied, optionally sandboxed"),
        tr("Optional profile encryption with a passphrase"),
        tr("System tray icon for quick profile switching"),
        tr("Dashboard with live network status and activity"),
        tr("Translated into 10 languages"),
    ];

    let mut notes = String::from("<ul>");
    for item in &items {
        notes.push_str("<li>");
        notes.push_str(&glib::markup_escape_text(item));
        notes.push_str("</li>");
    }
    notes.push_str("</ul>");
    notes
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .property("resource-base-path", "/com/chrisdaggas/network-manager")
            .build()
    }

    fn setup_actions(&self) {
        let action_quit = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| {
                app.quit();
            })
            .build();

        let action_about = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| {
                app.show_about();
            })
            .build();

        let action_preferences = gio::ActionEntry::builder("preferences")
            .activate(move |app: &Self, _, _| {
                app.show_preferences();
            })
            .build();

        self.add_action_entries([action_quit, action_about, action_preferences]);
    }

    fn load_css(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            warn!("No default display available; skipping CSS provider installation");
            return;
        };

        let provider = gtk::CssProvider::new();
        let css = include_str!("../data/style.css");
        provider.load_from_string(css);

        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    /// Apply the theme preference.
    pub fn apply_theme(&self, theme: crate::models::ThemePreference) {
        let style_manager = adw::StyleManager::default();
        match theme {
            crate::models::ThemePreference::System => {
                style_manager.set_color_scheme(adw::ColorScheme::Default);
            }
            crate::models::ThemePreference::Light => {
                style_manager.set_color_scheme(adw::ColorScheme::ForceLight);
            }
            crate::models::ThemePreference::Dark => {
                style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
            }
        }
    }

    fn show_about(&self) {
        let window = self.active_window();

        let about = adw::AboutDialog::builder()
            .application_name(tr(APP_NAME))
            .application_icon(APP_ID)
            .developer_name("Christos A. Daggas")
            .version(VERSION)
            .copyright("© 2026 Christos A. Daggas")
            .license_type(gtk::License::MitX11)
            .website("https://chrisdaggas.com")
            .issue_url("https://github.com/christosdaggas/Network-Manager/issues")
            .developers(vec!["Christos A. Daggas"])
            .comments(tr("Network and system profile manager for Linux"))
            .release_notes(release_notes())
            .build();

        about.present(window.as_ref());
    }

    fn show_preferences(&self) {
        let window = self.active_window();

        // A separate window (not an adw::PreferencesDialog) so it can be
        // moved independently of the main window.
        let dialog = crate::ui::dialog_window::new(&tr("Preferences"), 600, 640);

        let appearance_page = adw::PreferencesPage::new();
        appearance_page.set_title(&tr("Appearance"));
        appearance_page.set_icon_name(Some("preferences-desktop-appearance-symbolic"));

        let theme_group = adw::PreferencesGroup::new();
        theme_group.set_title(&tr("Theme"));

        let theme_labels = [
            // TRANSLATORS: Color scheme option that follows the system setting
            tr("Follow System"),
            // TRANSLATORS: Color scheme option
            tr("Light"),
            // TRANSLATORS: Color scheme option
            tr("Dark"),
        ];
        let theme_label_refs: Vec<&str> = theme_labels.iter().map(String::as_str).collect();
        let theme_model = gtk::StringList::new(&theme_label_refs);
        let theme_row = adw::ComboRow::builder()
            .title(tr("Color Scheme"))
            .subtitle(tr("Choose the application's appearance"))
            .model(&theme_model)
            .build();

        let current_theme = self.config().theme;
        match current_theme {
            crate::models::ThemePreference::System => theme_row.set_selected(0),
            crate::models::ThemePreference::Light => theme_row.set_selected(1),
            crate::models::ThemePreference::Dark => theme_row.set_selected(2),
        }

        let app_weak = self.downgrade();
        theme_row.connect_selected_notify(move |row| {
            if let Some(app) = app_weak.upgrade() {
                let theme = match row.selected() {
                    0 => crate::models::ThemePreference::System,
                    1 => crate::models::ThemePreference::Light,
                    2 => crate::models::ThemePreference::Dark,
                    _ => crate::models::ThemePreference::System,
                };
                app.apply_theme(theme);
                let mut config = app.config();
                config.theme = theme;
                app.update_config(config);
            }
        });

        theme_group.add(&theme_row);
        appearance_page.add(&theme_group);

        // === Behavior Page ===
        let behavior_page = adw::PreferencesPage::new();
        behavior_page.set_title(&tr("Behavior"));
        behavior_page.set_icon_name(Some("preferences-system-symbolic"));

        let startup_group = adw::PreferencesGroup::new();
        startup_group.set_title(&tr("Startup"));

        let config = self.config();

        // Check actual autostart state
        let autostart_enabled = crate::autostart::is_autostart_enabled();

        let autostart_row = adw::SwitchRow::builder()
            .title(tr("Start on Login"))
            .subtitle(tr("Automatically start Network Manager when you log in"))
            .active(autostart_enabled)
            .build();

        let app_weak = self.downgrade();
        autostart_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            if let Err(e) = crate::autostart::set_autostart(enabled) {
                tracing::error!("Failed to set autostart: {}", e);
            } else {
                if let Some(app) = app_weak.upgrade() {
                    let mut config = app.config();
                    config.autostart_on_login = enabled;
                    app.update_config(config);
                }
            }
        });

        startup_group.add(&autostart_row);

        let tray_row = adw::SwitchRow::builder()
            .title(tr("Show System Tray Icon"))
            .subtitle(tr("Display an icon in the system tray (AppIndicator on GNOME)"))
            .active(config.show_tray_icon)
            .build();

        let app_weak = self.downgrade();
        tray_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            if let Some(app) = app_weak.upgrade() {
                let mut config = app.config();
                if config.show_tray_icon == enabled {
                    return;
                }
                config.show_tray_icon = enabled;
                app.update_config(config);
                
                if enabled {
                    if !app.start_tray() {
                        if let Some(window) = app.active_window() {
                            if let Some(main_window) = window.downcast_ref::<MainWindow>() {
                                main_window.show_toast(&tr(
                                    "Tray icon unavailable. On GNOME, install the AppIndicator extension.",
                                ));
                            }
                        }
                    }
                } else {
                    app.stop_tray();
                }
            }
        });

        startup_group.add(&tray_row);

        behavior_page.add(&startup_group);

        let profile_group = adw::PreferencesGroup::new();
        profile_group.set_title(&tr("Profiles"));

        let confirm_switch_row = adw::SwitchRow::builder()
            .title(tr("Confirm Profile Switch"))
            .subtitle(tr("Ask for confirmation before switching profiles"))
            .active(true)
            .build();
        profile_group.add(&confirm_switch_row);

        behavior_page.add(&profile_group);

        // Pages, switched from the header bar
        let stack = adw::ViewStack::new();
        for page in [&appearance_page, &behavior_page] {
            stack.add_titled_with_icon(
                page,
                None,
                &page.title(),
                &page.icon_name().unwrap_or_default(),
            );
        }
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        dialog.set_content(Some(&toolbar));

        crate::ui::dialog_window::present(&dialog, window.as_ref());
    }

    /// Start with the window hidden (`--minimized`). Must be set before `run`.
    pub fn set_start_minimized(&self, minimized: bool) {
        self.imp().start_minimized.set(minimized);
    }

    /// Get the data store.
    pub fn data_store(&self) -> Option<Arc<DataStore>> {
        self.imp().data_store.borrow().clone()
    }

    /// Get the current configuration.
    pub fn config(&self) -> AppConfig {
        self.imp().config.borrow().clone()
    }

    /// Update and save configuration.
    pub fn update_config(&self, config: AppConfig) {
        *self.imp().config.borrow_mut() = config.clone();
        if let Some(store) = self.data_store() {
            // Keep the store's runtime settings (including the encryption
            // passphrase) in sync, then write the file once.
            store.update_settings(config);
        }
    }

    /// Start the system tray.
    ///
    /// The tray registers a StatusNotifierItem with the desktop. Stock GNOME
    /// has no watcher without the AppIndicator extension, so failure is
    /// reported instead of leaving an invisible process behind.
    fn start_tray(&self) -> bool {
        use crate::tray::{start_tray, TrayCommand};

        if self.imp().tray_handle.borrow().is_some() {
            return true;
        }

        if let Some((handle, receiver)) = start_tray(tokio_runtime()) {
            // Store the handle
            *self.imp().tray_handle.borrow_mut() = Some(handle);
            self.refresh_tray();

            // Process tray commands on the main thread
            let app_weak = self.downgrade();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                let Some(app) = app_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                while let Ok(cmd) = receiver.try_recv() {
                    match cmd {
                        TrayCommand::ShowWindow => {
                            app.activate();
                        }
                        TrayCommand::Quit => {
                            app.quit();
                        }
                        TrayCommand::ActivateProfile(profile_id) => {
                            if let Some(window) = app.main_window() {
                                window.apply_profile(&profile_id);
                            }
                        }
                    }
                }

                glib::ControlFlow::Continue
            });

            info!("System tray started");
            true
        } else {
            false
        }
    }

    /// Stop the tray service, if it is running.
    fn stop_tray(&self) {
        if let Some(handle) = self.imp().tray_handle.borrow_mut().take() {
            let awaiter = handle.shutdown();
            tokio_runtime().block_on(awaiter);
            info!("System tray stopped");
        }
    }

    /// Refresh the tray from the current profile cache.
    pub fn refresh_tray(&self) {
        let Some(store) = self.data_store() else {
            return;
        };
        let profiles = store.profiles();
        let tray_profiles = profiles
            .iter()
            .map(|profile| (profile.id().to_string(), profile.name().to_string()))
            .collect();
        let active = profiles
            .iter()
            .find(|profile| profile.status == crate::models::profile::ProfileStatus::Active)
            .map(|profile| profile.id().to_string());
        self.update_tray_profiles(tray_profiles, active);
    }

    /// Update the tray with profile information.
    pub fn update_tray_profiles(&self, profiles: Vec<(String, String)>, active: Option<String>) {
        if let Some(handle) = self.imp().tray_handle.borrow().as_ref() {
            handle.set_profiles(profiles);
            handle.set_active_profile(active);
        }
    }

    /// The main window, if one exists (it may be hidden when started minimized).
    fn main_window(&self) -> Option<MainWindow> {
        self.active_window().and_downcast::<MainWindow>()
    }

    /// Start background services (scheduler, watchdog, auto-switch).
    ///
    /// The timers are always installed and read the current configuration on
    /// every tick, so changes made in the settings take effect without a restart.
    fn start_background_services(&self) {
        self.start_scheduler();
        self.start_watchdog();
        self.start_autoswitch();
    }

    /// Start the profile scheduler.
    ///
    /// Each tick evaluates every minute since the previous tick, so a delayed
    /// tick cannot skip a scheduled minute.
    fn start_scheduler(&self) {
        use crate::scheduler::SchedulerService;

        info!("Starting profile scheduler");

        let app_weak = self.downgrade();
        let mut last_evaluated = None;
        glib::timeout_add_seconds_local(SCHEDULER_TICK_SECS, move || {
            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let (minutes, evaluated) =
                SchedulerService::minutes_to_evaluate(last_evaluated, chrono::Utc::now());
            last_evaluated = Some(evaluated);

            let config = app.config();
            if !config.scheduling_enabled || config.schedules.is_empty() || minutes.is_empty() {
                return glib::ControlFlow::Continue;
            }

            // Cron fields are matched against local time.
            let local_minutes: Vec<_> = minutes
                .iter()
                .map(|m| m.with_timezone(&chrono::Local))
                .collect();
            let due = SchedulerService::due_schedules(&config.schedules, &local_minutes);
            if due.is_empty() {
                return glib::ControlFlow::Continue;
            }

            let Some(main_window) = app.main_window() else {
                warn!(
                    "Scheduler: no main window, skipping {} due schedule(s)",
                    due.len()
                );
                return glib::ControlFlow::Continue;
            };

            let mut fired_one_shots = Vec::new();
            for entry in due {
                info!("Scheduler: activating profile {}", entry.profile_id);
                main_window.apply_profile(&entry.profile_id);
                if entry.one_shot {
                    fired_one_shots.push(entry.id.clone());
                }
            }

            // One-shot entries run once, then stay disabled.
            if !fired_one_shots.is_empty() {
                let mut config = app.config();
                if SchedulerService::disable_one_shots(&mut config.schedules, &fired_one_shots) {
                    app.update_config(config);
                }
            }

            glib::ControlFlow::Continue
        });
    }

    /// Start the connection watchdog.
    ///
    /// The blocking `ping` call runs on a background thread so the GTK
    /// main loop is never stalled.
    fn start_watchdog(&self) {
        use crate::models::{WatchdogAction, WatchdogConfig};
        use crate::services::WatchdogService;

        info!("Starting connection watchdog");

        let app_weak = self.downgrade();

        // Channel to receive watchdog events from the background thread.
        let (tx, rx) = std::sync::mpsc::channel::<WatchdogEvent>();

        // Handle watchdog events on the main thread.
        let app_weak_poll = app_weak.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            while let Ok(event) = rx.try_recv() {
                let Some(main_win) = app_weak_poll.upgrade().and_then(|app| app.main_window())
                else {
                    continue;
                };
                match event {
                    WatchdogEvent::Notify => {
                        main_win.show_toast(&tr("Connection lost! Check your network."));
                    }
                    WatchdogEvent::SwitchProfile(profile_id) => {
                        info!("Watchdog: switching to fallback profile {}", profile_id);
                        main_win.apply_profile(&profile_id);
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        // The service is recreated whenever the watchdog settings change.
        let mut current: Option<(WatchdogConfig, Arc<WatchdogService>)> = None;
        let mut last_check = Instant::now();

        glib::timeout_add_seconds_local(SERVICE_TICK_SECS, move || {
            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let config = app.config().watchdog;
            if !config.enabled {
                current = None;
                return glib::ControlFlow::Continue;
            }

            let watchdog = match &current {
                Some((active_config, watchdog)) if *active_config == config => Arc::clone(watchdog),
                _ => {
                    info!(
                        "Watchdog configured (interval: {}s, target: {})",
                        config.check_interval_secs, config.ping_target
                    );
                    let watchdog = Arc::new(WatchdogService::new(config.clone()));
                    current = Some((config.clone(), Arc::clone(&watchdog)));
                    last_check = Instant::now();
                    watchdog
                }
            };

            let interval = Duration::from_secs(u64::from(
                config.check_interval_secs.max(WATCHDOG_MIN_INTERVAL_SECS),
            ));
            if last_check.elapsed() < interval {
                return glib::ControlFlow::Continue;
            }
            last_check = Instant::now();

            let tx_inner = tx.clone();
            let fallback_profile_id = config.fallback_profile_id.clone();

            // Run the blocking connectivity check off the main thread.
            std::thread::spawn(move || {
                let Some(action) = watchdog.check() else {
                    return;
                };
                info!("Watchdog triggered action: {:?}", action);

                if let Err(e) = watchdog.execute_action(action) {
                    tracing::error!("Watchdog action failed: {}", e);
                }

                match action {
                    WatchdogAction::Notify => {
                        let _ = tx_inner.send(WatchdogEvent::Notify);
                    }
                    WatchdogAction::SwitchProfile => match fallback_profile_id {
                        Some(profile_id) => {
                            let _ = tx_inner.send(WatchdogEvent::SwitchProfile(profile_id));
                        }
                        None => {
                            warn!("Watchdog: no fallback profile configured, cannot switch");
                        }
                    },
                    WatchdogAction::Reconnect | WatchdogAction::RestartNetworkManager => {}
                }
            });

            glib::ControlFlow::Continue
        });
    }

    /// Start the auto-switch service.
    ///
    /// Blocking network checks (`nmcli`, `ip`, `ping`) run on a
    /// background thread so the GTK main loop stays responsive.
    fn start_autoswitch(&self) {
        use crate::services::AutoSwitchService;
        use std::sync::{Mutex, TryLockError};

        info!("Starting auto-switch service");

        // Keep the service behind Arc<Mutex<>> so it can be shared with the
        // background thread while preserving its mutable regex cache.
        let service = Arc::new(Mutex::new(AutoSwitchService::new()));

        let app_weak = self.downgrade();

        // Channel to post matched profile IDs from background to main thread.
        let (tx, rx) = std::sync::mpsc::channel::<String>();

        // Poll for auto-switch results on the main thread.
        let app_weak_poll = app_weak.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            while let Ok(profile_id) = rx.try_recv() {
                if let Some(main_window) = app_weak_poll.upgrade().and_then(|app| app.main_window())
                {
                    // apply_profile_auto shows its own result toast.
                    main_window.apply_profile_auto(&profile_id);
                }
            }
            glib::ControlFlow::Continue
        });

        // Re-read the configuration every tick; evaluate once per interval.
        let mut last_run = Instant::now();
        glib::timeout_add_seconds_local(SERVICE_TICK_SECS, move || {
            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let config = app.config();
            if !config.auto_switch_enabled {
                return glib::ControlFlow::Continue;
            }
            let interval = Duration::from_secs(u64::from(
                config
                    .auto_switch_interval_secs
                    .max(AUTO_SWITCH_MIN_INTERVAL_SECS),
            ));
            if last_run.elapsed() < interval {
                return glib::ControlFlow::Continue;
            }
            last_run = Instant::now();

            // Snapshot the profiles on the main thread (cheap clone).
            let profiles = if let Some(store) = app.imp().data_store.borrow().as_ref() {
                store.profiles()
            } else {
                Vec::new()
            };

            let service_clone = Arc::clone(&service);
            let tx_inner = tx.clone();

            // Offload the blocking evaluation to a background thread.
            std::thread::spawn(move || {
                let matched_id = {
                    let mut svc = match service_clone.try_lock() {
                        Ok(svc) => svc,
                        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                        Err(TryLockError::WouldBlock) => {
                            debug!("Auto-switch: previous evaluation still running, skipping");
                            return;
                        }
                    };
                    svc.evaluate_profiles(&profiles)
                };

                if let Some(profile_id) = matched_id {
                    info!("Auto-switch: Activating profile {}", profile_id);
                    let _ = tx_inner.send(profile_id);
                }
            });

            glib::ControlFlow::Continue
        });
    }
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}
