// Network Manager - Main Window
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Main Application Window.
//!
//! Navigation split view with sidebar and content area.
//! Follows GNOME HIG for adaptive layouts.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use crate::application::Application;
use crate::models::Profile;
use crate::storage::DataStore;
use crate::ui::dialog_window;
use crate::ui::pages::{
    DashboardPage, HelpPage, LogsPage, ProfilesPage, ScannerPage, SettingsPage,
};
use crate::{ntr, tr};

/// Navigation items for the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Dashboard,
    Profiles,
    Scanner,
    Logs,
    Settings,
    Help,
}

impl NavItem {
    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Dashboard => "view-grid-symbolic",
            Self::Profiles => "contact-new-symbolic",
            Self::Scanner => "system-search-symbolic",
            Self::Logs => "document-properties-symbolic",
            Self::Settings => "preferences-system-symbolic",
            Self::Help => "help-about-symbolic",
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Dashboard => tr("Dashboard"),
            Self::Profiles => tr("Profiles"),
            Self::Scanner => tr("Scanner"),
            Self::Logs => tr("Logs"),
            Self::Settings => tr("Settings"),
            Self::Help => tr("Help"),
        }
    }

    pub fn all() -> &'static [NavItem] {
        &[
            Self::Dashboard,
            Self::Profiles,
            Self::Scanner,
            Self::Logs,
            Self::Settings,
            Self::Help,
        ]
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MainWindow {
        pub sidebar_box: RefCell<Option<gtk::Box>>,
        pub sidebar_list: RefCell<Option<gtk::ListBox>>,
        pub content_stack: RefCell<Option<gtk::Stack>>,
        pub header_bar: RefCell<Option<adw::HeaderBar>>,
        pub content_title: RefCell<Option<adw::WindowTitle>>,
        pub current_nav: Cell<Option<NavItem>>,
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,

        pub sidebar_collapsed: Cell<bool>,
        pub sidebar_toggle_btn: RefCell<Option<gtk::Button>>,
        pub sidebar_header: RefCell<Option<gtk::Box>>,
        pub sidebar_title: RefCell<Option<adw::WindowTitle>>,
        pub nav_labels: RefCell<Vec<gtk::Label>>,
        pub nav_boxes: RefCell<Vec<gtk::Box>>,
        pub update_banner: RefCell<Option<gtk::Box>>,

        /// True while a profile's actions are running in the background.
        pub apply_in_progress: Cell<bool>,

        // Data store for persistence
        pub data_store: RefCell<Option<Arc<DataStore>>>,

        // Profiles storage
        pub profiles: Rc<RefCell<Vec<Profile>>>,

        // Views
        pub dashboard_page: RefCell<Option<DashboardPage>>,
        pub profiles_page: RefCell<Option<ProfilesPage>>,
        pub scanner_page: RefCell<Option<ScannerPage>>,
        pub logs_page: RefCell<Option<LogsPage>>,
        pub settings_page: RefCell<Option<SettingsPage>>,
        pub help_page: RefCell<Option<HelpPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MainWindow {
        const NAME: &'static str = "CdNetworkManagerMainWindow";
        type Type = super::MainWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for MainWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
            obj.setup_actions();
        }
    }

    impl WidgetImpl for MainWindow {}
    impl WindowImpl for MainWindow {}
    impl ApplicationWindowImpl for MainWindow {}
    impl AdwApplicationWindowImpl for MainWindow {}
}

glib::wrapper! {
    pub struct MainWindow(ObjectSubclass<imp::MainWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap;
}

/// Check if a profile contains any automation actions (scripts or programs).
fn profile_has_automation(profile: &crate::models::Profile) -> bool {
    !profile.automation_actions.is_empty()
}

/// Window size used when no valid size has been saved.
const DEFAULT_WINDOW_WIDTH: i32 = 1060;
const DEFAULT_WINDOW_HEIGHT: i32 = 680;

/// Per-adapter widgets shared by the create and edit profile dialogs.
type AdapterWidgets = (
    adw::SwitchRow,        // enabled
    adw::ComboRow,         // ipv4 method
    adw::EntryRow,         // static ip
    adw::EntryRow,         // subnet
    adw::EntryRow,         // gateway
    adw::ComboRow,         // dns method
    adw::EntryRow,         // dns1
    adw::EntryRow,         // dns2
    Option<adw::EntryRow>, // wifi ssid (only for wifi adapters)
);

/// Values read from one adapter form: interface, enabled, IPv4 method, static IP,
/// subnet, gateway, DNS method, primary DNS, secondary DNS, Wi-Fi SSID.
type AdapterConfig = (
    String,
    bool,
    u32,
    String,
    String,
    String,
    u32,
    String,
    String,
    Option<String>,
);

/// Outcome of a single profile action that did not fail.
enum ActionOutcome {
    Applied,
    /// Nothing was changed (unsupported action type or nothing to act on).
    Skipped(String),
}

type ActionResult = std::result::Result<ActionOutcome, String>;

/// Summary of a profile apply, built off the GTK thread.
#[derive(Default)]
struct ApplyReport {
    applied: usize,
    skipped: Vec<String>,
    errors: Vec<String>,
    /// A pre-script failed, so the remaining actions were not run.
    aborted: bool,
}

impl ApplyReport {
    /// Count one action's result; returns false if the action failed.
    fn record(&mut self, result: ActionResult) -> bool {
        match result {
            Ok(ActionOutcome::Applied) => self.applied += 1,
            Ok(ActionOutcome::Skipped(reason)) => self.skipped.push(reason),
            Err(e) => {
                self.errors.push(e);
                return false;
            }
        }
        true
    }
}

/// `nmcli` with a fixed locale so its output can be parsed.
fn nmcli() -> std::process::Command {
    let mut cmd = std::process::Command::new("nmcli");
    cmd.env("LC_ALL", "C");
    cmd
}

/// Run a command, turning a spawn failure or non-zero exit into an error.
fn run_checked(
    cmd: &mut std::process::Command,
    what: &str,
) -> std::result::Result<std::process::Output, String> {
    let output = cmd
        .output()
        .map_err(|e| format!("{}: failed to run: {}", what, e))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{}: {}",
            what,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Enable or disable an adapter's detail rows to match its switch and methods.
fn sync_adapter_sensitivity(widgets: &AdapterWidgets) {
    let (enabled_row, ip_method, static_ip, subnet, gateway, dns_method, dns1, dns2, wifi_ssid) =
        widgets;
    let enabled = enabled_row.is_active();
    let is_static = enabled && ip_method.selected() == 1;
    let is_manual_dns = enabled && dns_method.selected() == 1;
    ip_method.set_sensitive(enabled);
    dns_method.set_sensitive(enabled);
    static_ip.set_sensitive(is_static);
    subnet.set_sensitive(is_static);
    gateway.set_sensitive(is_static);
    dns1.set_sensitive(is_manual_dns);
    dns2.set_sensitive(is_manual_dns);
    if let Some(ssid) = wifi_ssid {
        ssid.set_sensitive(enabled);
    }
}

/// Build a drop-down model from already translated items.
fn string_list(items: &[String]) -> gtk::StringList {
    let items: Vec<&str> = items.iter().map(String::as_str).collect();
    gtk::StringList::new(&items)
}

impl MainWindow {
    pub fn new(app: &Application) -> Self {
        let config = app.config();

        // Restore the saved size, falling back to the default when it is unusable
        let size_ok = |w: i32, h: i32| (360..=16384).contains(&w) && (294..=16384).contains(&h);
        let (width, height) = if size_ok(config.window_width, config.window_height) {
            (config.window_width, config.window_height)
        } else {
            (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        };

        let window: Self = glib::Object::builder()
            .property("application", app)
            .property("default-width", width)
            .property("default-height", height)
            .build();

        // Apply saved maximized state
        if config.window_maximized {
            window.maximize();
        }

        window.set_title(Some(&tr(crate::APP_NAME)));

        // The application is only set once construction has finished, so
        // accelerators cannot be registered from constructed().
        window.setup_accels(app);

        // Show the saved settings before any of the settings handlers can save.
        if let Some(settings_page) = window.imp().settings_page.borrow().as_ref() {
            settings_page.load_from_config(&config);
        }

        // Keep the two-surface shell in sync with the live color scheme so
        // the sidebar and content surfaces follow the theme immediately.
        window.sync_shell_theme();
        let window_weak = window.downgrade();
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(window) = window_weak.upgrade() {
                window.sync_shell_theme();
            }
        });
        let window_weak = window.downgrade();
        adw::StyleManager::default().connect_high_contrast_notify(move |_| {
            if let Some(window) = window_weak.upgrade() {
                window.sync_shell_theme();
            }
        });

        // Connect close-request to save window state
        let app_weak = app.downgrade();
        window.connect_close_request(move |win| {
            if let Some(app) = app_weak.upgrade() {
                let mut config = app.config();

                // Only save size if not maximized
                config.window_maximized = win.is_maximized();
                if !config.window_maximized {
                    let (width, height) = win.default_size();
                    config.window_width = width;
                    config.window_height = height;
                }

                app.update_config(config);
            }
            glib::Propagation::Proceed
        });

        window
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Two-surface shell: navigation rail on the left and an uninterrupted
        // content surface on the right, with no divider between them.
        let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main_box.add_css_class("background");
        main_box.add_css_class("network-manager-window-layout");

        // Create sidebar content with fixed width
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.set_width_request(260);
        sidebar_box.set_hexpand(false);
        sidebar_box.add_css_class("sidebar-box");
        sidebar_box.add_css_class("network-manager-sidebar");

        // A plain box rather than an AdwHeaderBar: a header bar reserves a
        // minimum width and would otherwise floor the collapsed rail well
        // above the intended 80px.
        let sidebar_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sidebar_header.set_height_request(47);
        sidebar_header.set_hexpand(false);
        sidebar_header.add_css_class("toolbar");
        sidebar_header.add_css_class("network-manager-sidebar-header");

        let sidebar_title = adw::WindowTitle::new(&tr(crate::APP_NAME), "");
        sidebar_title.set_hexpand(true);
        sidebar_title.set_halign(gtk::Align::Start);
        sidebar_title.set_margin_start(12);
        sidebar_header.append(&sidebar_title);

        // Sidebar collapse button (trailing edge of the sidebar header)
        let sidebar_toggle_btn = gtk::Button::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(tr("Collapse sidebar"))
            .build();
        sidebar_toggle_btn.add_css_class("flat");
        sidebar_toggle_btn.set_valign(gtk::Align::Center);
        sidebar_toggle_btn.set_action_name(Some("win.toggle-sidebar"));
        sidebar_header.append(&sidebar_toggle_btn);

        // The plain box does not move the window by itself; the handle restores
        // dragging, double-click to maximize and the window menu on this strip.
        let sidebar_handle = gtk::WindowHandle::new();
        sidebar_handle.set_child(Some(&sidebar_header));
        sidebar_box.append(&sidebar_handle);

        // Navigation list
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
        sidebar_list.add_css_class("navigation-sidebar");

        // Add navigation items and collect labels for collapse/expand
        let mut nav_labels = Vec::new();
        let mut nav_boxes = Vec::new();
        for nav_item in NavItem::all() {
            let (row, label, hbox) = self.create_nav_row_with_label(*nav_item);
            sidebar_list.append(&row);
            nav_labels.push(label);
            nav_boxes.push(hbox);
        }

        // Handle navigation selection
        let window_weak = self.downgrade();
        sidebar_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                if let Some(window) = window_weak.upgrade() {
                    let index = row.index() as usize;
                    if let Some(nav_item) = NavItem::all().get(index) {
                        window.navigate_to(*nav_item);
                    }
                }
            }
        });

        let sidebar_scroll = gtk::ScrolledWindow::new();
        sidebar_scroll.set_vexpand(true);
        sidebar_scroll.set_child(Some(&sidebar_list));
        sidebar_box.append(&sidebar_scroll);

        // Add sidebar to main box
        main_box.append(&sidebar_box);

        // Create content area
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_box.set_hexpand(true);
        content_box.add_css_class("network-manager-content");
        content_box.add_css_class("network-manager-view");
        content_box.add_css_class("view");

        // Content header
        let header_bar = adw::HeaderBar::new();
        header_bar.add_css_class("toolbar");
        header_bar.add_css_class("network-manager-content-header");
        let content_title = adw::WindowTitle::new(&tr("Dashboard"), "");
        content_title.set_visible(false);
        header_bar.set_title_widget(Some(&content_title));

        // Add menu button
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some(&tr("Main Menu")));

        // Create custom popover with theme selector
        let popover = self.create_main_menu_popover();
        menu_button.set_popover(Some(&popover));

        header_bar.pack_end(&menu_button);

        content_box.append(&header_bar);

        // Content stack for views
        let content_stack = gtk::Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        content_stack.set_transition_duration(200);
        content_stack.set_vexpand(true);
        content_stack.set_hexpand(true);
        content_stack.add_css_class("view");
        content_stack.add_css_class("network-manager-view");

        // Create pages
        let dashboard_page = DashboardPage::new();
        let profiles_page = ProfilesPage::new();
        let scanner_page = ScannerPage::new();
        let logs_page = LogsPage::new();
        let settings_page = SettingsPage::new();
        let help_page = HelpPage::new();

        content_stack.add_named(&dashboard_page, Some("dashboard"));
        content_stack.add_named(&profiles_page, Some("profiles"));
        content_stack.add_named(&scanner_page, Some("scanner"));
        content_stack.add_named(&logs_page, Some("logs"));
        content_stack.add_named(&settings_page, Some("settings"));
        content_stack.add_named(&help_page, Some("help"));

        content_box.append(&content_stack);
        content_box.append(&self.create_status_bar());

        // Add content to main box
        main_box.append(&content_box);

        // Wrap main_box in ToastOverlay for notifications
        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&main_box));

        // Set the window content
        self.set_content(Some(&toast_overlay));

        // Store references
        *imp.sidebar_box.borrow_mut() = Some(sidebar_box);
        *imp.sidebar_list.borrow_mut() = Some(sidebar_list);
        *imp.content_stack.borrow_mut() = Some(content_stack);
        *imp.header_bar.borrow_mut() = Some(header_bar);
        *imp.content_title.borrow_mut() = Some(content_title);
        *imp.toast_overlay.borrow_mut() = Some(toast_overlay);
        *imp.sidebar_header.borrow_mut() = Some(sidebar_header);
        *imp.sidebar_title.borrow_mut() = Some(sidebar_title);
        *imp.nav_labels.borrow_mut() = nav_labels;
        *imp.nav_boxes.borrow_mut() = nav_boxes;
        imp.sidebar_collapsed.set(false);
        *imp.sidebar_toggle_btn.borrow_mut() = Some(sidebar_toggle_btn);
        *imp.dashboard_page.borrow_mut() = Some(dashboard_page);
        *imp.profiles_page.borrow_mut() = Some(profiles_page);
        *imp.scanner_page.borrow_mut() = Some(scanner_page);
        *imp.logs_page.borrow_mut() = Some(logs_page);
        *imp.settings_page.borrow_mut() = Some(settings_page);
        *imp.help_page.borrow_mut() = Some(help_page);

        // Select first item
        if let Some(list) = imp.sidebar_list.borrow().as_ref() {
            if let Some(first_row) = list.row_at_index(0) {
                list.select_row(Some(&first_row));
            }
        }
    }

    fn setup_actions(&self) {
        // New Profile action - shows create profile dialog
        let new_profile_action = gio::SimpleAction::new("new-profile", None);
        let window_weak = self.downgrade();
        new_profile_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.show_create_profile_dialog();
            }
        });
        self.add_action(&new_profile_action);

        // Edit Profile action
        let edit_profile_action =
            gio::SimpleAction::new("edit-profile", Some(&String::static_variant_type()));
        let window_weak = self.downgrade();
        edit_profile_action.connect_activate(move |_, param| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(profile_id) = param.and_then(|p| p.get::<String>()) {
                    window.show_edit_profile_dialog(&profile_id);
                }
            }
        });
        self.add_action(&edit_profile_action);

        // Delete Profile action
        let delete_profile_action =
            gio::SimpleAction::new("delete-profile", Some(&String::static_variant_type()));
        let window_weak = self.downgrade();
        delete_profile_action.connect_activate(move |_, param| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(profile_id) = param.and_then(|p| p.get::<String>()) {
                    window.show_delete_profile_dialog(&profile_id);
                }
            }
        });
        self.add_action(&delete_profile_action);

        // Duplicate Profile action
        let duplicate_profile_action =
            gio::SimpleAction::new("duplicate-profile", Some(&String::static_variant_type()));
        let window_weak = self.downgrade();
        duplicate_profile_action.connect_activate(move |_, param| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(profile_id) = param.and_then(|p| p.get::<String>()) {
                    window.duplicate_profile(&profile_id);
                }
            }
        });
        self.add_action(&duplicate_profile_action);

        // Export Profiles action
        let export_profiles_action = gio::SimpleAction::new("export-profiles", None);
        let window_weak = self.downgrade();
        export_profiles_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.show_export_profiles_dialog();
            }
        });
        self.add_action(&export_profiles_action);

        // Import Profiles action
        let import_profiles_action = gio::SimpleAction::new("import-profiles", None);
        let window_weak = self.downgrade();
        import_profiles_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.show_import_profiles_dialog();
            }
        });
        self.add_action(&import_profiles_action);

        // Apply Profile action - actually applies network configuration
        let apply_profile_action =
            gio::SimpleAction::new("apply-profile", Some(&String::static_variant_type()));
        let window_weak = self.downgrade();
        apply_profile_action.connect_activate(move |_, param| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(profile_id) = param.and_then(|p| p.get::<String>()) {
                    window.apply_profile(&profile_id);
                }
            }
        });
        self.add_action(&apply_profile_action);

        // Apply Profile (auto-switch) action - skips automation actions unless explicitly allowed
        let apply_profile_auto_action =
            gio::SimpleAction::new("apply-profile-auto", Some(&String::static_variant_type()));
        let window_weak = self.downgrade();
        apply_profile_auto_action.connect_activate(move |_, param| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(profile_id) = param.and_then(|p| p.get::<String>()) {
                    window.apply_profile_auto(&profile_id);
                }
            }
        });
        self.add_action(&apply_profile_auto_action);

        // Show Profile Switcher action - opens a dialog to select and apply a profile
        let show_switcher_action = gio::SimpleAction::new("show-profile-switcher", None);
        let window_weak = self.downgrade();
        show_switcher_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.show_profile_switcher_dialog();
            }
        });
        self.add_action(&show_switcher_action);

        // Toggle sidebar action
        let toggle_sidebar_action = gio::SimpleAction::new("toggle-sidebar", None);
        let window_weak = self.downgrade();
        toggle_sidebar_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.toggle_sidebar();
            }
        });
        self.add_action(&toggle_sidebar_action);

        // Copy network info to clipboard action
        let copy_network_info_action = gio::SimpleAction::new("copy-network-info", None);
        let window_weak = self.downgrade();
        copy_network_info_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.copy_network_info_to_clipboard();
            }
        });
        self.add_action(&copy_network_info_action);

        // Show network diagnostics action
        let show_diagnostics_action = gio::SimpleAction::new("show-diagnostics", None);
        let window_weak = self.downgrade();
        show_diagnostics_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.show_network_diagnostics_dialog();
            }
        });
        self.add_action(&show_diagnostics_action);

        // Navigation actions for keyboard shortcuts
        let navigate_dashboard = gio::SimpleAction::new("navigate-dashboard", None);
        let window_weak = self.downgrade();
        navigate_dashboard.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Dashboard);
            }
        });
        self.add_action(&navigate_dashboard);

        let navigate_profiles = gio::SimpleAction::new("navigate-profiles", None);
        let window_weak = self.downgrade();
        navigate_profiles.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Profiles);
            }
        });
        self.add_action(&navigate_profiles);

        let navigate_scanner = gio::SimpleAction::new("navigate-scanner", None);
        let window_weak = self.downgrade();
        navigate_scanner.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Scanner);
            }
        });
        self.add_action(&navigate_scanner);

        let navigate_logs = gio::SimpleAction::new("navigate-logs", None);
        let window_weak = self.downgrade();
        navigate_logs.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Logs);
            }
        });
        self.add_action(&navigate_logs);

        let navigate_settings = gio::SimpleAction::new("navigate-settings", None);
        let window_weak = self.downgrade();
        navigate_settings.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Settings);
            }
        });
        self.add_action(&navigate_settings);

        let navigate_help = gio::SimpleAction::new("navigate-help", None);
        let window_weak = self.downgrade();
        navigate_help.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.navigate_to(NavItem::Help);
            }
        });
        self.add_action(&navigate_help);

        // Refresh action
        let refresh_action = gio::SimpleAction::new("refresh", None);
        let window_weak = self.downgrade();
        refresh_action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.refresh_current_view();
            }
        });
        self.add_action(&refresh_action);
    }

    /// Register the window's keyboard shortcuts.
    fn setup_accels(&self, app: &Application) {
        app.set_accels_for_action("win.new-profile", &["<Primary>n"]);
        app.set_accels_for_action("win.show-profile-switcher", &["<Primary>p"]);
        app.set_accels_for_action("win.toggle-sidebar", &["<Primary>b"]);
        app.set_accels_for_action("win.copy-network-info", &["<Primary><Shift>c"]);
        app.set_accels_for_action("win.show-diagnostics", &["<Primary>d"]);
        app.set_accels_for_action("win.refresh", &["<Primary>r", "F5"]);
        app.set_accels_for_action("win.navigate-dashboard", &["<Primary>1"]);
        app.set_accels_for_action("win.navigate-profiles", &["<Primary>2"]);
        app.set_accels_for_action("win.navigate-scanner", &["<Primary>3"]);
        app.set_accels_for_action("win.navigate-logs", &["<Primary>4"]);
        app.set_accels_for_action("win.navigate-settings", &["<Primary>5"]);
        app.set_accels_for_action("win.navigate-help", &["<Primary>6", "F1"]);
    }

    /// Keep the shell's light/dark CSS classes aligned with libadwaita.
    fn sync_shell_theme(&self) {
        let style_manager = adw::StyleManager::default();

        // High contrast keeps the stock Adwaita surfaces, so the sampled
        // light/dark shell colours are only applied outside of it.
        if style_manager.is_high_contrast() {
            self.add_css_class("network-manager-high-contrast");
            self.remove_css_class("network-manager-light");
            self.remove_css_class("network-manager-dark");
        } else if style_manager.is_dark() {
            self.remove_css_class("network-manager-high-contrast");
            self.add_css_class("network-manager-dark");
            self.remove_css_class("network-manager-light");
        } else {
            self.remove_css_class("network-manager-high-contrast");
            self.add_css_class("network-manager-light");
            self.remove_css_class("network-manager-dark");
        }
    }

    /// Bottom status bar: update indicator and version on the trailing edge.
    fn create_status_bar(&self) -> gtk::Box {
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        status_bar.add_css_class("app-status-bar");

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        status_bar.append(&spacer);

        // Hidden until the version check completes.
        let update_banner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        update_banner.add_css_class("update-banner");
        update_banner.set_visible(false);
        update_banner.set_valign(gtk::Align::Center);

        let update_icon = gtk::Image::from_icon_name("software-update-available-symbolic");
        update_icon.set_pixel_size(12);
        update_banner.append(&update_icon);

        let update_label = gtk::Label::new(Some(&tr("New version available")));
        update_label.add_css_class("update-banner-label");
        update_banner.append(&update_label);

        status_bar.append(&update_banner);
        self.imp().update_banner.replace(Some(update_banner));

        let version_label = gtk::Label::new(Some(
            &tr("Version {version}").replace("{version}", env!("CARGO_PKG_VERSION")),
        ));
        version_label.add_css_class("app-version-label");
        version_label.set_halign(gtk::Align::End);
        status_bar.append(&version_label);

        status_bar
    }

    /// Toggle the sidebar between collapsed (icons only) and expanded.
    fn toggle_sidebar(&self) {
        let imp = self.imp();

        let is_collapsed = imp.sidebar_collapsed.get();
        let new_collapsed = !is_collapsed;
        imp.sidebar_collapsed.set(new_collapsed);

        // Update sidebar width and collapsed CSS class
        if let Some(sidebar_box) = imp.sidebar_box.borrow().as_ref() {
            if new_collapsed {
                sidebar_box.set_width_request(80); // Collapsed: icons only
                sidebar_box.add_css_class("sidebar-collapsed");
            } else {
                sidebar_box.set_width_request(260); // Expanded: full width
                sidebar_box.remove_css_class("sidebar-collapsed");
            }
        }

        // Hide/show sidebar title
        if let Some(sidebar_title) = imp.sidebar_title.borrow().as_ref() {
            sidebar_title.set_visible(!new_collapsed);
        }

        // Hide/show navigation labels and adjust nav box alignment
        for label in imp.nav_labels.borrow().iter() {
            label.set_visible(!new_collapsed);
        }
        for hbox in imp.nav_boxes.borrow().iter() {
            if new_collapsed {
                // Remove side margins and center the icon in the rail
                hbox.set_margin_start(0);
                hbox.set_margin_end(0);
                hbox.set_halign(gtk::Align::Center);
            } else {
                // Restore margins and alignment when expanded
                hbox.set_margin_start(12);
                hbox.set_margin_end(12);
                hbox.set_halign(gtk::Align::Fill);
            }
        }

        // Update toggle button tooltip and icon
        if let Some(btn) = imp.sidebar_toggle_btn.borrow().as_ref() {
            if new_collapsed {
                btn.set_tooltip_text(Some(&tr("Expand sidebar")));
                btn.set_icon_name("sidebar-show-right-symbolic");
            } else {
                btn.set_tooltip_text(Some(&tr("Collapse sidebar")));
                btn.set_icon_name("sidebar-show-symbolic");
            }
        }
    }

    fn show_create_profile_dialog(&self) {
        use crate::models::AdapterType;
        use crate::network_utils::detect_network_adapters;
        use std::collections::HashMap;

        let dialog = dialog_window::new(&tr("Create Profile"), 700, 850);

        let toolbar_view = adw::ToolbarView::new();

        // Header bar
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);

        let cancel_btn = gtk::Button::with_label(&tr("Cancel"));
        let create_btn = gtk::Button::with_label(&tr("Create"));
        create_btn.add_css_class("suggested-action");
        create_btn.set_sensitive(false);

        header.pack_start(&cancel_btn);
        header.pack_end(&create_btn);
        toolbar_view.add_top_bar(&header);

        // Shows why the profile could not be created
        let error_banner = adw::Banner::new("");
        error_banner.set_use_markup(false);
        toolbar_view.add_top_bar(&error_banner);

        // Scrollable content
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);

        // === Template Selection ===
        let template_group = adw::PreferencesGroup::new();
        template_group.set_title(&tr("Start From Template"));
        template_group
            .set_description(Some(&tr("Optionally start with a pre-configured template")));

        let template_model = string_list(&[
            tr("Blank (Empty)"),
            tr("Home Network"),
            tr("Office Network"),
            tr("Public WiFi"),
            tr("VPN Only"),
            // TRANSLATORS: profile template for software development work
            tr("Development"),
        ]);
        let template_row = adw::ComboRow::builder()
            .title(tr("Template"))
            .subtitle(tr("Pre-configured settings for common scenarios"))
            .model(&template_model)
            .selected(0) // Default to Blank
            .build();
        template_group.add(&template_row);

        content.append(&template_group);

        // === Profile Details ===
        let details_group = adw::PreferencesGroup::new();
        details_group.set_title(&tr("Profile Details"));

        let name_entry = adw::EntryRow::new();
        // TRANSLATORS: profile name field
        name_entry.set_title(&tr("Name"));
        details_group.add(&name_entry);

        let desc_entry = adw::EntryRow::new();
        // TRANSLATORS: profile description field
        desc_entry.set_title(&tr("Description"));
        details_group.add(&desc_entry);

        let group_entry = adw::EntryRow::new();
        group_entry.set_title(&tr("Group (optional)"));
        details_group.add(&group_entry);

        content.append(&details_group);

        // === Detect Available Network Adapters ===
        let adapters = detect_network_adapters();

        // Store adapter configuration widgets
        let adapter_widgets: Rc<RefCell<HashMap<String, AdapterWidgets>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Create adapter configuration section
        let adapters_group = adw::PreferencesGroup::new();
        adapters_group.set_title(&tr("Network Adapters"));
        adapters_group.set_description(Some(&tr("Configure settings for each network adapter. Each adapter can have its own IP, DNS, and state settings.")));

        if adapters.is_empty() {
            let no_adapters_label = gtk::Label::new(Some(&tr("No network adapters detected")));
            no_adapters_label.add_css_class("dim-label");
            no_adapters_label.set_margin_top(12);
            no_adapters_label.set_margin_bottom(12);
            adapters_group.add(&no_adapters_label);
        } else {
            for adapter in &adapters {
                // Create an expander row for each adapter
                let expander = adw::ExpanderRow::new();
                let icon_name = adapter.adapter_type.icon_name();

                // Add icon as a prefix widget (since set_icon_name is deprecated in 1.3+)
                let icon = gtk::Image::from_icon_name(icon_name);
                expander.add_prefix(&icon);

                expander.set_title(&adapter.name);

                let subtitle = if let Some(mac) = &adapter.mac_address {
                    format!("{} • {}", adapter.adapter_type.display_name(), mac)
                } else {
                    adapter.adapter_type.display_name().to_string()
                };
                expander.set_subtitle(&subtitle);

                // Enable/disable switch for this adapter
                let enabled_row = adw::SwitchRow::builder()
                    .title(tr("Enable Adapter"))
                    .subtitle(tr("Configure this adapter in the profile"))
                    .active(true)
                    .build();
                expander.add_row(&enabled_row);

                // Create a container for adapter-specific settings
                let settings_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
                settings_box.set_margin_start(16);

                // IPv4 Configuration
                let ip_method_model = string_list(&[
                    tr("DHCP (Automatic)"),
                    tr("Static (Manual)"),
                    // TRANSLATORS: IPv4 method: the adapter gets no IPv4 address
                    tr("Disabled"),
                ]);
                let ip_method_row = adw::ComboRow::builder()
                    .title(tr("IPv4 Configuration"))
                    .subtitle(tr("How to obtain IP address"))
                    .model(&ip_method_model)
                    .selected(0)
                    .build();
                expander.add_row(&ip_method_row);

                // Static IP fields
                let static_ip = adw::EntryRow::new();
                static_ip.set_title(&tr("IP Address"));
                static_ip.set_text("192.168.1.100");
                static_ip.set_sensitive(false);
                expander.add_row(&static_ip);

                let subnet = adw::EntryRow::new();
                subnet.set_title(&tr("Subnet Mask"));
                subnet.set_text("255.255.255.0");
                subnet.set_sensitive(false);
                expander.add_row(&subnet);

                let gateway = adw::EntryRow::new();
                gateway.set_title(&tr("Gateway"));
                gateway.set_text("192.168.1.1");
                gateway.set_sensitive(false);
                expander.add_row(&gateway);

                // DNS Configuration
                let dns_method_model = string_list(&[
                    tr("Automatic (from DHCP)"),
                    // TRANSLATORS: DNS method or proxy mode: entered by hand
                    tr("Manual"),
                ]);
                let dns_method_row = adw::ComboRow::builder()
                    .title(tr("DNS Configuration"))
                    .model(&dns_method_model)
                    .selected(0)
                    .build();
                expander.add_row(&dns_method_row);

                let dns1 = adw::EntryRow::new();
                dns1.set_title(&tr("Primary DNS"));
                dns1.set_text("8.8.8.8");
                dns1.set_sensitive(false);
                expander.add_row(&dns1);

                let dns2 = adw::EntryRow::new();
                dns2.set_title(&tr("Secondary DNS"));
                dns2.set_text("8.8.4.4");
                dns2.set_sensitive(false);
                expander.add_row(&dns2);

                // WiFi-specific: SSID selection
                let wifi_ssid_entry = if adapter.adapter_type == AdapterType::Wifi {
                    let ssid_entry = adw::EntryRow::new();
                    ssid_entry.set_title(&tr("WiFi Network (SSID)"));
                    ssid_entry.set_sensitive(true);
                    expander.add_row(&ssid_entry);
                    Some(ssid_entry)
                } else {
                    None
                };

                // Connect IP method change to enable/disable static fields
                let static_ip_weak = static_ip.downgrade();
                let subnet_weak = subnet.downgrade();
                let gateway_weak = gateway.downgrade();
                ip_method_row.connect_selected_notify(move |row| {
                    let is_static = row.selected() == 1;
                    if let Some(e) = static_ip_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                    if let Some(e) = subnet_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                    if let Some(e) = gateway_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                });

                // Connect DNS method change to enable/disable DNS fields
                let dns1_weak = dns1.downgrade();
                let dns2_weak = dns2.downgrade();
                dns_method_row.connect_selected_notify(move |row| {
                    let is_manual = row.selected() == 1;
                    if let Some(e) = dns1_weak.upgrade() {
                        e.set_sensitive(is_manual);
                    }
                    if let Some(e) = dns2_weak.upgrade() {
                        e.set_sensitive(is_manual);
                    }
                });

                // Connect enable switch to toggle all settings; re-enabling restores
                // the fields that the selected methods allow
                let widgets_weak = Rc::downgrade(&adapter_widgets);
                let iface = adapter.name.clone();
                enabled_row.connect_active_notify(move |_| {
                    if let Some(widgets) = widgets_weak.upgrade() {
                        if let Some(w) = widgets.borrow().get(&iface) {
                            sync_adapter_sensitivity(w);
                        }
                    }
                });

                // Store widget references for this adapter
                adapter_widgets.borrow_mut().insert(
                    adapter.name.clone(),
                    (
                        enabled_row,
                        ip_method_row,
                        static_ip,
                        subnet,
                        gateway,
                        dns_method_row,
                        dns1,
                        dns2,
                        wifi_ssid_entry,
                    ),
                );

                adapters_group.add(&expander);
            }
        }

        content.append(&adapters_group);

        // === VPN Connection ===
        let vpn_group = adw::PreferencesGroup::new();
        vpn_group.set_title(&tr("VPN Connection"));
        vpn_group.set_description(Some(&tr(
            "Optionally connect to a VPN when this profile is active",
        )));

        let vpn_enabled = adw::SwitchRow::builder()
            .title(tr("Connect to VPN"))
            .subtitle(tr("Use a VPN connection from NetworkManager"))
            .active(false)
            .build();
        vpn_group.add(&vpn_enabled);

        let vpn_name = adw::EntryRow::new();
        vpn_name.set_title(&tr("VPN Connection Name"));
        vpn_name.set_sensitive(false);
        vpn_group.add(&vpn_name);

        let vpn_name_weak = vpn_name.downgrade();
        vpn_enabled.connect_active_notify(move |row| {
            if let Some(e) = vpn_name_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        content.append(&vpn_group);

        // === Proxy Settings ===
        let proxy_group = adw::PreferencesGroup::new();
        proxy_group.set_title(&tr("Proxy Settings"));
        proxy_group.set_description(Some(&tr("Configure system proxy via gsettings")));

        let proxy_mode_model = string_list(&[
            // TRANSLATORS: proxy mode: no proxy is used
            tr("None"),
            // TRANSLATORS: DNS method or proxy mode: entered by hand
            tr("Manual"),
            tr("Automatic (PAC)"),
        ]);
        let proxy_mode_row = adw::ComboRow::builder()
            .title(tr("Proxy Mode"))
            .model(&proxy_mode_model)
            .selected(0)
            .build();
        proxy_group.add(&proxy_mode_row);

        let http_proxy = adw::EntryRow::new();
        http_proxy.set_title(&tr("HTTP Proxy"));
        http_proxy.set_text("http://proxy:8080");
        http_proxy.set_sensitive(false);
        proxy_group.add(&http_proxy);

        let https_proxy = adw::EntryRow::new();
        https_proxy.set_title(&tr("HTTPS Proxy"));
        https_proxy.set_text("http://proxy:8080");
        https_proxy.set_sensitive(false);
        proxy_group.add(&https_proxy);

        let no_proxy = adw::EntryRow::new();
        no_proxy.set_title(&tr("No Proxy For"));
        no_proxy.set_text("localhost,127.0.0.1");
        no_proxy.set_sensitive(false);
        proxy_group.add(&no_proxy);

        let pac_url = adw::EntryRow::new();
        pac_url.set_title(&tr("PAC URL"));
        pac_url.set_sensitive(false);
        proxy_group.add(&pac_url);

        let http_proxy_weak = http_proxy.downgrade();
        let https_proxy_weak = https_proxy.downgrade();
        let no_proxy_weak = no_proxy.downgrade();
        let pac_url_weak = pac_url.downgrade();
        proxy_mode_row.connect_selected_notify(move |row| {
            let mode = row.selected();
            let is_manual = mode == 1;
            let is_auto = mode == 2;
            if let Some(e) = http_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = https_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = no_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = pac_url_weak.upgrade() {
                e.set_sensitive(is_auto);
            }
        });

        content.append(&proxy_group);

        // === Scripts/Programs ===
        let scripts_group = adw::PreferencesGroup::new();
        scripts_group.set_title(&tr("Scripts and Programs"));
        scripts_group.set_description(Some(&tr("Run scripts when profile activates")));

        let pre_script_enabled = adw::SwitchRow::builder()
            .title(tr("Run Pre-Script"))
            .subtitle(tr("Execute script before applying profile"))
            .active(false)
            .build();
        scripts_group.add(&pre_script_enabled);

        let pre_script_path = adw::EntryRow::new();
        pre_script_path.set_title(&tr("Pre-Script Path"));
        pre_script_path.set_text("/path/to/script.sh");
        pre_script_path.set_sensitive(false);
        scripts_group.add(&pre_script_path);

        let post_script_enabled = adw::SwitchRow::builder()
            .title(tr("Run Post-Script"))
            .subtitle(tr("Execute script after applying profile"))
            .active(false)
            .build();
        scripts_group.add(&post_script_enabled);

        let post_script_path = adw::EntryRow::new();
        post_script_path.set_title(&tr("Post-Script Path"));
        post_script_path.set_text("/path/to/script.sh");
        post_script_path.set_sensitive(false);
        scripts_group.add(&post_script_path);

        let run_program_enabled = adw::SwitchRow::builder()
            .title(tr("Run Program"))
            .subtitle(tr("Launch an application when profile activates"))
            .active(false)
            .build();
        scripts_group.add(&run_program_enabled);

        let run_program_path = adw::EntryRow::new();
        run_program_path.set_title(&tr("Program Path"));
        run_program_path.set_sensitive(false);
        scripts_group.add(&run_program_path);

        let run_program_args = adw::EntryRow::new();
        run_program_args.set_title(&tr("Program Arguments"));
        run_program_args.set_sensitive(false);
        scripts_group.add(&run_program_args);

        // Script toggle handlers
        let pre_script_path_weak = pre_script_path.downgrade();
        pre_script_enabled.connect_active_notify(move |row| {
            if let Some(e) = pre_script_path_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        let post_script_path_weak = post_script_path.downgrade();
        post_script_enabled.connect_active_notify(move |row| {
            if let Some(e) = post_script_path_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        let run_program_path_weak = run_program_path.downgrade();
        let run_program_args_weak = run_program_args.downgrade();
        run_program_enabled.connect_active_notify(move |row| {
            let enabled = row.is_active();
            if let Some(e) = run_program_path_weak.upgrade() {
                e.set_sensitive(enabled);
            }
            if let Some(e) = run_program_args_weak.upgrade() {
                e.set_sensitive(enabled);
            }
        });

        content.append(&scripts_group);

        // Enable/disable create button based on name
        let create_btn_weak = create_btn.downgrade();
        name_entry.connect_changed(move |entry| {
            if let Some(btn) = create_btn_weak.upgrade() {
                btn.set_sensitive(!entry.text().is_empty());
            }
        });

        scroll.set_child(Some(&content));
        toolbar_view.set_content(Some(&scroll));
        dialog.set_content(Some(&toolbar_view));

        // Cancel action
        let dialog_weak = dialog.downgrade();
        cancel_btn.connect_clicked(move |_| {
            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }
        });

        // Create weak references for fields needed in create handler
        let dialog_weak = dialog.downgrade();
        let window_weak = self.downgrade();
        let template_row_weak = template_row.downgrade();
        let name_entry_weak = name_entry.downgrade();
        let desc_entry_weak = desc_entry.downgrade();
        let group_entry_weak = group_entry.downgrade();
        // VPN
        let vpn_enabled_weak = vpn_enabled.downgrade();
        let vpn_name_weak = vpn_name.downgrade();
        // Proxy
        let proxy_mode_row_weak = proxy_mode_row.downgrade();
        let http_proxy_weak = http_proxy.downgrade();
        let https_proxy_weak = https_proxy.downgrade();
        let no_proxy_weak = no_proxy.downgrade();
        let pac_url_weak = pac_url.downgrade();
        // Scripts
        let pre_script_enabled_weak = pre_script_enabled.downgrade();
        let pre_script_path_weak = pre_script_path.downgrade();
        let post_script_enabled_weak = post_script_enabled.downgrade();
        let post_script_path_weak = post_script_path.downgrade();
        let run_program_enabled_weak = run_program_enabled.downgrade();
        let run_program_path_weak = run_program_path.downgrade();
        let run_program_args_weak = run_program_args.downgrade();
        // Adapter widgets
        let adapter_widgets_ref = adapter_widgets.clone();
        let error_banner_weak = error_banner.downgrade();

        create_btn.connect_clicked(move |_| {
            // Refuse to create the profile while an adapter form is invalid
            if let Some(message) = Self::validate_adapter_widgets(&adapter_widgets_ref.borrow()) {
                if let Some(banner) = error_banner_weak.upgrade() {
                    banner.set_title(&message);
                    banner.set_revealed(true);
                }
                return;
            }

            let template_idx = template_row_weak
                .upgrade()
                .map(|r| r.selected())
                .unwrap_or(0);
            let name = name_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let description = desc_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let group_name = group_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // VPN settings
            let vpn_on = vpn_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let vpn_name_val = vpn_name_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Proxy settings
            let proxy_mode = proxy_mode_row_weak
                .upgrade()
                .map(|r| r.selected())
                .unwrap_or(0);
            let http_proxy_val = http_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let https_proxy_val = https_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let no_proxy_val = no_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let pac_url_val = pac_url_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Script settings
            let pre_script_on = pre_script_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let pre_script = pre_script_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let post_script_on = post_script_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let post_script = post_script_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let run_prog_on = run_program_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let run_prog_path = run_program_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let run_prog_args = run_program_args_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Collect adapter configurations
            let adapter_configs: Vec<AdapterConfig> = adapter_widgets_ref
                .borrow()
                .iter()
                .map(|(iface, widgets)| {
                    let (
                        enabled,
                        ip_method,
                        static_ip,
                        subnet,
                        gateway,
                        dns_method,
                        dns1,
                        dns2,
                        wifi_ssid,
                    ) = widgets;
                    (
                        iface.clone(),
                        enabled.is_active(),
                        ip_method.selected(),
                        static_ip.text().to_string(),
                        subnet.text().to_string(),
                        gateway.text().to_string(),
                        dns_method.selected(),
                        dns1.text().to_string(),
                        dns2.text().to_string(),
                        wifi_ssid.as_ref().map(|e| e.text().to_string()),
                    )
                })
                .collect();

            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }

            if let Some(window) = window_weak.upgrade() {
                use crate::models::{
                    AutomationAction, NetworkAction, ProfileTemplate, ProxyConfig, ProxyMode,
                    SystemAction,
                };
                use std::path::PathBuf;

                // Create the profile - either from template or blank
                let mut profile = match template_idx {
                    0 => Profile::new(&name), // Blank
                    1 => ProfileTemplate::HomeNetwork.create_profile(&name),
                    2 => ProfileTemplate::OfficeNetwork.create_profile(&name),
                    3 => ProfileTemplate::PublicWifi.create_profile(&name),
                    4 => ProfileTemplate::VpnOnly.create_profile(&name),
                    5 => ProfileTemplate::Development.create_profile(&name),
                    _ => Profile::new(&name),
                };

                // Override description/group if user provided them
                if !description.is_empty() {
                    profile.metadata.description = Some(description);
                }

                if !group_name.is_empty() {
                    profile.metadata.group = Some(crate::models::ProfileGroup::new(&group_name));
                }

                // === Process each adapter configuration ===
                for config in &adapter_configs {
                    profile
                        .network_actions
                        .extend(Self::adapter_network_actions(config));
                }

                // === VPN Connection ===
                if vpn_on && !vpn_name_val.is_empty() {
                    profile.network_actions.push(NetworkAction::VpnConnect {
                        connection_name: vpn_name_val,
                    });
                }

                // === Proxy Configuration ===
                if proxy_mode > 0 {
                    let mode = match proxy_mode {
                        1 => ProxyMode::Manual,
                        2 => ProxyMode::Auto,
                        _ => ProxyMode::None,
                    };

                    let no_proxy_list: Vec<String> = no_proxy_val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();

                    profile
                        .system_actions
                        .push(SystemAction::ProxyConfig(ProxyConfig {
                            mode,
                            http_proxy: if proxy_mode == 1 && !http_proxy_val.is_empty() {
                                Some(http_proxy_val)
                            } else {
                                None
                            },
                            https_proxy: if proxy_mode == 1 && !https_proxy_val.is_empty() {
                                Some(https_proxy_val)
                            } else {
                                None
                            },
                            ftp_proxy: None,
                            socks_proxy: None,
                            no_proxy: no_proxy_list,
                            pac_url: if proxy_mode == 2 && !pac_url_val.is_empty() {
                                Some(pac_url_val)
                            } else {
                                None
                            },
                        }));
                }

                // === Pre-Script ===
                if pre_script_on && !pre_script.is_empty() {
                    profile
                        .automation_actions
                        .push(AutomationAction::PreScript {
                            path: PathBuf::from(pre_script),
                            args: vec![],
                            env: std::collections::HashMap::new(),
                            mode: crate::models::ScriptMode::Wait,
                            working_dir: None,
                            continue_on_error: false,
                        });
                }

                // === Post-Script ===
                if post_script_on && !post_script.is_empty() {
                    profile
                        .automation_actions
                        .push(AutomationAction::PostScript {
                            path: PathBuf::from(post_script),
                            args: vec![],
                            env: std::collections::HashMap::new(),
                            mode: crate::models::ScriptMode::Wait,
                            working_dir: None,
                            continue_on_error: false,
                        });
                }

                // === Run Program ===
                if run_prog_on && !run_prog_path.is_empty() {
                    let args: Vec<String> = run_prog_args
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect();

                    profile
                        .automation_actions
                        .push(AutomationAction::RunProgram {
                            program: run_prog_path,
                            args,
                            env: std::collections::HashMap::new(),
                            mode: crate::models::ProgramMode::Background,
                            working_dir: None,
                        });
                }

                // Store the profile
                let imp = window.imp();
                let profile_name_for_log = name.clone();
                imp.profiles.borrow_mut().push(profile);

                // Save to cache
                window.save_profiles_to_cache();

                // Log the profile creation
                if let Some(store) = imp.data_store.borrow().as_ref() {
                    store.append_log(
                        "INFO",
                        &format!("Profile '{}' created", profile_name_for_log),
                    );
                }

                // Refresh logs page
                if let Some(logs_page) = imp.logs_page.borrow().as_ref() {
                    logs_page.refresh_logs();
                }

                // Update the profiles page
                if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
                    let profiles = imp.profiles.borrow().clone();
                    profiles_page.update_profiles(profiles);
                }

                // Navigate to profiles page
                window.navigate_to(NavItem::Profiles);

                window.show_toast(&tr("Profile '{name}' created").replace("{name}", &name));
            }
        });

        dialog_window::present(&dialog, Some(self));
    }

    /// Helper to convert subnet mask string to prefix length
    fn subnet_to_prefix(subnet: &str) -> Option<u8> {
        match subnet.trim() {
            "255.255.255.255" => Some(32),
            "255.255.255.254" => Some(31),
            "255.255.255.252" => Some(30),
            "255.255.255.248" => Some(29),
            "255.255.255.240" => Some(28),
            "255.255.255.224" => Some(27),
            "255.255.255.192" => Some(26),
            "255.255.255.128" => Some(25),
            "255.255.255.0" => Some(24),
            "255.255.254.0" => Some(23),
            "255.255.252.0" => Some(22),
            "255.255.248.0" => Some(21),
            "255.255.240.0" => Some(20),
            "255.255.224.0" => Some(19),
            "255.255.192.0" => Some(18),
            "255.255.128.0" => Some(17),
            "255.255.0.0" => Some(16),
            "255.254.0.0" => Some(15),
            "255.252.0.0" => Some(14),
            "255.248.0.0" => Some(13),
            "255.240.0.0" => Some(12),
            "255.224.0.0" => Some(11),
            "255.192.0.0" => Some(10),
            "255.128.0.0" => Some(9),
            "255.0.0.0" => Some(8),
            _ => None,
        }
    }

    /// Accept a dotted subnet mask or a prefix length ("24" or "/24").
    fn parse_prefix(subnet: &str) -> Option<u8> {
        let subnet = subnet.trim();
        Self::subnet_to_prefix(subnet).or_else(|| {
            subnet
                .trim_start_matches('/')
                .parse::<u8>()
                .ok()
                .filter(|p| (1..=32).contains(p))
        })
    }

    /// Parse a static IPv4 form; an empty gateway means none.
    fn parse_static_ipv4(
        ip: &str,
        subnet: &str,
        gateway: &str,
    ) -> Option<(std::net::Ipv4Addr, u8, Option<std::net::Ipv4Addr>)> {
        let address = ip.trim().parse().ok()?;
        let prefix = Self::parse_prefix(subnet)?;
        let gateway = match gateway.trim() {
            "" => None,
            gw => Some(gw.parse().ok()?),
        };
        Some((address, prefix, gateway))
    }

    /// Parse the manual DNS fields; empty fields are ignored but one is required.
    fn parse_dns_servers(dns1: &str, dns2: &str) -> Option<Vec<std::net::IpAddr>> {
        let mut servers = Vec::new();
        for text in [dns1.trim(), dns2.trim()] {
            if !text.is_empty() {
                servers.push(text.parse().ok()?);
            }
        }
        (!servers.is_empty()).then_some(servers)
    }

    /// Check every enabled adapter form, highlight invalid rows and return
    /// a message for the first adapter that cannot be saved.
    fn validate_adapter_widgets(
        widgets: &std::collections::HashMap<String, AdapterWidgets>,
    ) -> Option<String> {
        fn mark(row: &adw::EntryRow, ok: bool) -> bool {
            if ok {
                row.remove_css_class("error");
            } else {
                row.add_css_class("error");
            }
            ok
        }

        let mut names: Vec<&String> = widgets.keys().collect();
        names.sort();
        let mut first_error = None;

        for name in names {
            let (enabled, ip_method, static_ip, subnet, gateway, dns_method, dns1, dns2, _) =
                &widgets[name];
            let is_static = enabled.is_active() && ip_method.selected() == 1;
            let manual_dns = enabled.is_active() && dns_method.selected() == 1;
            let mut problems: Vec<String> = Vec::new();

            if !mark(
                static_ip,
                !is_static
                    || static_ip
                        .text()
                        .trim()
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok(),
            ) {
                // TRANSLATORS: field name, listed in "{adapter}: invalid {fields}"
                problems.push(tr("IP address"));
            }
            if !mark(
                subnet,
                !is_static || Self::parse_prefix(&subnet.text()).is_some(),
            ) {
                // TRANSLATORS: field name, listed in "{adapter}: invalid {fields}"
                problems.push(tr("subnet mask or prefix"));
            }
            let gw = gateway.text();
            if !mark(
                gateway,
                !is_static
                    || gw.trim().is_empty()
                    || gw.trim().parse::<std::net::Ipv4Addr>().is_ok(),
            ) {
                // TRANSLATORS: field name, listed in "{adapter}: invalid {fields}"
                problems.push(tr("gateway"));
            }

            let (d1, d2) = (dns1.text(), dns2.text());
            let d1_ok = d1.trim().is_empty() || d1.trim().parse::<std::net::IpAddr>().is_ok();
            let d2_ok = d2.trim().is_empty() || d2.trim().parse::<std::net::IpAddr>().is_ok();
            let any_dns = !d1.trim().is_empty() || !d2.trim().is_empty();
            if !mark(dns1, !manual_dns || (d1_ok && any_dns)) {
                // TRANSLATORS: field name, listed in "{adapter}: invalid {fields}"
                problems.push(tr("primary DNS server"));
            }
            if !mark(dns2, !manual_dns || d2_ok) {
                // TRANSLATORS: field name, listed in "{adapter}: invalid {fields}"
                problems.push(tr("secondary DNS server"));
            }

            if first_error.is_none() && !problems.is_empty() {
                first_error = Some(
                    // TRANSLATORS: {adapter} is an interface name such as wlp2s0,
                    // {fields} a comma-separated list of the field names above
                    tr("{adapter}: invalid {fields}")
                        .replace("{fields}", &problems.join(", "))
                        .replace("{adapter}", name),
                );
            }
        }

        first_error
    }

    /// Build the network actions for one adapter form (validated beforehand).
    fn adapter_network_actions(config: &AdapterConfig) -> Vec<crate::models::NetworkAction> {
        use crate::models::{InterfaceState, Ipv4Address, Ipv4Method, NetworkAction};

        let (
            iface,
            enabled,
            ip_method,
            static_ip,
            subnet,
            gateway,
            dns_method,
            dns1,
            dns2,
            wifi_ssid,
        ) = config;
        let mut actions = vec![NetworkAction::InterfaceEnable(InterfaceState {
            interface: iface.clone(),
            enabled: *enabled,
        })];
        if !*enabled {
            return actions;
        }

        // IPv4 Configuration
        let ipv4 = match ip_method {
            0 => Some((Ipv4Method::Auto, vec![], None)),
            1 => Self::parse_static_ipv4(static_ip, subnet, gateway).map(
                |(address, prefix, gateway)| {
                    (
                        Ipv4Method::Manual,
                        vec![Ipv4Address { address, prefix }],
                        gateway,
                    )
                },
            ),
            2 => Some((Ipv4Method::Disabled, vec![], None)),
            _ => None,
        };
        if let Some((method, addresses, gateway)) = ipv4 {
            actions.push(NetworkAction::Ipv4Config {
                interface: Some(iface.clone()),
                method,
                addresses,
                gateway,
            });
        }

        // DNS: manual servers, or an empty list so automatic DNS clears old servers
        let servers = if *dns_method == 1 {
            Self::parse_dns_servers(dns1, dns2).unwrap_or_default()
        } else {
            Vec::new()
        };
        actions.push(NetworkAction::DnsServers {
            interface: Some(iface.clone()),
            servers,
        });

        // WiFi SSID (if applicable)
        if let Some(ssid) = wifi_ssid {
            if !ssid.is_empty() {
                actions.push(NetworkAction::WifiConnect {
                    ssid: ssid.clone(),
                    interface: Some(iface.clone()),
                });
            }
        }

        actions
    }

    /// Get the NetworkManager connection name for a given interface
    fn get_connection_for_interface(interface: &str) -> Option<String> {
        // Use nmcli to get the connection name for this device
        let output = nmcli()
            .args(["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"])
            .output()
            .ok()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 && parts[1] == interface {
                    return Some(parts[0].to_string());
                }
            }
        }

        // Fallback: try to find any connection that uses this interface
        let output = nmcli()
            .args(["-t", "-f", "NAME,DEVICE", "connection", "show"])
            .output()
            .ok()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 && parts[1] == interface {
                    return Some(parts[0].to_string());
                }
            }
        }

        None
    }

    /// Show edit profile dialog
    fn show_edit_profile_dialog(&self, profile_id: &str) {
        use crate::models::{
            AdapterType, AutomationAction, Ipv4Method, NetworkAction, ProxyMode, SystemAction,
        };
        use crate::network_utils::detect_network_adapters;
        use std::collections::HashMap;

        let imp = self.imp();

        // Find the profile
        let profile_opt = {
            let profiles = imp.profiles.borrow();
            profiles
                .iter()
                .find(|p| p.id().to_string() == profile_id)
                .cloned()
        };

        let Some(profile) = profile_opt else {
            self.show_toast(&tr("Profile not found"));
            return;
        };

        let dialog = dialog_window::new(&tr("Edit Profile"), 700, 850);

        let toolbar_view = adw::ToolbarView::new();

        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);

        let cancel_btn = gtk::Button::with_label(&tr("Cancel"));
        let save_btn = gtk::Button::with_label(&tr("Save"));
        save_btn.add_css_class("suggested-action");

        header.pack_start(&cancel_btn);
        header.pack_end(&save_btn);
        toolbar_view.add_top_bar(&header);

        // Shows why the changes could not be saved
        let error_banner = adw::Banner::new("");
        error_banner.set_use_markup(false);
        toolbar_view.add_top_bar(&error_banner);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);

        // === Profile Details ===
        let details_group = adw::PreferencesGroup::new();
        details_group.set_title(&tr("Profile Details"));

        let name_entry = adw::EntryRow::new();
        // TRANSLATORS: profile name field
        name_entry.set_title(&tr("Name"));
        name_entry.set_text(profile.name());
        details_group.add(&name_entry);

        let desc_entry = adw::EntryRow::new();
        // TRANSLATORS: profile description field
        desc_entry.set_title(&tr("Description"));
        if let Some(desc) = &profile.metadata.description {
            desc_entry.set_text(desc);
        }
        details_group.add(&desc_entry);

        let group_entry = adw::EntryRow::new();
        group_entry.set_title(&tr("Group (optional)"));
        if let Some(grp) = &profile.metadata.group {
            group_entry.set_text(&grp.name);
        }
        details_group.add(&group_entry);

        content.append(&details_group);

        // === Detect Available Network Adapters ===
        let adapters = detect_network_adapters();

        // Helper to find network action for an interface
        let find_ipv4_config = |iface: &str| -> Option<(Ipv4Method, String, String, String)> {
            for action in &profile.network_actions {
                if let NetworkAction::Ipv4Config {
                    interface,
                    method,
                    addresses,
                    gateway,
                } = action
                {
                    if interface.as_deref() == Some(iface) {
                        let ip = addresses
                            .first()
                            .map(|a| a.address.to_string())
                            .unwrap_or_default();
                        let prefix = addresses.first().map(|a| a.prefix).unwrap_or(24);
                        let subnet = Self::prefix_to_subnet(prefix);
                        let gw = gateway.map(|g| g.to_string()).unwrap_or_default();
                        return Some((method.clone(), ip, subnet, gw));
                    }
                }
            }
            None
        };

        let find_dns_servers = |iface: &str| -> Option<(String, String)> {
            for action in &profile.network_actions {
                // An empty server list means automatic DNS
                if let NetworkAction::DnsServers { interface, servers } = action {
                    if interface.as_deref() == Some(iface) && !servers.is_empty() {
                        let dns1 = servers.first().map(|s| s.to_string()).unwrap_or_default();
                        let dns2 = servers.get(1).map(|s| s.to_string()).unwrap_or_default();
                        return Some((dns1, dns2));
                    }
                }
            }
            None
        };

        let find_interface_enabled = |iface: &str| -> bool {
            for action in &profile.network_actions {
                if let NetworkAction::InterfaceEnable(state) = action {
                    if state.interface == iface {
                        return state.enabled;
                    }
                }
            }
            true
        };

        let find_wifi_ssid = |iface: &str| -> Option<String> {
            for action in &profile.network_actions {
                if let NetworkAction::WifiConnect { ssid, interface } = action {
                    if interface.as_deref() == Some(iface) {
                        return Some(ssid.clone());
                    }
                }
            }
            None
        };

        // Store adapter configuration widgets
        let adapter_widgets: Rc<RefCell<HashMap<String, AdapterWidgets>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Create adapter configuration section
        let adapters_group = adw::PreferencesGroup::new();
        adapters_group.set_title(&tr("Network Adapters"));
        adapters_group.set_description(Some(&tr("Configure settings for each network adapter")));

        if adapters.is_empty() {
            let no_adapters_label = gtk::Label::new(Some(&tr("No network adapters detected")));
            no_adapters_label.add_css_class("dim-label");
            no_adapters_label.set_margin_top(12);
            no_adapters_label.set_margin_bottom(12);
            adapters_group.add(&no_adapters_label);
        } else {
            for adapter in &adapters {
                let expander = adw::ExpanderRow::new();
                let icon_name = adapter.adapter_type.icon_name();
                let icon = gtk::Image::from_icon_name(icon_name);
                expander.add_prefix(&icon);
                expander.set_title(&adapter.name);

                let subtitle = if let Some(mac) = &adapter.mac_address {
                    format!("{} • {}", adapter.adapter_type.display_name(), mac)
                } else {
                    adapter.adapter_type.display_name().to_string()
                };
                expander.set_subtitle(&subtitle);

                // Load existing values
                let iface_enabled = find_interface_enabled(&adapter.name);
                let ipv4_config = find_ipv4_config(&adapter.name);
                let dns_config = find_dns_servers(&adapter.name);
                let wifi_ssid = find_wifi_ssid(&adapter.name);

                // Enable/disable switch for this adapter
                let enabled_row = adw::SwitchRow::builder()
                    .title(tr("Enable Adapter"))
                    .subtitle(tr("Configure this adapter in the profile"))
                    .active(iface_enabled)
                    .build();
                expander.add_row(&enabled_row);

                // IPv4 Configuration
                let ip_method_model = string_list(&[
                    tr("DHCP (Automatic)"),
                    tr("Static (Manual)"),
                    // TRANSLATORS: IPv4 method: the adapter gets no IPv4 address
                    tr("Disabled"),
                ]);
                let ip_method_row = adw::ComboRow::builder()
                    .title(tr("IPv4 Configuration"))
                    .subtitle(tr("How to obtain IP address"))
                    .model(&ip_method_model)
                    .selected(match &ipv4_config {
                        Some((Ipv4Method::Auto, _, _, _)) => 0,
                        Some((Ipv4Method::Manual, _, _, _)) => 1,
                        Some((Ipv4Method::Disabled, _, _, _)) => 2,
                        _ => 0,
                    })
                    .build();
                expander.add_row(&ip_method_row);

                // Static IP fields
                let static_ip = adw::EntryRow::new();
                static_ip.set_title(&tr("IP Address"));
                static_ip.set_text(
                    &ipv4_config
                        .as_ref()
                        .map(|(_, ip, _, _)| ip.clone())
                        .unwrap_or_else(|| "192.168.1.100".to_string()),
                );
                static_ip
                    .set_sensitive(matches!(&ipv4_config, Some((Ipv4Method::Manual, _, _, _))));
                expander.add_row(&static_ip);

                let subnet = adw::EntryRow::new();
                subnet.set_title(&tr("Subnet Mask"));
                subnet.set_text(
                    &ipv4_config
                        .as_ref()
                        .map(|(_, _, s, _)| s.clone())
                        .unwrap_or_else(|| "255.255.255.0".to_string()),
                );
                subnet.set_sensitive(matches!(&ipv4_config, Some((Ipv4Method::Manual, _, _, _))));
                expander.add_row(&subnet);

                let gateway = adw::EntryRow::new();
                gateway.set_title(&tr("Gateway"));
                gateway.set_text(
                    &ipv4_config
                        .as_ref()
                        .map(|(_, _, _, g)| g.clone())
                        .unwrap_or_else(|| "192.168.1.1".to_string()),
                );
                gateway.set_sensitive(matches!(&ipv4_config, Some((Ipv4Method::Manual, _, _, _))));
                expander.add_row(&gateway);

                // DNS Configuration
                let has_manual_dns = dns_config.is_some();
                let dns_method_model = string_list(&[
                    tr("Automatic (from DHCP)"),
                    // TRANSLATORS: DNS method or proxy mode: entered by hand
                    tr("Manual"),
                ]);
                let dns_method_row = adw::ComboRow::builder()
                    .title(tr("DNS Configuration"))
                    .model(&dns_method_model)
                    .selected(if has_manual_dns { 1 } else { 0 })
                    .build();
                expander.add_row(&dns_method_row);

                let dns1 = adw::EntryRow::new();
                dns1.set_title(&tr("Primary DNS"));
                dns1.set_text(
                    &dns_config
                        .as_ref()
                        .map(|(d1, _)| d1.clone())
                        .unwrap_or_else(|| "8.8.8.8".to_string()),
                );
                dns1.set_sensitive(has_manual_dns);
                expander.add_row(&dns1);

                let dns2 = adw::EntryRow::new();
                dns2.set_title(&tr("Secondary DNS"));
                dns2.set_text(
                    &dns_config
                        .as_ref()
                        .map(|(_, d2)| d2.clone())
                        .unwrap_or_else(|| "8.8.4.4".to_string()),
                );
                dns2.set_sensitive(has_manual_dns);
                expander.add_row(&dns2);

                // WiFi-specific: SSID selection
                let wifi_ssid_entry = if adapter.adapter_type == AdapterType::Wifi {
                    let ssid_entry = adw::EntryRow::new();
                    ssid_entry.set_title(&tr("WiFi Network (SSID)"));
                    ssid_entry.set_text(&wifi_ssid.unwrap_or_default());
                    ssid_entry.set_sensitive(iface_enabled);
                    expander.add_row(&ssid_entry);
                    Some(ssid_entry)
                } else {
                    None
                };

                // Connect IP method change to enable/disable static fields
                let static_ip_weak = static_ip.downgrade();
                let subnet_weak = subnet.downgrade();
                let gateway_weak = gateway.downgrade();
                ip_method_row.connect_selected_notify(move |row| {
                    let is_static = row.selected() == 1;
                    if let Some(e) = static_ip_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                    if let Some(e) = subnet_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                    if let Some(e) = gateway_weak.upgrade() {
                        e.set_sensitive(is_static);
                    }
                });

                // Connect DNS method change to enable/disable DNS fields
                let dns1_weak = dns1.downgrade();
                let dns2_weak = dns2.downgrade();
                dns_method_row.connect_selected_notify(move |row| {
                    let is_manual = row.selected() == 1;
                    if let Some(e) = dns1_weak.upgrade() {
                        e.set_sensitive(is_manual);
                    }
                    if let Some(e) = dns2_weak.upgrade() {
                        e.set_sensitive(is_manual);
                    }
                });

                // Connect enable switch to toggle all settings; re-enabling restores
                // the fields that the selected methods allow
                let widgets_weak = Rc::downgrade(&adapter_widgets);
                let iface = adapter.name.clone();
                enabled_row.connect_active_notify(move |_| {
                    if let Some(widgets) = widgets_weak.upgrade() {
                        if let Some(w) = widgets.borrow().get(&iface) {
                            sync_adapter_sensitivity(w);
                        }
                    }
                });

                let widgets = (
                    enabled_row,
                    ip_method_row,
                    static_ip,
                    subnet,
                    gateway,
                    dns_method_row,
                    dns1,
                    dns2,
                    wifi_ssid_entry,
                );
                // A disabled adapter starts with its detail rows insensitive
                sync_adapter_sensitivity(&widgets);
                adapter_widgets
                    .borrow_mut()
                    .insert(adapter.name.clone(), widgets);

                adapters_group.add(&expander);
            }
        }

        content.append(&adapters_group);

        // === VPN Connection ===
        let vpn_group = adw::PreferencesGroup::new();
        vpn_group.set_title(&tr("VPN Connection"));
        vpn_group.set_description(Some(&tr(
            "Optionally connect to a VPN when this profile is active",
        )));

        // Find existing VPN config
        let existing_vpn = profile.network_actions.iter().find_map(|a| {
            if let NetworkAction::VpnConnect { connection_name } = a {
                Some(connection_name.clone())
            } else {
                None
            }
        });

        let vpn_enabled = adw::SwitchRow::builder()
            .title(tr("Connect to VPN"))
            .subtitle(tr("Use a VPN connection from NetworkManager"))
            .active(existing_vpn.is_some())
            .build();
        vpn_group.add(&vpn_enabled);

        let vpn_name = adw::EntryRow::new();
        vpn_name.set_title(&tr("VPN Connection Name"));
        vpn_name.set_text(&existing_vpn.unwrap_or_default());
        vpn_name.set_sensitive(vpn_enabled.is_active());
        vpn_group.add(&vpn_name);

        let vpn_name_weak = vpn_name.downgrade();
        vpn_enabled.connect_active_notify(move |row| {
            if let Some(e) = vpn_name_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        content.append(&vpn_group);

        // === Proxy Settings ===
        let proxy_group = adw::PreferencesGroup::new();
        proxy_group.set_title(&tr("Proxy Settings"));
        proxy_group.set_description(Some(&tr("Configure system proxy via gsettings")));

        // Find existing proxy config
        let existing_proxy = profile.system_actions.iter().find_map(|a| {
            if let SystemAction::ProxyConfig(cfg) = a {
                Some(cfg.clone())
            } else {
                None
            }
        });

        let proxy_mode_idx = existing_proxy
            .as_ref()
            .map(|p| match p.mode {
                ProxyMode::None => 0,
                ProxyMode::Manual => 1,
                ProxyMode::Auto => 2,
            })
            .unwrap_or(0);

        let proxy_mode_model = string_list(&[
            // TRANSLATORS: proxy mode: no proxy is used
            tr("None"),
            // TRANSLATORS: DNS method or proxy mode: entered by hand
            tr("Manual"),
            tr("Automatic (PAC)"),
        ]);
        let proxy_mode_row = adw::ComboRow::builder()
            .title(tr("Proxy Mode"))
            .model(&proxy_mode_model)
            .selected(proxy_mode_idx)
            .build();
        proxy_group.add(&proxy_mode_row);

        let http_proxy = adw::EntryRow::new();
        http_proxy.set_title(&tr("HTTP Proxy"));
        http_proxy.set_text(
            &existing_proxy
                .as_ref()
                .and_then(|p| p.http_proxy.clone())
                .unwrap_or_else(|| "http://proxy:8080".to_string()),
        );
        http_proxy.set_sensitive(proxy_mode_idx == 1);
        proxy_group.add(&http_proxy);

        let https_proxy = adw::EntryRow::new();
        https_proxy.set_title(&tr("HTTPS Proxy"));
        https_proxy.set_text(
            &existing_proxy
                .as_ref()
                .and_then(|p| p.https_proxy.clone())
                .unwrap_or_else(|| "http://proxy:8080".to_string()),
        );
        https_proxy.set_sensitive(proxy_mode_idx == 1);
        proxy_group.add(&https_proxy);

        let no_proxy = adw::EntryRow::new();
        no_proxy.set_title(&tr("No Proxy For"));
        no_proxy.set_text(
            &existing_proxy
                .as_ref()
                .map(|p| p.no_proxy.join(","))
                .unwrap_or_else(|| "localhost,127.0.0.1".to_string()),
        );
        no_proxy.set_sensitive(proxy_mode_idx == 1);
        proxy_group.add(&no_proxy);

        let pac_url = adw::EntryRow::new();
        pac_url.set_title(&tr("PAC URL"));
        pac_url.set_text(
            &existing_proxy
                .as_ref()
                .and_then(|p| p.pac_url.clone())
                .unwrap_or_default(),
        );
        pac_url.set_sensitive(proxy_mode_idx == 2);
        proxy_group.add(&pac_url);

        let http_proxy_weak = http_proxy.downgrade();
        let https_proxy_weak = https_proxy.downgrade();
        let no_proxy_weak = no_proxy.downgrade();
        let pac_url_weak = pac_url.downgrade();
        proxy_mode_row.connect_selected_notify(move |row| {
            let mode = row.selected();
            let is_manual = mode == 1;
            let is_auto = mode == 2;
            if let Some(e) = http_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = https_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = no_proxy_weak.upgrade() {
                e.set_sensitive(is_manual);
            }
            if let Some(e) = pac_url_weak.upgrade() {
                e.set_sensitive(is_auto);
            }
        });

        content.append(&proxy_group);

        // === Scripts/Programs ===
        let scripts_group = adw::PreferencesGroup::new();
        scripts_group.set_title(&tr("Scripts and Programs"));
        scripts_group.set_description(Some(&tr("Run scripts when profile activates")));

        // Find existing script/program configs
        let existing_pre_script = profile.automation_actions.iter().find_map(|a| {
            if let AutomationAction::PreScript { path, .. } = a {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        });

        let existing_post_script = profile.automation_actions.iter().find_map(|a| {
            if let AutomationAction::PostScript { path, .. } = a {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        });

        let existing_program = profile.automation_actions.iter().find_map(|a| {
            if let AutomationAction::RunProgram { program, args, .. } = a {
                Some((program.clone(), args.join(" ")))
            } else {
                None
            }
        });

        let pre_script_enabled = adw::SwitchRow::builder()
            .title(tr("Run Pre-Script"))
            .subtitle(tr("Execute script before applying profile"))
            .active(existing_pre_script.is_some())
            .build();
        scripts_group.add(&pre_script_enabled);

        let pre_script_path = adw::EntryRow::new();
        pre_script_path.set_title(&tr("Pre-Script Path"));
        pre_script_path
            .set_text(&existing_pre_script.unwrap_or_else(|| "/path/to/script.sh".to_string()));
        pre_script_path.set_sensitive(pre_script_enabled.is_active());
        scripts_group.add(&pre_script_path);

        let post_script_enabled = adw::SwitchRow::builder()
            .title(tr("Run Post-Script"))
            .subtitle(tr("Execute script after applying profile"))
            .active(existing_post_script.is_some())
            .build();
        scripts_group.add(&post_script_enabled);

        let post_script_path = adw::EntryRow::new();
        post_script_path.set_title(&tr("Post-Script Path"));
        post_script_path
            .set_text(&existing_post_script.unwrap_or_else(|| "/path/to/script.sh".to_string()));
        post_script_path.set_sensitive(post_script_enabled.is_active());
        scripts_group.add(&post_script_path);

        let run_program_enabled = adw::SwitchRow::builder()
            .title(tr("Run Program"))
            .subtitle(tr("Launch an application when profile activates"))
            .active(existing_program.is_some())
            .build();
        scripts_group.add(&run_program_enabled);

        let run_program_path = adw::EntryRow::new();
        run_program_path.set_title(&tr("Program Path"));
        run_program_path.set_text(
            &existing_program
                .as_ref()
                .map(|(p, _)| p.clone())
                .unwrap_or_default(),
        );
        run_program_path.set_sensitive(run_program_enabled.is_active());
        scripts_group.add(&run_program_path);

        let run_program_args = adw::EntryRow::new();
        run_program_args.set_title(&tr("Program Arguments"));
        run_program_args.set_text(
            &existing_program
                .as_ref()
                .map(|(_, a)| a.clone())
                .unwrap_or_default(),
        );
        run_program_args.set_sensitive(run_program_enabled.is_active());
        scripts_group.add(&run_program_args);

        // Script toggle handlers
        let pre_script_path_weak = pre_script_path.downgrade();
        pre_script_enabled.connect_active_notify(move |row| {
            if let Some(e) = pre_script_path_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        let post_script_path_weak = post_script_path.downgrade();
        post_script_enabled.connect_active_notify(move |row| {
            if let Some(e) = post_script_path_weak.upgrade() {
                e.set_sensitive(row.is_active());
            }
        });

        let run_program_path_weak = run_program_path.downgrade();
        let run_program_args_weak = run_program_args.downgrade();
        run_program_enabled.connect_active_notify(move |row| {
            let enabled = row.is_active();
            if let Some(e) = run_program_path_weak.upgrade() {
                e.set_sensitive(enabled);
            }
            if let Some(e) = run_program_args_weak.upgrade() {
                e.set_sensitive(enabled);
            }
        });

        content.append(&scripts_group);

        scroll.set_child(Some(&content));
        toolbar_view.set_content(Some(&scroll));
        dialog.set_content(Some(&toolbar_view));

        // Cancel action
        let dialog_weak = dialog.downgrade();
        cancel_btn.connect_clicked(move |_| {
            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }
        });

        // Create weak references for save handler
        let dialog_weak = dialog.downgrade();
        let window_weak = self.downgrade();
        let profile_id = profile_id.to_string();
        let name_entry_weak = name_entry.downgrade();
        let desc_entry_weak = desc_entry.downgrade();
        let group_entry_weak = group_entry.downgrade();
        // VPN
        let vpn_enabled_weak = vpn_enabled.downgrade();
        let vpn_name_weak = vpn_name.downgrade();
        // Proxy
        let proxy_mode_row_weak = proxy_mode_row.downgrade();
        let http_proxy_weak = http_proxy.downgrade();
        let https_proxy_weak = https_proxy.downgrade();
        let no_proxy_weak = no_proxy.downgrade();
        let pac_url_weak = pac_url.downgrade();
        // Scripts
        let pre_script_enabled_weak = pre_script_enabled.downgrade();
        let pre_script_path_weak = pre_script_path.downgrade();
        let post_script_enabled_weak = post_script_enabled.downgrade();
        let post_script_path_weak = post_script_path.downgrade();
        let run_program_enabled_weak = run_program_enabled.downgrade();
        let run_program_path_weak = run_program_path.downgrade();
        let run_program_args_weak = run_program_args.downgrade();
        // Adapter widgets
        let adapter_widgets_ref = adapter_widgets.clone();
        let error_banner_weak = error_banner.downgrade();

        save_btn.connect_clicked(move |_| {
            let new_name = name_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let new_desc = desc_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let new_group = group_entry_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // VPN settings
            let vpn_on = vpn_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let vpn_name_val = vpn_name_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Proxy settings
            let proxy_mode = proxy_mode_row_weak
                .upgrade()
                .map(|r| r.selected())
                .unwrap_or(0);
            let http_proxy_val = http_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let https_proxy_val = https_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let no_proxy_val = no_proxy_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let pac_url_val = pac_url_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Script settings
            let pre_script_on = pre_script_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let pre_script = pre_script_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let post_script_on = post_script_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let post_script = post_script_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let run_prog_on = run_program_enabled_weak
                .upgrade()
                .map(|r| r.is_active())
                .unwrap_or(false);
            let run_prog_path = run_program_path_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();
            let run_prog_args = run_program_args_weak
                .upgrade()
                .map(|e| e.text().to_string())
                .unwrap_or_default();

            // Collect adapter configurations
            let adapter_configs: Vec<AdapterConfig> = adapter_widgets_ref
                .borrow()
                .iter()
                .map(|(iface, widgets)| {
                    let (
                        enabled,
                        ip_method,
                        static_ip,
                        subnet,
                        gateway,
                        dns_method,
                        dns1,
                        dns2,
                        wifi_ssid,
                    ) = widgets;
                    (
                        iface.clone(),
                        enabled.is_active(),
                        ip_method.selected(),
                        static_ip.text().to_string(),
                        subnet.text().to_string(),
                        gateway.text().to_string(),
                        dns_method.selected(),
                        dns1.text().to_string(),
                        dns2.text().to_string(),
                        wifi_ssid.as_ref().map(|e| e.text().to_string()),
                    )
                })
                .collect();

            // Refuse to save while the name is empty or an adapter form is invalid
            let validation_error = if new_name.trim().is_empty() {
                Some(tr("Enter a profile name"))
            } else {
                Self::validate_adapter_widgets(&adapter_widgets_ref.borrow())
            };
            if let Some(message) = validation_error {
                if let Some(banner) = error_banner_weak.upgrade() {
                    banner.set_title(&message);
                    banner.set_revealed(true);
                }
                return;
            }

            if let Some(d) = dialog_weak.upgrade() {
                d.close();
            }

            if let Some(window) = window_weak.upgrade() {
                use crate::models::{
                    AutomationAction, NetworkAction, ProxyConfig, ProxyMode, SystemAction,
                };
                use std::collections::HashSet;
                use std::path::PathBuf;

                let imp = window.imp();

                // Update profile in storage
                {
                    let mut profiles = imp.profiles.borrow_mut();
                    if let Some(profile) = profiles
                        .iter_mut()
                        .find(|p| p.id().to_string() == profile_id)
                    {
                        // Update metadata
                        profile.metadata.name = new_name.clone();
                        profile.metadata.description = if new_desc.is_empty() {
                            None
                        } else {
                            Some(new_desc)
                        };
                        profile.metadata.group = if new_group.is_empty() {
                            None
                        } else {
                            Some(crate::models::ProfileGroup::new(&new_group))
                        };
                        profile.metadata.updated_at = chrono::Utc::now();

                        // Replace only what this dialog manages. Actions for adapters that
                        // are not present right now, action types the dialog cannot show
                        // and extra script settings are kept as they are.
                        let shown_ifaces: HashSet<&str> =
                            adapter_configs.iter().map(|c| c.0.as_str()).collect();
                        let wifi_ifaces: HashSet<&str> = adapter_configs
                            .iter()
                            .filter(|c| c.9.is_some())
                            .map(|c| c.0.as_str())
                            .collect();
                        // The dialog shows the first VPN connection only
                        let first_vpn = profile
                            .network_actions
                            .iter()
                            .position(|a| matches!(a, NetworkAction::VpnConnect { .. }));
                        let mut index = 0;
                        profile.network_actions.retain(|action| {
                            let position = index;
                            index += 1;
                            match action {
                                NetworkAction::InterfaceEnable(state) => {
                                    !shown_ifaces.contains(state.interface.as_str())
                                }
                                NetworkAction::Ipv4Config {
                                    interface: Some(iface),
                                    ..
                                }
                                | NetworkAction::DnsServers {
                                    interface: Some(iface),
                                    ..
                                } => !shown_ifaces.contains(iface.as_str()),
                                NetworkAction::WifiConnect {
                                    interface: Some(iface),
                                    ..
                                } => !wifi_ifaces.contains(iface.as_str()),
                                NetworkAction::VpnConnect { .. } => Some(position) != first_vpn,
                                _ => true,
                            }
                        });

                        // === Process each adapter configuration ===
                        for config in &adapter_configs {
                            profile
                                .network_actions
                                .extend(MainWindow::adapter_network_actions(config));
                        }

                        // === VPN Connection ===
                        if vpn_on && !vpn_name_val.is_empty() {
                            profile.network_actions.push(NetworkAction::VpnConnect {
                                connection_name: vpn_name_val,
                            });
                        }

                        // === Proxy Configuration === (the dialog shows the first one)
                        let proxy_pos = profile
                            .system_actions
                            .iter()
                            .position(|a| matches!(a, SystemAction::ProxyConfig(_)));
                        let previous_proxy =
                            proxy_pos.and_then(|pos| match profile.system_actions.remove(pos) {
                                SystemAction::ProxyConfig(cfg) => Some(cfg),
                                _ => None,
                            });
                        if proxy_mode > 0 {
                            let mode = match proxy_mode {
                                1 => ProxyMode::Manual,
                                2 => ProxyMode::Auto,
                                _ => ProxyMode::None,
                            };

                            let no_proxy_list: Vec<String> = no_proxy_val
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();

                            let action = SystemAction::ProxyConfig(ProxyConfig {
                                mode,
                                http_proxy: if proxy_mode == 1 && !http_proxy_val.is_empty() {
                                    Some(http_proxy_val)
                                } else {
                                    None
                                },
                                https_proxy: if proxy_mode == 1 && !https_proxy_val.is_empty() {
                                    Some(https_proxy_val)
                                } else {
                                    None
                                },
                                // Not editable here; keep what the profile already had
                                ftp_proxy: previous_proxy
                                    .as_ref()
                                    .and_then(|p| p.ftp_proxy.clone()),
                                socks_proxy: previous_proxy
                                    .as_ref()
                                    .and_then(|p| p.socks_proxy.clone()),
                                no_proxy: no_proxy_list,
                                pac_url: if proxy_mode == 2 && !pac_url_val.is_empty() {
                                    Some(pac_url_val)
                                } else {
                                    None
                                },
                            });
                            match proxy_pos {
                                Some(pos) => profile.system_actions.insert(pos, action),
                                None => profile.system_actions.push(action),
                            }
                        }

                        // === Scripts and programs === (the dialog shows the first of each
                        // kind; existing arguments, environment and options are kept)
                        let pre_wanted = pre_script_on && !pre_script.is_empty();
                        match profile
                            .automation_actions
                            .iter()
                            .position(|a| matches!(a, AutomationAction::PreScript { .. }))
                        {
                            Some(pos) if pre_wanted => {
                                if let AutomationAction::PreScript { path, .. } =
                                    &mut profile.automation_actions[pos]
                                {
                                    *path = PathBuf::from(&pre_script);
                                }
                            }
                            Some(pos) => {
                                profile.automation_actions.remove(pos);
                            }
                            None if pre_wanted => {
                                profile
                                    .automation_actions
                                    .push(AutomationAction::PreScript {
                                        path: PathBuf::from(&pre_script),
                                        args: vec![],
                                        env: HashMap::new(),
                                        mode: crate::models::ScriptMode::Wait,
                                        working_dir: None,
                                        continue_on_error: false,
                                    })
                            }
                            None => {}
                        }

                        let post_wanted = post_script_on && !post_script.is_empty();
                        match profile
                            .automation_actions
                            .iter()
                            .position(|a| matches!(a, AutomationAction::PostScript { .. }))
                        {
                            Some(pos) if post_wanted => {
                                if let AutomationAction::PostScript { path, .. } =
                                    &mut profile.automation_actions[pos]
                                {
                                    *path = PathBuf::from(&post_script);
                                }
                            }
                            Some(pos) => {
                                profile.automation_actions.remove(pos);
                            }
                            None if post_wanted => {
                                profile
                                    .automation_actions
                                    .push(AutomationAction::PostScript {
                                        path: PathBuf::from(&post_script),
                                        args: vec![],
                                        env: HashMap::new(),
                                        mode: crate::models::ScriptMode::Wait,
                                        working_dir: None,
                                        continue_on_error: false,
                                    })
                            }
                            None => {}
                        }

                        let program_wanted = run_prog_on && !run_prog_path.is_empty();
                        let program_args: Vec<String> = run_prog_args
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect();
                        match profile
                            .automation_actions
                            .iter()
                            .position(|a| matches!(a, AutomationAction::RunProgram { .. }))
                        {
                            Some(pos) if program_wanted => {
                                if let AutomationAction::RunProgram { program, args, .. } =
                                    &mut profile.automation_actions[pos]
                                {
                                    *program = run_prog_path;
                                    *args = program_args;
                                }
                            }
                            Some(pos) => {
                                profile.automation_actions.remove(pos);
                            }
                            None if program_wanted => {
                                profile
                                    .automation_actions
                                    .push(AutomationAction::RunProgram {
                                        program: run_prog_path,
                                        args: program_args,
                                        env: HashMap::new(),
                                        mode: crate::models::ProgramMode::Background,
                                        working_dir: None,
                                    })
                            }
                            None => {}
                        }
                    }
                }

                // Persist the edit
                window.save_profiles_to_cache();

                // Refresh profiles page
                if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
                    let profiles = imp.profiles.borrow().clone();
                    profiles_page.update_profiles(profiles);
                }

                window.show_toast(&tr("Profile '{name}' updated").replace("{name}", &new_name));
            }
        });

        dialog_window::present(&dialog, Some(self));
    }

    /// Convert prefix length to subnet mask string
    fn prefix_to_subnet(prefix: u8) -> String {
        match prefix {
            32 => "255.255.255.255",
            31 => "255.255.255.254",
            30 => "255.255.255.252",
            29 => "255.255.255.248",
            28 => "255.255.255.240",
            27 => "255.255.255.224",
            26 => "255.255.255.192",
            25 => "255.255.255.128",
            24 => "255.255.255.0",
            23 => "255.255.254.0",
            22 => "255.255.252.0",
            21 => "255.255.248.0",
            20 => "255.255.240.0",
            19 => "255.255.224.0",
            18 => "255.255.192.0",
            17 => "255.255.128.0",
            16 => "255.255.0.0",
            15 => "255.254.0.0",
            14 => "255.252.0.0",
            13 => "255.248.0.0",
            12 => "255.240.0.0",
            11 => "255.224.0.0",
            10 => "255.192.0.0",
            9 => "255.128.0.0",
            8 => "255.0.0.0",
            _ => "255.255.255.0",
        }
        .to_string()
    }

    /// Show delete profile confirmation dialog
    fn show_delete_profile_dialog(&self, profile_id: &str) {
        let imp = self.imp();

        // Find the profile name
        let profile_name = {
            let profiles = imp.profiles.borrow();
            profiles
                .iter()
                .find(|p| p.id().to_string() == profile_id)
                .map(|p| p.name().to_string())
        };

        let Some(name) = profile_name else {
            self.show_toast(&tr("Profile not found"));
            return;
        };

        let dialog = adw::AlertDialog::new(
            Some(&tr("Delete Profile?")),
            Some(
                &tr("Are you sure you want to delete \"{name}\"? This action cannot be undone.")
                    .replace("{name}", &name),
            ),
        );

        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("delete", &tr("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let window_weak = self.downgrade();
        let profile_id = profile_id.to_string();
        let profile_name = name.clone();

        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                if let Some(window) = window_weak.upgrade() {
                    window.delete_profile(&profile_id, &profile_name);
                }
            }
        });

        dialog.present(Some(self));
    }

    /// Actually delete a profile
    fn delete_profile(&self, profile_id: &str, profile_name: &str) {
        let imp = self.imp();

        // Remove from storage
        {
            let mut profiles = imp.profiles.borrow_mut();
            profiles.retain(|p| p.id().to_string() != profile_id);
        }

        // Save to cache
        self.save_profiles_to_cache();

        // Do not leave the watchdog pointing at a profile that no longer exists.
        if let Some(app) = self
            .application()
            .and_downcast::<crate::application::Application>()
        {
            let mut config = app.config();
            if config.watchdog.fallback_profile_id.as_deref() == Some(profile_id) {
                config.watchdog.fallback_profile_id = None;
                app.update_config(config);
                if let Some(settings_page) = imp.settings_page.borrow().as_ref() {
                    settings_page.set_profiles(imp.profiles.borrow().clone());
                }
            }
        }

        // Log the profile deletion
        if let Some(store) = imp.data_store.borrow().as_ref() {
            store.append_log("INFO", &format!("Profile '{}' deleted", profile_name));
        }

        // Refresh logs page
        if let Some(logs_page) = imp.logs_page.borrow().as_ref() {
            logs_page.refresh_logs();
        }

        // Refresh profiles page
        if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
            let profiles = imp.profiles.borrow().clone();
            profiles_page.update_profiles(profiles);
        }

        self.show_toast(&tr("Profile '{name}' deleted").replace("{name}", profile_name));
    }

    /// Show a dialog to select and switch to a profile
    fn show_profile_switcher_dialog(&self) {
        let imp = self.imp();
        let profiles = imp.profiles.borrow().clone();

        if profiles.is_empty() {
            self.show_toast(&tr("No profiles available. Create one first!"));
            return;
        }

        // Create the dialog
        let dialog = dialog_window::new(&tr("Switch Profile"), 400, 450);

        // Create content
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Header bar
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .show_start_title_buttons(false)
            .build();
        content.append(&header);

        // Description
        let desc_label = gtk::Label::builder()
            .label(tr("Select a profile to apply its network configuration:"))
            .wrap(true)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(18)
            .margin_end(18)
            .xalign(0.0)
            .build();
        content.append(&desc_label);

        // Scrolled window for profile list
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .margin_start(18)
            .margin_end(18)
            .margin_bottom(18)
            .build();
        list_box.add_css_class("boxed-list");

        // Add each profile as a row
        for profile in &profiles {
            let profile_name = profile.name();
            let profile_desc = profile.metadata.description.as_deref().unwrap_or("");

            // Profile names are user text, not Pango markup
            let row = adw::ActionRow::builder()
                .title(profile_name)
                .subtitle(profile_desc)
                .use_markup(false)
                .activatable(true)
                .build();

            // Add icon based on profile type
            let icon_name = if profile_name.to_lowercase().contains("wifi") {
                "network-wireless-symbolic"
            } else if profile_name.to_lowercase().contains("ethernet")
                || profile_name.to_lowercase().contains("wired")
            {
                "network-wired-symbolic"
            } else if profile_name.to_lowercase().contains("vpn") {
                "network-vpn-symbolic"
            } else {
                "network-workgroup-symbolic"
            };

            let icon = gtk::Image::from_icon_name(icon_name);
            row.add_prefix(&icon);

            // Add apply arrow
            let arrow = gtk::Image::from_icon_name("go-next-symbolic");
            row.add_suffix(&arrow);

            // Store profile ID
            let profile_id = profile.id().to_string();
            let dialog_weak = dialog.downgrade();
            let window_weak = self.downgrade();

            row.connect_activated(move |_| {
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.close();
                }
                if let Some(window) = window_weak.upgrade() {
                    window.apply_profile(&profile_id);
                }
            });

            list_box.append(&row);
        }

        scrolled.set_child(Some(&list_box));
        content.append(&scrolled);

        dialog.set_content(Some(&content));
        dialog_window::present(&dialog, Some(self));
    }

    /// Apply a profile - execute network configuration
    pub fn apply_profile(&self, profile_id: &str) {
        self.apply_profile_internal(profile_id, false);
    }

    /// Apply a profile from auto-switch - may skip automation actions.
    pub fn apply_profile_auto(&self, profile_id: &str) {
        self.apply_profile_internal(profile_id, true);
    }

    fn apply_profile_internal(&self, profile_id: &str, is_auto_switch: bool) {
        let imp = self.imp();

        if imp.apply_in_progress.get() {
            self.show_toast(&tr("Another profile is still being applied"));
            return;
        }

        // Find the profile
        let profile_opt = {
            let profiles = imp.profiles.borrow();
            profiles
                .iter()
                .find(|p| p.id().to_string() == profile_id)
                .cloned()
        };

        let Some(profile) = profile_opt else {
            self.show_toast(&tr("Profile not found"));
            return;
        };

        // Security: warn about scripts in unapproved profiles
        if !profile.approved_for_users && profile_has_automation(&profile) {
            self.show_script_safety_dialog(&profile, is_auto_switch);
            return;
        }

        // Check if confirmation is required
        let confirm_required = self
            .application()
            .and_downcast_ref::<crate::application::Application>()
            .map(|app| app.config().confirm_profile_switch)
            .unwrap_or(true);

        if confirm_required && profile.has_actions() {
            self.show_apply_confirmation_dialog(&profile, is_auto_switch);
        } else {
            self.do_apply_profile(&profile, is_auto_switch);
        }
    }

    /// Show confirmation dialog before applying a profile.
    fn show_apply_confirmation_dialog(&self, profile: &Profile, is_auto_switch: bool) {
        let total = profile.action_count();
        let network = profile.network_actions.len();
        let system = profile.system_actions.len();
        let automation = profile.automation_actions.len();
        let body = format!(
            "{}\n• {}\n• {}\n• {}",
            ntr(
                "This will apply {n} action to your system:",
                "This will apply {n} actions to your system:",
                total as u32,
            )
            .replace("{n}", &total.to_string()),
            ntr("{n} network action", "{n} network actions", network as u32)
                .replace("{n}", &network.to_string()),
            ntr("{n} system action", "{n} system actions", system as u32)
                .replace("{n}", &system.to_string()),
            ntr(
                "{n} automation action",
                "{n} automation actions",
                automation as u32,
            )
            .replace("{n}", &automation.to_string()),
        );
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Apply '{name}'?").replace("{name}", profile.name()))
            .body(body)
            .build();

        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("apply", &tr("Apply"));
        dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("apply"));
        dialog.set_close_response("cancel");

        let profile_clone = profile.clone();
        let window_weak = self.downgrade();
        dialog.connect_response(None, move |_, response| {
            if response == "apply" {
                if let Some(window) = window_weak.upgrade() {
                    window.do_apply_profile(&profile_clone, is_auto_switch);
                }
            }
        });

        dialog.present(Some(self));
    }

    /// Show a security warning when applying a profile with automation actions
    /// that has not been marked as approved_for_users.
    fn show_script_safety_dialog(&self, profile: &Profile, is_auto_switch: bool) {
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Security Warning"))
            .body(
                tr("The profile '{name}' contains executable scripts or programs that have not been marked as trusted.\n\nOnly run profiles from trusted sources.")
                    .replace("{name}", profile.name()),
            )
            .build();

        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("run", &tr("Run Anyway"));
        dialog.set_response_appearance("run", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let profile_clone = profile.clone();
        let window_weak = self.downgrade();
        dialog.connect_response(None, move |_, response| {
            if response == "run" {
                tracing::warn!(
                    "User overrode script safety warning for profile '{}'",
                    profile_clone.name()
                );
                if let Some(window) = window_weak.upgrade() {
                    window.do_apply_profile(&profile_clone, is_auto_switch);
                }
            }
        });

        dialog.present(Some(self));
    }

    /// Actually apply a profile's actions.
    ///
    /// The commands run on a worker thread so the window stays responsive;
    /// the result is shown on the main thread once they have finished.
    fn do_apply_profile(&self, profile: &Profile, is_auto_switch: bool) {
        let imp = self.imp();

        // Only one profile is applied at a time
        if imp.apply_in_progress.get() {
            self.show_toast(&tr("Another profile is still being applied"));
            return;
        }

        let config = self
            .application()
            .and_downcast_ref::<crate::application::Application>()
            .map(|app| app.config());
        let allow_scripts = config
            .as_ref()
            .map(|c| c.auto_switch_allow_scripts)
            .unwrap_or(false);
        let sandbox_mode = config.map(|c| c.sandbox_mode).unwrap_or_default();

        // Automation actions are skipped during auto-switch unless explicitly allowed
        let run_automation = !is_auto_switch || allow_scripts;
        if !run_automation {
            tracing::info!("Auto-switch: skipping automation actions for profile '{}' (auto_switch_allow_scripts is false)", profile.name());
        }

        imp.apply_in_progress.set(true);

        // Shown until the actions have finished
        let progress_toast =
            adw::Toast::new(&tr("Applying profile '{name}'…").replace("{name}", profile.name()));
        progress_toast.set_use_markup(false);
        progress_toast.set_timeout(0);
        if let Some(toast_overlay) = imp.toast_overlay.borrow().as_ref() {
            toast_overlay.add_toast(progress_toast.clone());
        }

        let job = profile.clone();
        let profile_id = profile.id().to_string();
        let profile_name = profile.name().to_string();
        let window_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                Self::execute_profile_actions(&job, run_automation, sandbox_mode)
            })
            .await;

            progress_toast.dismiss();
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            window.imp().apply_in_progress.set(false);
            match result {
                Ok(report) => window.finish_apply_profile(&profile_id, &profile_name, report),
                Err(_) => {
                    tracing::error!("Applying profile '{}' panicked", profile_name);
                    window.show_toast(
                        &tr("Profile '{name}' could not be applied")
                            .replace("{name}", &profile_name),
                    );
                }
            }
        });
    }

    /// Run a profile's actions in order: pre-scripts, network actions, system
    /// actions, then post-scripts and programs. Runs on a worker thread.
    fn execute_profile_actions(
        profile: &Profile,
        run_automation: bool,
        sandbox_mode: crate::models::SandboxMode,
    ) -> ApplyReport {
        use crate::models::AutomationAction;

        let mut report = ApplyReport::default();
        let (pre_scripts, post_actions): (Vec<&AutomationAction>, Vec<&AutomationAction>) =
            if run_automation {
                profile
                    .automation_actions
                    .iter()
                    .partition(|a| matches!(a, AutomationAction::PreScript { .. }))
            } else {
                (Vec::new(), Vec::new())
            };

        // A failing pre-script stops the profile unless it allows errors
        for action in pre_scripts {
            tracing::info!("Running pre-script for profile '{}'", profile.name());
            if !report.record(Self::run_automation_action(action, sandbox_mode))
                && !Self::continues_on_error(action)
            {
                report.aborted = true;
                return report;
            }
        }

        for action in &profile.network_actions {
            report.record(Self::apply_network_action(action));
        }

        for action in &profile.system_actions {
            report.record(Self::apply_system_action(action));
        }

        for action in post_actions {
            tracing::info!("Running automation action for profile '{}'", profile.name());
            if !report.record(Self::run_automation_action(action, sandbox_mode))
                && !Self::continues_on_error(action)
            {
                break;
            }
        }

        report
    }

    /// Whether a failed automation action lets the following ones run.
    fn continues_on_error(action: &crate::models::AutomationAction) -> bool {
        use crate::models::AutomationAction;
        match action {
            AutomationAction::PreScript {
                continue_on_error, ..
            }
            | AutomationAction::PostScript {
                continue_on_error, ..
            } => *continue_on_error,
            _ => true,
        }
    }

    /// Record a finished apply and update the UI (main thread).
    fn finish_apply_profile(&self, profile_id: &str, profile_name: &str, report: ApplyReport) {
        use crate::models::profile::ProfileStatus;
        let imp = self.imp();

        for msg in &report.errors {
            tracing::warn!("Profile apply error: {}", msg);
        }
        for msg in &report.skipped {
            tracing::info!("Profile apply skipped: {}", msg);
        }

        if report.aborted {
            self.show_toast(
                &tr("Profile '{name}' was not applied: a pre-script failed")
                    .replace("{name}", profile_name),
            );
            return;
        }

        // Make this the only active profile and persist that
        let applied_at = {
            let mut profiles = imp.profiles.borrow_mut();
            for p in profiles
                .iter_mut()
                .filter(|p| p.status == ProfileStatus::Active)
            {
                p.status = ProfileStatus::Inactive;
            }
            profiles
                .iter_mut()
                .find(|p| p.id().to_string() == profile_id)
                .and_then(|p| {
                    p.mark_applied();
                    p.metadata.last_applied_at
                })
        };
        self.save_profiles_to_cache();

        // Update active profile in profiles page
        if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
            profiles_page.set_active_profile(Some(profile_id));
            let profiles = imp.profiles.borrow().clone();
            profiles_page.update_profiles(profiles);
        }

        // Update the dashboard with active profile info
        if let Some(dashboard_page) = imp.dashboard_page.borrow().as_ref() {
            let applied_time = applied_at
                .unwrap_or_else(chrono::Utc::now)
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            dashboard_page.update_active_profile(Some(profile_name), Some(&applied_time));
            // Immediate refresh to update UI
            dashboard_page.update_network_info();
        }

        // Refresh dashboard network info again after a short delay to allow network changes to take effect
        let window_weak = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
            if let Some(window) = window_weak.upgrade() {
                let imp = window.imp();
                if let Some(dashboard_page) = imp.dashboard_page.borrow().as_ref() {
                    dashboard_page.update_network_info();
                }
            }
        });

        // Second refresh after more time for network stack to settle
        let window_weak = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(3000), move || {
            if let Some(window) = window_weak.upgrade() {
                let imp = window.imp();
                if let Some(dashboard_page) = imp.dashboard_page.borrow().as_ref() {
                    dashboard_page.update_network_info();
                }
            }
        });

        // Show result
        let skipped_count = report.skipped.len();
        let skipped = if skipped_count == 0 {
            String::new()
        } else {
            ntr(
                // TRANSLATORS: inserted as {skipped} at the end of the
                // "Profile '{name}' applied …" messages
                ", {n} skipped as unsupported or not applicable",
                ", {n} skipped as unsupported or not applicable",
                skipped_count as u32,
            )
            .replace("{n}", &skipped_count.to_string())
        };
        if !report.errors.is_empty() {
            let errors = report.errors.len();
            self.show_toast(
                &ntr(
                    "Profile '{name}' applied with {n} error{skipped}",
                    "Profile '{name}' applied with {n} errors{skipped}",
                    errors as u32,
                )
                .replace("{n}", &errors.to_string())
                .replace("{skipped}", &skipped)
                .replace("{name}", profile_name),
            );
        } else if report.applied > 0 || skipped_count > 0 {
            self.show_toast(
                &ntr(
                    "Profile '{name}' applied ({n} action{skipped})",
                    "Profile '{name}' applied ({n} actions{skipped})",
                    report.applied as u32,
                )
                .replace("{n}", &report.applied.to_string())
                .replace("{skipped}", &skipped)
                .replace("{name}", profile_name),
            );
        } else {
            self.show_toast(&tr("Profile '{name}' activated").replace("{name}", profile_name));
        }
    }

    /// Make a modified connection take effect: reapply it on the device, or
    /// re-activate it when the change cannot be reapplied.
    fn reapply_connection(iface: &str, conn_name: &str) -> std::result::Result<(), String> {
        if run_checked(
            nmcli().args(["device", "reapply", iface]),
            "nmcli device reapply",
        )
        .is_ok()
        {
            return Ok(());
        }
        run_checked(
            nmcli().args(["connection", "up", conn_name]),
            "nmcli connection up",
        )
        .map(|_| ())
    }

    /// Current NetworkManager state of a device ("connected", "unmanaged", ...).
    fn device_state(iface: &str) -> Option<String> {
        let output = nmcli()
            .args(["-t", "-f", "DEVICE,STATE", "device", "status"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| {
                let (device, state) = line.split_once(':')?;
                (device == iface).then(|| state.to_string())
            })
    }

    /// Apply a network action using nmcli.
    ///
    /// # Privilege model
    ///
    /// These calls invoke `nmcli` directly from the GUI process. NetworkManager
    /// itself enforces Polkit authorisation, so most operations will prompt for
    /// credentials when required. However, this means the Polkit policy shipped
    /// with this application (com.chrisdaggas.network-manager.policy) only
    /// governs the D-Bus daemon — the GUI is **not** mediated by our own D-Bus
    /// service. A future release should route privileged operations through the
    /// daemon so that a single Polkit policy covers both paths.
    fn apply_network_action(action: &crate::models::NetworkAction) -> ActionResult {
        use crate::models::{Ipv4Method, NetworkAction};

        let no_connection = |what: &str, iface: &str| {
            Ok(ActionOutcome::Skipped(format!(
                "{} for {}: no NetworkManager connection",
                what, iface
            )))
        };

        match action {
            NetworkAction::Ipv4Config {
                interface,
                method,
                addresses,
                gateway,
            } => {
                let iface = interface.as_deref().unwrap_or("eth0");

                // Find connection name for this interface
                let Some(conn_name) = Self::get_connection_for_interface(iface) else {
                    return no_connection("IPv4", iface);
                };

                // Every method also resets what the others leave behind, so DHCP
                // does not keep an old static address or gateway
                let (method_name, address_list, gateway_str) = match method {
                    Ipv4Method::Auto => ("auto", String::new(), String::new()),
                    Ipv4Method::Manual => {
                        if addresses.is_empty() {
                            return Err(format!(
                                "IPv4 for {}: static configuration has no address",
                                iface
                            ));
                        }
                        let list = addresses
                            .iter()
                            .map(|a| format!("{}/{}", a.address, a.prefix))
                            .collect::<Vec<_>>()
                            .join(",");
                        (
                            "manual",
                            list,
                            gateway.map(|g| g.to_string()).unwrap_or_default(),
                        )
                    }
                    Ipv4Method::LinkLocal => ("link-local", String::new(), String::new()),
                    Ipv4Method::Disabled => ("disabled", String::new(), String::new()),
                };

                run_checked(
                    nmcli().args([
                        "connection",
                        "modify",
                        &conn_name,
                        "ipv4.method",
                        method_name,
                        "ipv4.addresses",
                        &address_list,
                        "ipv4.gateway",
                        &gateway_str,
                    ]),
                    "nmcli connection modify",
                )?;
                Self::reapply_connection(iface, &conn_name)?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::DnsServers { interface, servers } => {
                let iface = interface.as_deref().unwrap_or("eth0");

                // Only configure DNS if there's an actual NetworkManager connection
                let Some(conn_name) = Self::get_connection_for_interface(iface) else {
                    return no_connection("DNS", iface);
                };

                // An empty list clears the manual servers so DHCP-provided DNS is used
                let (v4, v6): (Vec<_>, Vec<_>) = servers.iter().partition(|s| s.is_ipv4());
                let join = |list: &[&std::net::IpAddr]| {
                    list.iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let (v4, v6) = (join(&v4), join(&v6));
                let mut args = vec![
                    "connection",
                    "modify",
                    conn_name.as_str(),
                    "ipv4.dns",
                    v4.as_str(),
                ];
                if !v6.is_empty() {
                    args.extend(["ipv6.dns", v6.as_str()]);
                }
                run_checked(nmcli().args(&args), "nmcli connection modify")?;
                Self::reapply_connection(iface, &conn_name)?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::WifiConnect { ssid, interface: _ } => {
                run_checked(
                    nmcli().args(["connection", "up", ssid]),
                    "WiFi connection failed",
                )?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::VpnConnect { connection_name } => {
                run_checked(
                    nmcli().args(["connection", "up", connection_name]),
                    "VPN connection failed",
                )?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::InterfaceEnable(state) => {
                // Leave devices already in the wanted state alone, so enabling
                // does not re-activate a working connection
                match Self::device_state(&state.interface).as_deref() {
                    Some("unmanaged") => Ok(ActionOutcome::Skipped(format!(
                        "{}: not managed by NetworkManager",
                        state.interface
                    ))),
                    Some(s) if state.enabled && s.starts_with("connected") => {
                        Ok(ActionOutcome::Applied)
                    }
                    Some("disconnected") | Some("unavailable") if !state.enabled => {
                        Ok(ActionOutcome::Applied)
                    }
                    _ => {
                        let action = if state.enabled {
                            "connect"
                        } else {
                            "disconnect"
                        };
                        run_checked(
                            nmcli().args(["device", action, &state.interface]),
                            &format!("nmcli device {}", action),
                        )?;
                        Ok(ActionOutcome::Applied)
                    }
                }
            }

            NetworkAction::VpnDisconnect { connection_name } => {
                run_checked(
                    nmcli().args(["connection", "down", connection_name]),
                    "VPN disconnect failed",
                )?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::SetMtu { interface, mtu } => {
                let Some(conn_name) = Self::get_connection_for_interface(interface) else {
                    return no_connection("MTU", interface);
                };
                run_checked(
                    nmcli().args([
                        "connection",
                        "modify",
                        &conn_name,
                        "802-3-ethernet.mtu",
                        &mtu.to_string(),
                    ]),
                    "nmcli connection modify",
                )?;
                Self::reapply_connection(interface, &conn_name)?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::SetMacAddress {
                interface,
                mac_address,
            } => {
                let Some(conn_name) = Self::get_connection_for_interface(interface) else {
                    return no_connection("MAC address", interface);
                };
                run_checked(
                    nmcli().args([
                        "connection",
                        "modify",
                        &conn_name,
                        "802-3-ethernet.cloned-mac-address",
                        mac_address,
                    ]),
                    "nmcli connection modify",
                )?;
                Self::reapply_connection(interface, &conn_name)?;
                Ok(ActionOutcome::Applied)
            }

            NetworkAction::DnsSearchDomains { interface, domains } => {
                let iface = interface.as_deref().unwrap_or("eth0");
                let Some(conn_name) = Self::get_connection_for_interface(iface) else {
                    return no_connection("DNS search domains", iface);
                };
                run_checked(
                    nmcli().args([
                        "connection",
                        "modify",
                        &conn_name,
                        "ipv4.dns-search",
                        &domains.join(","),
                    ]),
                    "nmcli connection modify",
                )?;
                Self::reapply_connection(iface, &conn_name)?;
                Ok(ActionOutcome::Applied)
            }

            // These require more complex handling or are less common
            NetworkAction::Ipv6Config { .. } => Ok(ActionOutcome::Skipped(
                "IPv6 configuration is not supported yet".to_string(),
            )),
            NetworkAction::StaticRoutes { .. } => Ok(ActionOutcome::Skipped(
                "Static routes are not supported yet".to_string(),
            )),
            NetworkAction::VlanConfig { .. } => Ok(ActionOutcome::Skipped(
                "VLAN configuration is not supported yet".to_string(),
            )),
        }
    }

    /// Split a proxy written as "[scheme://]host:port[/]" into host and port.
    fn parse_proxy_address(proxy: &str) -> Option<(String, u16)> {
        let proxy = proxy.trim();
        let rest = proxy
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(proxy);
        let (host, port) = rest.trim_end_matches('/').rsplit_once(':')?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return None;
        }
        Some((host.to_string(), port.parse().ok()?))
    }

    /// Apply a system action
    fn apply_system_action(action: &crate::models::SystemAction) -> ActionResult {
        use crate::models::{ProxyMode, SystemAction};
        use std::process::Command;

        match action {
            SystemAction::ProxyConfig(config) => {
                // Verify the gsettings schema exists (may not on non-GNOME desktops)
                if run_checked(
                    Command::new("gsettings").args(["list-keys", "org.gnome.system.proxy"]),
                    "gsettings",
                )
                .is_err()
                {
                    return Err(
                        "Proxy settings require GNOME — org.gnome.system.proxy schema not found."
                            .to_string(),
                    );
                }
                let set = |schema: &str, key: &str, value: &str| {
                    run_checked(
                        Command::new("gsettings").args(["set", schema, key, value]),
                        "gsettings set",
                    )
                    .map(|_| ())
                };

                // Set GNOME proxy settings via gsettings
                match config.mode {
                    ProxyMode::None => set("org.gnome.system.proxy", "mode", "none")?,
                    ProxyMode::Manual => {
                        // Check every address before changing anything
                        let proxies = [
                            ("org.gnome.system.proxy.http", &config.http_proxy),
                            ("org.gnome.system.proxy.https", &config.https_proxy),
                            ("org.gnome.system.proxy.ftp", &config.ftp_proxy),
                            ("org.gnome.system.proxy.socks", &config.socks_proxy),
                        ]
                        .into_iter()
                        .filter_map(|(schema, value)| {
                            value
                                .as_deref()
                                .filter(|v| !v.trim().is_empty())
                                .map(|v| (schema, v))
                        })
                        .map(|(schema, value)| {
                            Self::parse_proxy_address(value)
                                .map(|(host, port)| (schema, host, port))
                                .ok_or_else(|| {
                                    format!("Invalid proxy '{}': expected host:port", value)
                                })
                        })
                        .collect::<std::result::Result<Vec<_>, String>>()?;

                        set("org.gnome.system.proxy", "mode", "manual")?;
                        for (schema, host, port) in proxies {
                            set(schema, "host", &host)?;
                            set(schema, "port", &port.to_string())?;
                        }
                        if !config.no_proxy.is_empty() {
                            let hosts = config.no_proxy.to_variant().print(false).to_string();
                            set("org.gnome.system.proxy", "ignore-hosts", &hosts)?;
                        }
                    }
                    ProxyMode::Auto => {
                        set("org.gnome.system.proxy", "mode", "auto")?;
                        if let Some(pac) = &config.pac_url {
                            set("org.gnome.system.proxy", "autoconfig-url", pac)?;
                        }
                    }
                }
                Ok(ActionOutcome::Applied)
            }

            SystemAction::SetHostname {
                hostname,
                pretty_hostname,
            } => {
                // Set static hostname via hostnamectl
                run_checked(
                    Command::new("hostnamectl").args(["set-hostname", hostname]),
                    "Hostname change failed",
                )?;

                // Set pretty hostname if provided
                if let Some(pretty) = pretty_hostname {
                    run_checked(
                        Command::new("hostnamectl").args(["set-hostname", "--pretty", pretty]),
                        "Pretty hostname change failed",
                    )?;
                }
                Ok(ActionOutcome::Applied)
            }

            SystemAction::SetTimezone { timezone } => {
                run_checked(
                    Command::new("timedatectl").args(["set-timezone", timezone]),
                    "Timezone change failed",
                )?;
                Ok(ActionOutcome::Applied)
            }

            SystemAction::DefaultPrinter { printer_name } => {
                run_checked(
                    Command::new("lpoptions").args(["-d", printer_name]),
                    "Printer change failed",
                )?;
                Ok(ActionOutcome::Applied)
            }

            SystemAction::EnvironmentVariables { variables } => {
                // TODO: Implement safe .profile modification.
                // The previous code used unescaped format!("export {}=\"{}\"", key, value)
                // which is vulnerable to shell injection. If implementing this feature,
                // use the `shlex` crate to properly escape keys and values, or write
                // to a structured env file instead of shell syntax.
                Ok(ActionOutcome::Skipped(format!(
                    "Environment variables ({}) are not supported yet",
                    variables.len()
                )))
            }

            // These require root privileges and are more complex
            SystemAction::HostsEntries { .. } => Ok(ActionOutcome::Skipped(
                "Hosts entries are not supported yet".to_string(),
            )),
            SystemAction::FirewallConfig(_) => Ok(ActionOutcome::Skipped(
                "Firewall configuration is not supported yet".to_string(),
            )),
        }
    }

    /// Run an automation action
    fn run_automation_action(
        action: &crate::models::AutomationAction,
        sandbox_mode: crate::models::SandboxMode,
    ) -> ActionResult {
        use crate::models::AutomationAction;
        use crate::services::SandboxRunner;
        use std::process::Command;

        match action {
            AutomationAction::PreScript {
                path,
                args,
                env,
                working_dir,
                ..
            }
            | AutomationAction::PostScript {
                path,
                args,
                env,
                working_dir,
                ..
            } => {
                // Use sandbox runner if sandboxing is enabled
                if sandbox_mode != crate::models::SandboxMode::None {
                    // The sandbox runner does not take environment variables or a
                    // working directory, so those settings are not applied here
                    if !env.is_empty() || working_dir.is_some() {
                        tracing::warn!(
                            "Sandboxed script {:?}: environment and working directory are ignored",
                            path
                        );
                    }
                    let path_str = path.to_string_lossy().to_string();
                    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    let output = SandboxRunner::new(sandbox_mode)
                        .execute(&path_str, &args_refs)
                        .map_err(|e| format!("Sandbox execution failed: {:?}", e))?;
                    if !output.status.success() {
                        return Err(format!(
                            "Script failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                } else {
                    // Direct execution without sandboxing
                    let mut cmd = Command::new(path);
                    cmd.args(args).envs(env);
                    if let Some(wd) = working_dir {
                        cmd.current_dir(wd);
                    }
                    run_checked(&mut cmd, "Script failed")?;
                }
                Ok(ActionOutcome::Applied)
            }

            AutomationAction::RunProgram {
                program,
                args,
                env,
                working_dir,
                ..
            } => {
                // NOTE: RunProgram intentionally spawns without sandboxing.
                // The user explicitly configures the program path; sandboxing
                // would break legitimate program launches (e.g. GUI apps).
                let mut cmd = Command::new(program);
                cmd.args(args).envs(env);
                if let Some(wd) = working_dir {
                    cmd.current_dir(wd);
                }

                cmd.spawn()
                    .map_err(|e| format!("Failed to launch program: {}", e))?;
                Ok(ActionOutcome::Applied)
            }

            other => Ok(ActionOutcome::Skipped(format!(
                "{} is not supported yet",
                other.name()
            ))),
        }
    }

    fn create_main_menu_popover(&self) -> gtk::Popover {
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");

        let main_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .width_request(280)
            .build();

        // Theme selector section
        let theme_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(18)
            .halign(gtk::Align::Center)
            .margin_top(18)
            .margin_bottom(18)
            .build();

        // Create theme toggle buttons
        let default_btn = gtk::ToggleButton::new();
        let light_btn = gtk::ToggleButton::new();
        let dark_btn = gtk::ToggleButton::new();

        // Helper to create theme button content
        fn create_theme_content(css_class: &str, is_selected: bool) -> gtk::Overlay {
            let overlay = gtk::Overlay::new();

            let icon = gtk::Box::builder()
                .width_request(44)
                .height_request(44)
                .build();
            icon.add_css_class("theme-selector");
            icon.add_css_class(css_class);
            overlay.set_child(Some(&icon));

            if is_selected {
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.add_css_class("theme-check");
                check.set_halign(gtk::Align::Center);
                check.set_valign(gtk::Align::Center);
                overlay.add_overlay(&check);
            }

            overlay
        }

        // Set initial content
        default_btn.set_child(Some(&create_theme_content("theme-default", false)));
        // TRANSLATORS: theme button: follow the system style
        default_btn.set_tooltip_text(Some(&tr("System")));
        default_btn.add_css_class("flat");
        default_btn.add_css_class("circular");
        default_btn.add_css_class("theme-button");

        light_btn.set_child(Some(&create_theme_content("theme-light", false)));
        // TRANSLATORS: theme button: light style
        light_btn.set_tooltip_text(Some(&tr("Light")));
        light_btn.add_css_class("flat");
        light_btn.add_css_class("circular");
        light_btn.add_css_class("theme-button");

        dark_btn.set_child(Some(&create_theme_content("theme-dark", false)));
        // TRANSLATORS: theme button: dark style
        dark_btn.set_tooltip_text(Some(&tr("Dark")));
        dark_btn.add_css_class("flat");
        dark_btn.add_css_class("circular");
        dark_btn.add_css_class("theme-button");

        // Group the toggle buttons (radio-button behavior)
        light_btn.set_group(Some(&default_btn));
        dark_btn.set_group(Some(&default_btn));

        // Set initial state based on current theme
        let style_manager = adw::StyleManager::default();

        match style_manager.color_scheme() {
            adw::ColorScheme::ForceLight => {
                light_btn.set_active(true);
                light_btn.set_child(Some(&create_theme_content("theme-light", true)));
            }
            adw::ColorScheme::ForceDark => {
                dark_btn.set_active(true);
                dark_btn.set_child(Some(&create_theme_content("theme-dark", true)));
            }
            _ => {
                default_btn.set_active(true);
                default_btn.set_child(Some(&create_theme_content("theme-default", true)));
            }
        }

        // Connect theme button signals
        let light_btn_clone = light_btn.clone();
        let dark_btn_clone = dark_btn.clone();
        let window_weak = self.downgrade();
        default_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(window) = window_weak.upgrade() {
                    window.set_theme_preference(crate::models::ThemePreference::System);
                }
                btn.set_child(Some(&create_theme_content("theme-default", true)));
                light_btn_clone.set_child(Some(&create_theme_content("theme-light", false)));
                dark_btn_clone.set_child(Some(&create_theme_content("theme-dark", false)));
            }
        });

        let default_btn_clone = default_btn.clone();
        let dark_btn_clone2 = dark_btn.clone();
        let window_weak = self.downgrade();
        light_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(window) = window_weak.upgrade() {
                    window.set_theme_preference(crate::models::ThemePreference::Light);
                }
                btn.set_child(Some(&create_theme_content("theme-light", true)));
                default_btn_clone.set_child(Some(&create_theme_content("theme-default", false)));
                dark_btn_clone2.set_child(Some(&create_theme_content("theme-dark", false)));
            }
        });

        let default_btn_clone2 = default_btn.clone();
        let light_btn_clone2 = light_btn.clone();
        let window_weak = self.downgrade();
        dark_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                if let Some(window) = window_weak.upgrade() {
                    window.set_theme_preference(crate::models::ThemePreference::Dark);
                }
                btn.set_child(Some(&create_theme_content("theme-dark", true)));
                default_btn_clone2.set_child(Some(&create_theme_content("theme-default", false)));
                light_btn_clone2.set_child(Some(&create_theme_content("theme-light", false)));
            }
        });

        theme_box.append(&default_btn);
        theme_box.append(&light_btn);
        theme_box.append(&dark_btn);
        main_box.append(&theme_box);

        // Separator
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_margin_start(12);
        separator.set_margin_end(12);
        main_box.append(&separator);

        // Menu items
        let menu_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu_list.set_margin_top(6);
        menu_list.set_margin_bottom(6);
        menu_list.set_margin_start(6);
        menu_list.set_margin_end(6);

        // Preferences button
        let prefs_btn = gtk::Button::new();
        let prefs_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        prefs_box.set_margin_start(6);
        prefs_box.set_margin_end(6);
        prefs_box.set_margin_top(8);
        prefs_box.set_margin_bottom(8);
        let prefs_icon = gtk::Image::from_icon_name("emblem-system-symbolic");
        let prefs_label = gtk::Label::new(Some(&tr("Preferences")));
        prefs_label.set_halign(gtk::Align::Start);
        prefs_label.set_hexpand(true);
        prefs_box.append(&prefs_icon);
        prefs_box.append(&prefs_label);
        prefs_btn.set_child(Some(&prefs_box));
        prefs_btn.add_css_class("flat");
        prefs_btn.add_css_class("menu-item");
        prefs_btn.set_action_name(Some("app.preferences"));
        menu_list.append(&prefs_btn);

        // About button
        let about_btn = gtk::Button::new();
        let about_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        about_box.set_margin_start(6);
        about_box.set_margin_end(6);
        about_box.set_margin_top(8);
        about_box.set_margin_bottom(8);
        let about_icon = gtk::Image::from_icon_name("help-about-symbolic");
        let about_label = gtk::Label::new(Some(&tr("About")));
        about_label.set_halign(gtk::Align::Start);
        about_label.set_hexpand(true);
        about_box.append(&about_icon);
        about_box.append(&about_label);
        about_btn.set_child(Some(&about_box));
        about_btn.add_css_class("flat");
        about_btn.add_css_class("menu-item");
        about_btn.set_action_name(Some("app.about"));
        menu_list.append(&about_btn);

        // Quit button
        let quit_btn = gtk::Button::new();
        let quit_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        quit_box.set_margin_start(6);
        quit_box.set_margin_end(6);
        quit_box.set_margin_top(8);
        quit_box.set_margin_bottom(8);
        let quit_icon = gtk::Image::from_icon_name("application-exit-symbolic");
        let quit_label = gtk::Label::new(Some(&tr("Quit")));
        quit_label.set_halign(gtk::Align::Start);
        quit_label.set_hexpand(true);
        quit_box.append(&quit_icon);
        quit_box.append(&quit_label);
        quit_btn.set_child(Some(&quit_box));
        quit_btn.add_css_class("flat");
        quit_btn.add_css_class("menu-item");
        quit_btn.set_action_name(Some("app.quit"));
        menu_list.append(&quit_btn);

        main_box.append(&menu_list);

        // A plain popover does not close by itself when one of its buttons
        // runs an action, so close it before the dialog or window opens
        for button in [&prefs_btn, &about_btn, &quit_btn] {
            let popover_weak = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(popover) = popover_weak.upgrade() {
                    popover.popdown();
                }
            });
        }

        popover.set_child(Some(&main_box));
        popover
    }

    /// Apply a color scheme chosen in the main menu and remember it.
    fn set_theme_preference(&self, theme: crate::models::ThemePreference) {
        if let Some(app) = self.application().and_downcast::<Application>() {
            app.apply_theme(theme);
            let mut config = app.config();
            if config.theme != theme {
                config.theme = theme;
                app.update_config(config);
            }
        }
    }

    fn create_nav_row_with_label(
        &self,
        nav_item: NavItem,
    ) -> (gtk::ListBoxRow, gtk::Label, gtk::Box) {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(true);
        row.set_tooltip_text(Some(&nav_item.title()));

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(16);
        hbox.set_margin_bottom(16);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);
        hbox.add_css_class("nav-row-box");

        let icon = gtk::Image::from_icon_name(nav_item.icon_name());
        icon.set_pixel_size(20);
        hbox.append(&icon);

        let label = gtk::Label::new(Some(&nav_item.title()));
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        label.add_css_class("nav-label");
        hbox.append(&label);

        row.set_child(Some(&hbox));
        (row, label, hbox)
    }

    fn navigate_to(&self, nav_item: NavItem) {
        let imp = self.imp();

        if let Some(stack) = imp.content_stack.borrow().as_ref() {
            let page_name = match nav_item {
                NavItem::Dashboard => "dashboard",
                NavItem::Profiles => "profiles",
                NavItem::Scanner => "scanner",
                NavItem::Logs => "logs",
                NavItem::Settings => "settings",
                NavItem::Help => "help",
            };
            stack.set_visible_child_name(page_name);
        }

        if let Some(title) = imp.content_title.borrow().as_ref() {
            title.set_title(&nav_item.title());
            // The dashboard keeps the header title-free, matching the
            // reference shell; other pages name themselves in the header.
            title.set_visible(nav_item != NavItem::Dashboard);
        }

        imp.current_nav.set(Some(nav_item));

        // Keep the sidebar highlight in step when navigating by shortcut or code.
        // Selecting an already selected row does not emit row-selected again.
        let list = imp.sidebar_list.borrow().clone();
        if let (Some(list), Some(index)) =
            (list, NavItem::all().iter().position(|n| *n == nav_item))
        {
            if let Some(row) = list.row_at_index(index as i32) {
                if !row.is_selected() {
                    list.select_row(Some(&row));
                }
            }
        }
    }

    /// Initialize with data store.
    pub fn init_with_store(&self, store: Arc<DataStore>) {
        let imp = self.imp();

        // Store reference to data store
        *imp.data_store.borrow_mut() = Some(store.clone());

        // Load profiles from cache. When the profiles are encrypted but the
        // passphrase is not unlocked yet, ask for it first. This also covers
        // an encrypted cache whose setting was lost (e.g. reset settings).
        if store.encryption_locked() {
            self.prompt_startup_unlock(store.clone());
        } else {
            store.load_profiles_cache();
            self.apply_cached_profiles();
        }

        // Initialize logs page with data store
        if let Some(logs_page) = imp.logs_page.borrow().as_ref() {
            logs_page.init_with_store(store.clone());
        }

        // Log application startup
        store.append_log("INFO", "Application started");

        // Check for updates on startup
        self.check_for_updates();
    }

    /// Push the profiles currently held by the data store into the UI.
    fn apply_cached_profiles(&self) {
        let imp = self.imp();
        let Some(store) = imp.data_store.borrow().clone() else {
            return;
        };
        let cached_profiles = store.profiles();
        *imp.profiles.borrow_mut() = cached_profiles.clone();

        if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
            profiles_page.update_profiles(cached_profiles.clone());
        }
        if let Some(settings_page) = imp.settings_page.borrow().as_ref() {
            settings_page.set_profiles(cached_profiles.clone());
        }

        let active = cached_profiles
            .iter()
            .find(|p| p.status == crate::models::profile::ProfileStatus::Active);

        if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
            if let Some(active) = active {
                profiles_page.set_active_profile(Some(&active.id().to_string()));
            }
        }

        if let Some(dashboard) = imp.dashboard_page.borrow().as_ref() {
            if let Some(active) = active {
                let applied_time = active.metadata.last_applied_at.map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                });
                dashboard.update_active_profile(Some(active.name()), applied_time.as_deref());
            } else {
                dashboard.update_active_profile(None, None);
            }
        }
    }

    /// Ask for the encryption passphrase on startup.
    fn prompt_startup_unlock(&self, store: Arc<DataStore>) {
        let window = self.clone();
        crate::ui::encryption_dialog::show_passphrase_dialog(
            Some(self.upcast_ref::<gtk::Window>()),
            &tr("Unlock Profiles"),
            &tr("Profile encryption is enabled. Enter your passphrase to load your profiles."),
            false,
            &tr("Unlock"),
            move |passphrase| {
                let app = window
                    .application()
                    .and_downcast::<Application>()
                    .ok_or_else(|| tr("Could not access the application."))?;
                let mut config = app.config();

                if !store
                    .verify_encryption_passphrase(passphrase, config.encryption_check.as_deref())
                {
                    return Err(tr("That passphrase is not correct."));
                }

                config.encryption_key = Some(passphrase.to_string());
                // Keep the profiles encrypted if the setting itself was lost
                if store.has_encrypted_cache() {
                    config.encrypt_profiles = true;
                }
                app.update_config(config.clone());

                store.reload_profiles_cache();
                window.apply_cached_profiles();

                if let Some(settings_page) = window.imp().settings_page.borrow().as_ref() {
                    settings_page.load_from_config(&config);
                }
                Ok(())
            },
        );
    }

    /// Run the one-time GitHub release check in the background.
    fn check_for_updates(&self) {
        let obj_weak = self.downgrade();
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Spawn the HTTP request on the Tokio runtime
        crate::application::tokio_runtime().spawn(async move {
            let result = crate::version_check::check_for_update(crate::VERSION).await;
            let _ = tx.send(result);
        });

        // Receive the result on the GTK main thread
        glib::spawn_future_local(async move {
            if let Ok(Some(update_info)) = rx.await {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.show_update_available(&update_info);
                }
            }
        });
    }

    /// Display the update banner with a clickable link.
    fn show_update_available(&self, info: &crate::version_check::UpdateInfo) {
        let imp = self.imp();
        if let Some(ref banner) = *imp.update_banner.borrow() {
            // Clear placeholder children and rebuild with real info
            while let Some(child) = banner.first_child() {
                banner.remove(&child);
            }

            let icon = gtk::Image::from_icon_name("software-update-available-symbolic");
            icon.set_pixel_size(14);
            icon.add_css_class("update-icon");
            banner.append(&icon);

            let label_text = tr("v{version} available").replace("{version}", &info.latest_version);
            let link = gtk::LinkButton::with_label(&info.download_url, &label_text);
            link.add_css_class("update-link");
            banner.append(&link);

            banner.set_visible(true);
        }
    }

    /// Save profiles to the data store cache
    fn save_profiles_to_cache(&self) {
        let imp = self.imp();
        if let Some(store) = imp.data_store.borrow().as_ref() {
            if store.encryption_locked() {
                self.show_toast(&tr(
                    "Profile changes were not saved. Unlock encryption in Settings first.",
                ));
                return;
            }
            let profiles = imp.profiles.borrow().clone();
            store.update_profiles_cache(profiles.clone());
            if let Some(settings_page) = imp.settings_page.borrow().as_ref() {
                settings_page.set_profiles(profiles);
            }
            if let Some(app) = self
                .application()
                .and_downcast::<crate::application::Application>()
            {
                app.refresh_tray();
            }
        }
    }

    /// Reload profiles from cache file and update UI
    pub fn reload_profiles_from_cache(&self) {
        let imp = self.imp();
        let store = imp.data_store.borrow().clone();
        if let Some(store) = store {
            // Reload from disk
            store.reload_profiles_cache();
            self.apply_cached_profiles();
            tracing::info!(
                "Reloaded {} profiles from cache",
                imp.profiles.borrow().len()
            );
        }
    }

    /// Show a toast notification.
    pub fn show_toast(&self, message: &str) {
        let imp = self.imp();
        if let Some(toast_overlay) = imp.toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            // Messages include profile names, which are not Pango markup
            toast.set_use_markup(false);
            toast.set_timeout(3);
            toast_overlay.add_toast(toast);
        }
    }

    /// Show a toast with an action button.
    pub fn show_toast_with_action(&self, message: &str, action_label: &str, action_name: &str) {
        let imp = self.imp();
        if let Some(toast_overlay) = imp.toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_use_markup(false);
            toast.set_button_label(Some(action_label));
            toast.set_action_name(Some(action_name));
            toast.set_timeout(5);
            toast_overlay.add_toast(toast);
        }
    }

    /// Duplicate an existing profile.
    fn duplicate_profile(&self, profile_id: &str) {
        let imp = self.imp();

        let duplicated_opt = {
            let profiles = imp.profiles.borrow();
            profiles
                .iter()
                .find(|p| p.id().to_string() == profile_id)
                .map(|original| {
                    let mut duplicated = original.clone();
                    // Generate new ID and update name
                    duplicated.metadata.id = uuid::Uuid::new_v4();
                    // TRANSLATORS: name of a duplicated profile
                    let copy_name = tr("{name} (Copy)").replace("{name}", original.name());
                    duplicated.metadata.name = copy_name;
                    duplicated.metadata.created_at = chrono::Utc::now();
                    duplicated.metadata.updated_at = chrono::Utc::now();
                    duplicated.status = crate::models::profile::ProfileStatus::Inactive;
                    duplicated
                })
        };

        if let Some(duplicated) = duplicated_opt {
            let name = duplicated.metadata.name.clone();

            // Add to profiles list
            imp.profiles.borrow_mut().push(duplicated);

            // Update UI
            if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
                profiles_page.update_profiles(imp.profiles.borrow().clone());
            }

            // Save to cache
            self.save_profiles_to_cache();

            self.show_toast(&tr("Profile '{name}' created").replace("{name}", &name));
        }
    }

    /// Copy network information to clipboard.
    fn copy_network_info_to_clipboard(&self) {
        use std::process::Command;

        // Gather network info
        let mut info = String::new();
        info.push_str(&format!("=== {} ===\n\n", tr("Network Information")));

        // Get IP addresses
        if let Ok(output) = Command::new("ip")
            .env("LC_ALL", "C")
            .args(["-4", "addr", "show"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.contains("inet ") {
                        info.push_str(&format!("{}\n", line.trim()));
                    }
                }
            }
        }

        info.push('\n');

        // Get default route
        if let Ok(output) = Command::new("ip")
            .env("LC_ALL", "C")
            .args(["route", "show", "default"])
            .output()
        {
            if output.status.success() {
                info.push_str(&format!("{}\n", tr("Default Route:")));
                info.push_str(&String::from_utf8_lossy(&output.stdout));
            }
        }

        info.push('\n');

        // Get DNS servers
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            info.push_str(&format!("{}\n", tr("DNS Servers:")));
            for line in content.lines() {
                if line.starts_with("nameserver ") {
                    info.push_str(&format!("  {}\n", line));
                }
            }
        }

        // Copy to clipboard using GTK clipboard
        if let Some(display) = gtk::gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text(&info);
            self.show_toast(&tr("Network info copied to clipboard"));
        }
    }

    /// Show network diagnostics dialog.
    fn show_network_diagnostics_dialog(&self) {
        use std::process::Command;

        let dialog = dialog_window::new(&tr("Network Diagnostics"), 700, 600);

        let toolbar_view = adw::ToolbarView::new();

        let header = adw::HeaderBar::new();
        toolbar_view.add_top_bar(&header);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);

        // Ping test section
        let ping_group = adw::PreferencesGroup::new();
        ping_group.set_title(&tr("Connectivity Test"));
        ping_group.set_description(Some(&tr("Test network connectivity to common servers")));

        let ping_entry = adw::EntryRow::new();
        ping_entry.set_title(&tr("Host to ping"));
        ping_entry.set_text("8.8.8.8");
        ping_group.add(&ping_entry);

        let ping_btn = gtk::Button::with_label(&tr("Run Ping Test"));
        ping_btn.add_css_class("suggested-action");
        ping_btn.set_margin_top(8);

        let ping_result = gtk::TextView::new();
        ping_result.set_editable(false);
        ping_result.set_monospace(true);
        ping_result.set_wrap_mode(gtk::WrapMode::WordChar);
        ping_result.add_css_class("log-view");

        let ping_scroll = gtk::ScrolledWindow::new();
        ping_scroll.set_min_content_height(120);
        ping_scroll.set_child(Some(&ping_result));
        ping_scroll.set_margin_top(8);

        let ping_entry_weak = ping_entry.downgrade();
        let ping_result_weak = ping_result.downgrade();
        ping_btn.connect_clicked(move |_| {
            if let (Some(entry), Some(result)) = (ping_entry_weak.upgrade(), ping_result_weak.upgrade()) {
                let host = entry.text().to_string();
                let buffer = result.buffer();

                // Validate hostname/IP: only allow alphanumeric, dots, hyphens, colons (IPv6)
                let is_valid = !host.is_empty()
                    && host.len() <= 253
                    && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':');

                if !is_valid {
                    buffer.set_text(&tr("Invalid host. Only alphanumeric characters, dots, hyphens, and colons are allowed."));
                    return;
                }

                buffer.set_text(&format!("{}\n", tr("Running ping test...")));
                
                // Run ping in background using a channel to send results back
                let (tx, rx) = std::sync::mpsc::channel::<String>();
                
                std::thread::spawn(move || {
                    let output = Command::new("ping")
                        .args(["-c", "4", "--", &host])
                        .output();
                    
                    let text = match output {
                        Ok(out) => {
                            if out.status.success() {
                                String::from_utf8_lossy(&out.stdout).to_string()
                            } else {
                                format!("{}\n{}", tr("Ping failed:"), String::from_utf8_lossy(&out.stderr))
                            }
                        }
                        Err(e) => tr("Error running ping: {error}").replace("{error}", &e.to_string()),
                    };
                    let _ = tx.send(text);
                });
                
                // Poll for result on main thread  
                let buffer_weak = buffer.downgrade();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(text) => {
                            if let Some(buffer) = buffer_weak.upgrade() {
                                buffer.set_text(&text);
                            }
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            }
        });

        content.append(&ping_group);
        content.append(&ping_btn);
        content.append(&ping_scroll);

        // DNS lookup section
        let dns_group = adw::PreferencesGroup::new();
        dns_group.set_title(&tr("DNS Lookup"));
        dns_group.set_description(Some(&tr("Resolve domain names")));
        dns_group.set_margin_top(16);

        let dns_entry = adw::EntryRow::new();
        dns_entry.set_title(&tr("Domain to lookup"));
        dns_entry.set_text("google.com");
        dns_group.add(&dns_entry);

        let dns_btn = gtk::Button::with_label(&tr("Lookup DNS"));
        dns_btn.add_css_class("suggested-action");
        dns_btn.set_margin_top(8);

        let dns_result = gtk::TextView::new();
        dns_result.set_editable(false);
        dns_result.set_monospace(true);
        dns_result.set_wrap_mode(gtk::WrapMode::WordChar);
        dns_result.add_css_class("log-view");

        let dns_scroll = gtk::ScrolledWindow::new();
        dns_scroll.set_min_content_height(80);
        dns_scroll.set_child(Some(&dns_result));
        dns_scroll.set_margin_top(8);

        let dns_entry_weak = dns_entry.downgrade();
        let dns_result_weak = dns_result.downgrade();
        dns_btn.connect_clicked(move |btn| {
            if let (Some(entry), Some(result)) = (dns_entry_weak.upgrade(), dns_result_weak.upgrade()) {
                let domain = entry.text().to_string();
                let buffer = result.buffer();

                // Validate domain: only allow alphanumeric, dots, hyphens
                let is_valid = !domain.is_empty()
                    && domain.len() <= 253
                    && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');

                if !is_valid {
                    buffer.set_text(&tr("Invalid domain. Only alphanumeric characters, dots, and hyphens are allowed."));
                    return;
                }

                buffer.set_text(&tr("Looking up…"));
                btn.set_sensitive(false);

                // dig can wait several seconds per server, so run it off the GTK thread
                let buffer_weak = buffer.downgrade();
                let btn_weak = btn.downgrade();
                glib::spawn_future_local(async move {
                    let text = gio::spawn_blocking(move || {
                        if let Ok(output) = Command::new("dig").args(["+short", "--", &domain]).output() {
                            if output.status.success() {
                                let text = String::from_utf8_lossy(&output.stdout).to_string();
                                if text.is_empty() {
                                    tr("No DNS records found")
                                } else {
                                    text
                                }
                            } else {
                                format!("{}\n{}", tr("DNS lookup failed:"), String::from_utf8_lossy(&output.stderr))
                            }
                        } else {
                            // Fallback to host command
                            match Command::new("host").arg(&domain).output() {
                                Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
                                Err(_) => tr("DNS lookup tools (dig, host) not available"),
                            }
                        }
                    }).await.unwrap_or_else(|_| tr("DNS lookup failed"));

                    if let Some(buffer) = buffer_weak.upgrade() {
                        buffer.set_text(&text);
                    }
                    if let Some(btn) = btn_weak.upgrade() {
                        btn.set_sensitive(true);
                    }
                });
            }
        });

        content.append(&dns_group);
        content.append(&dns_btn);
        content.append(&dns_scroll);

        // Quick diagnostics
        let quick_group = adw::PreferencesGroup::new();
        quick_group.set_title(&tr("Quick Test Results"));
        quick_group.set_margin_top(16);

        let internet_row = adw::ActionRow::builder()
            .title(tr("Internet Connectivity"))
            .subtitle(tr("Checking…"))
            .build();
        quick_group.add(&internet_row);

        let dns_row = adw::ActionRow::builder()
            .title(tr("DNS Resolution"))
            .subtitle(tr("Checking…"))
            .build();
        quick_group.add(&dns_row);

        content.append(&quick_group);

        // Run quick tests off the GTK thread; each can take seconds when offline
        let internet_row_weak = internet_row.downgrade();
        let dns_row_weak = dns_row.downgrade();
        glib::spawn_future_local(async move {
            let (internet_ok, dns_ok) = gio::spawn_blocking(|| {
                let succeeded =
                    |cmd: &mut Command| cmd.output().map(|o| o.status.success()).unwrap_or(false);
                (
                    succeeded(Command::new("ping").args(["-c", "1", "-W", "2", "8.8.8.8"])),
                    succeeded(Command::new("host").args(["-W", "2", "google.com"])),
                )
            })
            .await
            .unwrap_or((false, false));

            if let Some(row) = internet_row_weak.upgrade() {
                row.set_subtitle(&if internet_ok {
                    // TRANSLATORS: internet connectivity test result
                    format!("✓ {}", tr("Connected"))
                } else {
                    // TRANSLATORS: internet connectivity test result
                    format!("✗ {}", tr("No connection"))
                });
            }
            if let Some(row) = dns_row_weak.upgrade() {
                row.set_subtitle(&if dns_ok {
                    // TRANSLATORS: DNS resolution test result
                    format!("✓ {}", tr("DNS Working"))
                } else {
                    // TRANSLATORS: DNS resolution test result
                    format!("✗ {}", tr("DNS Failed"))
                });
            }
        });

        // Speed Test section
        let speed_group = adw::PreferencesGroup::new();
        speed_group.set_title(&tr("Speed Test"));
        speed_group.set_description(Some(&tr(
            "Measure download/upload speed (uses speedtest-cli if available)",
        )));
        speed_group.set_margin_top(16);

        let speed_btn = gtk::Button::with_label(&tr("Run Speed Test"));
        speed_btn.add_css_class("suggested-action");

        let speed_result = gtk::TextView::new();
        speed_result.set_editable(false);
        speed_result.set_monospace(true);
        speed_result.set_wrap_mode(gtk::WrapMode::WordChar);
        speed_result.add_css_class("log-view");

        let speed_scroll = gtk::ScrolledWindow::new();
        speed_scroll.set_min_content_height(100);
        speed_scroll.set_child(Some(&speed_result));
        speed_scroll.set_margin_top(8);

        let speed_result_weak = speed_result.downgrade();
        let speed_btn_weak = speed_btn.downgrade();
        speed_btn.connect_clicked(move |_| {
            if let (Some(result), Some(btn)) =
                (speed_result_weak.upgrade(), speed_btn_weak.upgrade())
            {
                let buffer = result.buffer();
                buffer.set_text(&tr("Running speed test... (this may take a minute)"));
                btn.set_sensitive(false);

                let (tx, rx) = std::sync::mpsc::channel::<String>();

                std::thread::spawn(move || {
                    // Try speedtest-cli first
                    let output = Command::new("speedtest-cli").arg("--simple").output();

                    let text = match output {
                        Ok(out) if out.status.success() => {
                            String::from_utf8_lossy(&out.stdout).to_string()
                        }
                        _ => {
                            // Fallback: simple download test using curl
                            let download_result = Command::new("curl")
                                .args([
                                    "-o",
                                    "/dev/null",
                                    "-w",
                                    "%{speed_download}",
                                    "-s",
                                    "https://speed.cloudflare.com/__down?bytes=10000000",
                                ])
                                .output();

                            match download_result {
                                Ok(out) if out.status.success() => {
                                    let speed_str = String::from_utf8_lossy(&out.stdout);
                                    if let Ok(speed) = speed_str.trim().parse::<f64>() {
                                        let speed_mbps = speed * 8.0 / 1_000_000.0;
                                        format!(
                                            "{}\n{}",
                                            tr("Download: {speed} Mbps")
                                                .replace("{speed}", &format!("{:.2}", speed_mbps)),
                                            tr("(speedtest-cli not installed for full test)")
                                        )
                                    } else {
                                        tr("Could not parse speed result")
                                    }
                                }
                                _ => format!(
                                    "{}\n{}",
                                    tr("Speed test tools not available."),
                                    tr("Install speedtest-cli: {command}")
                                        .replace("{command}", "pip install speedtest-cli")
                                ),
                            }
                        }
                    };
                    let _ = tx.send(text);
                });

                // Poll for result
                let buffer_weak = buffer.downgrade();
                let btn_weak2 = btn.downgrade();
                glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                    match rx.try_recv() {
                        Ok(text) => {
                            if let Some(buffer) = buffer_weak.upgrade() {
                                buffer.set_text(&text);
                            }
                            if let Some(btn) = btn_weak2.upgrade() {
                                btn.set_sensitive(true);
                            }
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            if let Some(btn) = btn_weak2.upgrade() {
                                btn.set_sensitive(true);
                            }
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
        });

        content.append(&speed_group);
        content.append(&speed_btn);
        content.append(&speed_scroll);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&content));

        toolbar_view.set_content(Some(&scroll));
        dialog.set_content(Some(&toolbar_view));

        dialog_window::present(&dialog, Some(self));
    }

    /// Refresh the current view.
    fn refresh_current_view(&self) {
        let imp = self.imp();

        if let Some(nav_item) = imp.current_nav.get() {
            match nav_item {
                NavItem::Dashboard => {
                    if let Some(dashboard) = imp.dashboard_page.borrow().as_ref() {
                        dashboard.update_network_info();
                    }
                }
                NavItem::Profiles => {
                    self.reload_profiles_from_cache();
                }
                NavItem::Scanner => {
                    if let Some(scanner) = imp.scanner_page.borrow().as_ref() {
                        scanner.refresh();
                    }
                }
                NavItem::Logs => {
                    if let Some(logs_page) = imp.logs_page.borrow().as_ref() {
                        logs_page.refresh_logs();
                    }
                }
                _ => {}
            }
        }

        self.show_toast(&tr("Refreshed"));
    }

    /// Show export profiles dialog.
    fn show_export_profiles_dialog(&self) {
        let imp = self.imp();
        let profiles = imp.profiles.borrow().clone();

        if profiles.is_empty() {
            self.show_toast(&tr("No profiles to export"));
            return;
        }

        let dialog = gtk::FileDialog::builder()
            .title(tr("Export Profiles"))
            .initial_name("network-profiles.json")
            .build();

        let window_weak = self.downgrade();
        dialog.save(Some(self), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if let Some(window) = window_weak.upgrade() {
                        window.export_profiles_to_file(&path);
                    }
                }
            }
        });
    }

    /// Export profiles to a file.
    fn export_profiles_to_file(&self, path: &std::path::Path) {
        let imp = self.imp();
        let profiles = imp.profiles.borrow().clone();

        match serde_json::to_string_pretty(&profiles) {
            Ok(json) => match std::fs::write(path, json) {
                Ok(_) => {
                    let n = profiles.len();
                    self.show_toast(
                        &ntr("Exported {n} profile", "Exported {n} profiles", n as u32)
                            .replace("{n}", &n.to_string()),
                    );
                }
                Err(e) => {
                    tracing::error!("Export failed: {}", e);
                    self.show_toast(&tr("Export failed. Check logs for details."));
                }
            },
            Err(e) => {
                tracing::error!("Export failed: {}", e);
                self.show_toast(&tr("Export failed. Check logs for details."));
            }
        }
    }

    /// Show import profiles dialog.
    fn show_import_profiles_dialog(&self) {
        let json_filter = gtk::FileFilter::new();
        json_filter.set_name(Some(&tr("JSON Files")));
        json_filter.add_pattern("*.json");

        let all_filter = gtk::FileFilter::new();
        all_filter.set_name(Some(&tr("All Files")));
        all_filter.add_pattern("*");

        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&json_filter);
        filters.append(&all_filter);

        let dialog = gtk::FileDialog::builder()
            .title(tr("Import Profiles"))
            .filters(&filters)
            .build();

        let window_weak = self.downgrade();
        dialog.open(Some(self), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if let Some(window) = window_weak.upgrade() {
                        window.import_profiles_from_file(&path);
                    }
                }
            }
        });
    }

    /// Import profiles from a file.
    fn import_profiles_from_file(&self, path: &std::path::Path) {
        let json = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to read import file: {}", e);
                self.show_toast(&tr("Failed to read file. Check logs for details."));
                return;
            }
        };

        // Accept the plain list written by Export as well as the
        // `{ "profiles": [...] }` backups made by older versions
        let imported_profiles: Vec<Profile> = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                let wrapped = serde_json::from_str::<serde_json::Value>(&json)
                    .ok()
                    .and_then(|mut v| v.get_mut("profiles").map(serde_json::Value::take))
                    .and_then(|v| serde_json::from_value::<Vec<Profile>>(v).ok());
                match wrapped {
                    Some(p) => p,
                    None => {
                        self.show_toast(
                            &tr("Invalid profile format: {error}")
                                .replace("{error}", &e.to_string()),
                        );
                        return;
                    }
                }
            }
        };

        if imported_profiles.is_empty() {
            self.show_toast(&tr("No profiles found in file"));
            return;
        }

        // Check if any imported profiles contain automation actions
        let has_automation = imported_profiles.iter().any(|p| profile_has_automation(p));

        if has_automation {
            let dialog = adw::AlertDialog::builder()
                .heading(tr("Import Warning"))
                .body(tr("The imported file contains profiles with executable scripts or programs.\n\nDo you want to import them with or without these automation actions?"))
                .build();

            dialog.add_response("cancel", &tr("Cancel"));
            dialog.add_response("without-scripts", &tr("Import Without Scripts"));
            dialog.add_response("with-scripts", &tr("Import With Scripts"));
            dialog.set_response_appearance("without-scripts", adw::ResponseAppearance::Suggested);
            dialog.set_response_appearance("with-scripts", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("without-scripts"));
            dialog.set_close_response("cancel");

            let window_weak = self.downgrade();
            dialog.connect_response(None, move |_, response| {
                if response == "without-scripts" || response == "with-scripts" {
                    if let Some(window) = window_weak.upgrade() {
                        let strip_scripts = response == "without-scripts";
                        window.finish_import_profiles(imported_profiles.clone(), strip_scripts);
                    }
                }
            });

            dialog.present(Some(self));
        } else {
            self.finish_import_profiles(imported_profiles, false);
        }
    }

    /// Complete the profile import after any user prompts.
    fn finish_import_profiles(&self, imported_profiles: Vec<Profile>, strip_scripts: bool) {
        let imp = self.imp();

        let mut new_profiles: Vec<Profile> = imported_profiles
            .into_iter()
            .map(|mut p| {
                p.metadata.id = uuid::Uuid::new_v4();
                // TRANSLATORS: name of an imported profile
                p.metadata.name = tr("{name} (Imported)").replace("{name}", &p.metadata.name);
                p.metadata.created_at = chrono::Utc::now();
                p.metadata.updated_at = chrono::Utc::now();
                p.status = crate::models::profile::ProfileStatus::Inactive;
                // Trust is never imported: scripts from a file always get the safety prompt
                p.approved_for_users = false;
                if strip_scripts {
                    p.automation_actions.clear();
                }
                p
            })
            .collect();

        let count = new_profiles.len();

        imp.profiles.borrow_mut().append(&mut new_profiles);

        if let Some(profiles_page) = imp.profiles_page.borrow().as_ref() {
            profiles_page.update_profiles(imp.profiles.borrow().clone());
        }

        self.save_profiles_to_cache();

        if strip_scripts {
            self.show_toast(
                &ntr(
                    "Imported {n} profile without automation actions",
                    "Imported {n} profiles without automation actions",
                    count as u32,
                )
                .replace("{n}", &count.to_string()),
            );
        } else {
            self.show_toast(
                &ntr(
                    "Imported {n} profile",
                    "Imported {n} profiles",
                    count as u32,
                )
                .replace("{n}", &count.to_string()),
            );
        }
    }
}
