// Network Manager - Scanner Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Network scanner page.
//!
//! Sweeps a subnet or IP range, then lists every address with its hostname,
//! MAC address, vendor, reply time, and open services. Inspired by the classic
//! LAN scanners, adapted to the application's flat libadwaita surfaces.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::services::scanner::{self, HostResult, RangeError, ScanEvent, ScanMode};
use crate::tr;
use crate::ui::dialog_window;

const COL_STATUS: i32 = 12;
const COL_ACTIONS: i32 = 58;
const CELL_SPACING: i32 = 7;
const DATA_COLUMN_COUNT: usize = 6;
const MAX_COLUMN_WIDTH: i32 = 480;
const RESIZE_HANDLE_WIDTH: i32 = 8;

/// A data column whose width can be dragged by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataColumn {
    Ip,
    Name,
    Mac,
    Vendor,
    Ping,
    Services,
}

impl DataColumn {
    const ALL: [Self; DATA_COLUMN_COUNT] = [
        Self::Ip,
        Self::Name,
        Self::Mac,
        Self::Vendor,
        Self::Ping,
        Self::Services,
    ];

    fn index(self) -> usize {
        match self {
            Self::Ip => 0,
            Self::Name => 1,
            Self::Mac => 2,
            Self::Vendor => 3,
            Self::Ping => 4,
            Self::Services => 5,
        }
    }

    fn default_width(self) -> i32 {
        match self {
            Self::Ip => 105,
            Self::Name => 120,
            Self::Mac => 110,
            Self::Vendor => 120,
            Self::Ping => 48,
            Self::Services => 96,
        }
    }

    fn min_width(self) -> i32 {
        match self {
            Self::Ip => 80,
            Self::Name => 60,
            Self::Mac => 90,
            Self::Vendor => 60,
            Self::Ping => 40,
            Self::Services => 50,
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Ip => Some(Self::Name),
            Self::Name => Some(Self::Mac),
            Self::Mac => Some(Self::Vendor),
            Self::Vendor => Some(Self::Ping),
            Self::Ping => Some(Self::Services),
            Self::Services => None,
        }
    }

    fn title(self) -> String {
        match self {
            Self::Ip => tr("IP Address"),
            Self::Name => tr("Device Name"),
            Self::Mac => tr("MAC Address"),
            Self::Vendor => tr("Vendor"),
            Self::Ping => tr("Ping"),
            Self::Services => tr("Services"),
        }
    }
}

/// Widgets that make up one result row, kept for in-place updates.
#[derive(Clone)]
pub struct RowWidgets {
    dot: gtk::Box,
    ip: gtk::Label,
    hostname: gtk::Label,
    mac: gtk::Label,
    vendor: gtk::Label,
    latency: gtk::Label,
    services: gtk::Label,
    wake: gtk::Button,
}

impl RowWidgets {
    fn label(&self, column: DataColumn) -> &gtk::Label {
        match column {
            DataColumn::Ip => &self.ip,
            DataColumn::Name => &self.hostname,
            DataColumn::Mac => &self.mac,
            DataColumn::Vendor => &self.vendor,
            DataColumn::Ping => &self.latency,
            DataColumn::Services => &self.services,
        }
    }
}

/// A label sized like its column header.
fn column_label(text: &str, width: i32, expand: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_size_request(width, -1);
    label.set_hexpand(expand);
    // Keep long hostnames and vendors from widening the whole table; the
    // label ellipsizes and the full value lives in its tooltip.
    label.set_width_chars(1);
    label.set_max_width_chars(1);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label
}

fn placeholder_label(width: i32, expand: bool, css: Option<&str>) -> gtk::Label {
    let label = column_label("—", width, expand);
    label.add_css_class("dim-label");
    if let Some(css) = css {
        label.add_css_class(css);
    }
    label
}

fn header_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_width_chars(1);
    label.set_max_width_chars(1);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label
}

fn set_text_or_dash(label: &gtk::Label, value: Option<&str>) {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => {
            label.set_text(value);
            label.remove_css_class("dim-label");
            label.set_tooltip_text(Some(value));
        }
        None => {
            label.set_text("—");
            label.add_css_class("dim-label");
            label.set_tooltip_text(None);
        }
    }
}

fn format_latency(latency: Option<f64>) -> String {
    match latency {
        Some(ms) if ms < 0.05 => "< 0.1 ms".to_string(),
        Some(ms) if ms < 10.0 => format!("{:.1} ms", ms),
        Some(ms) => format!("{:.0} ms", ms),
        None => "—".to_string(),
    }
}

/// Plain-text summary of a device, used by the copy actions.
fn device_summary(result: Option<&HostResult>, ip: Ipv4Addr, status: &str) -> String {
    let unknown = tr("Unknown");
    let hostname = result
        .and_then(|result| result.hostname.as_deref())
        .unwrap_or(&unknown);
    let mac = result
        .and_then(|result| result.mac.as_deref())
        .unwrap_or(&unknown);
    let vendor = result
        .and_then(|result| result.vendor.as_deref())
        .unwrap_or(&unknown);
    let ping = result
        .and_then(|result| result.latency_ms)
        .map(|latency| format_latency(Some(latency)))
        .unwrap_or_else(|| {
            if result.is_some_and(|result| result.online) {
                tr("No ICMP reply")
            } else {
                tr("No reply")
            }
        });
    let services = result
        .and_then(|result| services_tooltip(&result.open_ports))
        .unwrap_or_else(|| tr("None"));

    [
        format!("{}: {}", tr("IP Address"), ip),
        format!("{}: {}", tr("Hostname"), hostname),
        format!("{}: {}", tr("MAC Address"), mac),
        format!("{}: {}", tr("Vendor"), vendor),
        format!("{}: {}", tr("Status"), status),
        format!("{}: {}", tr("Ping"), ping),
        format!("{}: {}", tr("Services"), services),
    ]
    .join("\n")
}

/// Human-readable protocol names for the service ports we probe.
fn service_name(port: u16) -> &'static str {
    match port {
        22 => "SSH",
        53 => "DNS",
        80 => "HTTP",
        443 => "HTTPS",
        445 => "SMB",
        631 => "IPP",
        3306 => "MySQL",
        3389 => "RDP",
        5432 => "PostgreSQL",
        8080 => "HTTP",
        8123 => "Home Assistant",
        9090 => "Cockpit",
        32400 => "Plex",
        _ => "",
    }
}

fn services_text(ports: &[u16]) -> String {
    if ports.is_empty() {
        return "—".to_string();
    }
    ports
        .iter()
        .map(|&port| service_name(port))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn services_tooltip(ports: &[u16]) -> Option<String> {
    if ports.is_empty() {
        return None;
    }
    Some(
        ports
            .iter()
            .map(|&port| format!("{} ({})", port, service_name(port)))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ScannerPage {
        pub range_entry: RefCell<Option<gtk::Entry>>,
        pub mode_dropdown: RefCell<Option<gtk::DropDown>>,
        pub scan_button: RefCell<Option<gtk::Button>>,
        pub cancel_button: RefCell<Option<gtk::Button>>,
        pub export_button: RefCell<Option<gtk::Button>>,
        pub progress_row: RefCell<Option<gtk::Box>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub error_label: RefCell<Option<gtk::Label>>,
        pub count_label: RefCell<Option<gtk::Label>>,
        pub filter_dropdown: RefCell<Option<gtk::DropDown>>,
        pub sort_dropdown: RefCell<Option<gtk::DropDown>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub list: RefCell<Option<gtk::ListBox>>,
        pub column_widths: RefCell<[i32; DATA_COLUMN_COUNT]>,
        pub header_cells: RefCell<Vec<gtk::Box>>,

        /// Results by address; `None` while the address is still being scanned.
        pub data: Rc<RefCell<HashMap<Ipv4Addr, Option<HostResult>>>>,
        pub rows: RefCell<HashMap<Ipv4Addr, RowWidgets>>,
        /// Target order, used for CSV export.
        pub order: RefCell<Vec<Ipv4Addr>>,

        pub cancel_flag: RefCell<Option<Arc<AtomicBool>>>,
        pub generation: Cell<u64>,
        pub scanning: Cell<bool>,
        pub cancelled: Cell<bool>,
        pub scanned: Cell<usize>,
        pub total: Cell<usize>,
        pub online: Cell<usize>,
        pub scan_started: Cell<Option<Instant>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ScannerPage {
        const NAME: &'static str = "CdNetworkManagerScannerPage";
        type Type = super::ScannerPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for ScannerPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for ScannerPage {}
    impl BoxImpl for ScannerPage {}
}

glib::wrapper! {
    pub struct ScannerPage(ObjectSubclass<imp::ScannerPage>)
        @extends gtk::Widget, gtk::Box;
}

impl ScannerPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Column widths are kept per page and can be changed from the header.
        {
            let mut widths = imp.column_widths.borrow_mut();
            for (index, column) in DataColumn::ALL.iter().enumerate() {
                widths[index] = column.default_width();
            }
        }

        // Page header: title and a quiet export action.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.set_margin_top(24);
        header.set_margin_bottom(12);
        header.set_margin_start(24);
        header.set_margin_end(24);

        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        title_box.set_hexpand(true);

        let title = gtk::Label::new(Some(&tr("Network Scanner")));
        title.add_css_class("title-2");
        title.set_halign(gtk::Align::Start);
        title_box.append(&title);

        let subtitle = gtk::Label::new(Some(&tr("Discover devices on your local network")));
        subtitle.add_css_class("caption");
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(gtk::Align::Start);
        title_box.append(&subtitle);

        header.append(&title_box);

        let export_button = gtk::Button::from_icon_name("document-save-symbolic");
        export_button.add_css_class("flat");
        export_button.set_tooltip_text(Some(&tr("Export Results")));
        export_button.set_valign(gtk::Align::Center);
        export_button.set_sensitive(false);
        header.append(&export_button);

        self.append(&header);

        // Scan controls: range, mode, and the actions that drive the scan.
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.add_css_class("scanner-controls");
        controls.set_margin_start(12);
        controls.set_margin_end(12);
        controls.set_margin_bottom(12);

        let control_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let network_icon = gtk::Image::from_icon_name("network-wired-symbolic");
        network_icon.add_css_class("accent");
        network_icon.set_valign(gtk::Align::Center);
        control_row.append(&network_icon);

        let range_entry = gtk::Entry::new();
        range_entry.set_hexpand(true);
        range_entry.set_valign(gtk::Align::Center);
        range_entry.set_placeholder_text(Some("192.168.1.0/24"));
        range_entry.set_tooltip_text(Some(&tr(
            "Examples: 192.168.1.0/24, 192.168.1.1-254, or 192.168.1.20",
        )));
        let detected =
            scanner::detect_local_range().unwrap_or_else(|| "192.168.1.0/24".to_string());
        range_entry.set_text(&detected);
        control_row.append(&range_entry);

        // Re-detect the local subnet after a network change.
        let detect_button = gtk::Button::from_icon_name("view-refresh-symbolic");
        detect_button.add_css_class("flat");
        detect_button.set_tooltip_text(Some(&tr("Detect local network")));
        detect_button.set_valign(gtk::Align::Center);
        control_row.append(&detect_button);

        let mode_labels = [tr("Fast"), tr("Thorough")];
        let mode_refs: Vec<&str> = mode_labels.iter().map(String::as_str).collect();
        let mode_dropdown = gtk::DropDown::from_strings(&mode_refs);
        mode_dropdown.set_selected(0);
        mode_dropdown.set_valign(gtk::Align::Center);
        mode_dropdown.set_size_request(120, -1);
        mode_dropdown.set_tooltip_text(Some(&tr(
            "Fast uses ping; thorough also probes common service ports",
        )));
        control_row.append(&mode_dropdown);

        let scan_button = gtk::Button::with_label(&tr("Scan"));
        scan_button.add_css_class("suggested-action");
        scan_button.set_icon_name("system-search-symbolic");
        scan_button.set_valign(gtk::Align::Center);
        control_row.append(&scan_button);

        let cancel_button = gtk::Button::with_label(&tr("Stop"));
        cancel_button.set_icon_name("process-stop-symbolic");
        cancel_button.set_valign(gtk::Align::Center);
        cancel_button.set_visible(false);
        control_row.append(&cancel_button);

        controls.append(&control_row);

        let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);

        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_hexpand(true);
        progress_bar.set_valign(gtk::Align::Center);
        progress_bar.set_show_text(false);
        progress_bar.set_visible(false);
        progress_row.append(&progress_bar);

        let status_label = gtk::Label::new(Some(&tr("Ready to scan")));
        status_label.add_css_class("caption");
        status_label.add_css_class("dim-label");
        status_label.set_xalign(0.0);
        status_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status_label.set_tooltip_text(Some(&tr("Scan status")));
        progress_row.append(&status_label);

        controls.append(&progress_row);

        let error_label = gtk::Label::new(None);
        error_label.add_css_class("scanner-error");
        error_label.set_xalign(0.0);
        error_label.set_wrap(true);
        error_label.set_visible(false);
        controls.append(&error_label);

        self.append(&controls);

        // Results vs. empty state.
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(150);

        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("network-wired-symbolic"));
        empty_state.set_title(&tr("Scan Your Network"));
        empty_state.set_description(Some(&tr(
            "Find computers, phones, routers, and other devices on your local \
             network. Enter a subnet or IP range to begin.",
        )));

        let empty_scan_button = gtk::Button::with_label(&tr("Start Scan"));
        empty_scan_button.add_css_class("suggested-action");
        empty_scan_button.add_css_class("pill");
        empty_scan_button.set_halign(gtk::Align::Center);
        empty_state.set_child(Some(&empty_scan_button));

        stack.add_named(&empty_state, Some("empty"));

        let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let results_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        results_header.set_margin_start(12);
        results_header.set_margin_end(12);
        results_header.set_margin_bottom(8);

        let count_label = gtk::Label::new(Some(&tr("No scan yet")));
        count_label.add_css_class("caption");
        count_label.add_css_class("dim-label");
        count_label.set_xalign(0.0);
        count_label.set_hexpand(true);
        count_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        results_header.append(&count_label);

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some(&tr("Search devices")));
        search_entry.set_width_chars(18);
        search_entry.set_tooltip_text(Some(&tr("Search by IP, name, MAC, or vendor")));
        results_header.append(&search_entry);

        let filter_labels = [tr("All"), tr("Online"), tr("Offline")];
        let filter_refs: Vec<&str> = filter_labels.iter().map(String::as_str).collect();
        let filter_dropdown = gtk::DropDown::from_strings(&filter_refs);
        filter_dropdown.set_selected(0);
        filter_dropdown.set_tooltip_text(Some(&tr("Filter devices by status")));
        results_header.append(&filter_dropdown);

        let sort_labels = [tr("IP"), tr("Name"), tr("Status")];
        let sort_refs: Vec<&str> = sort_labels.iter().map(String::as_str).collect();
        let sort_dropdown = gtk::DropDown::from_strings(&sort_refs);
        sort_dropdown.set_selected(0);
        sort_dropdown.set_tooltip_text(Some(&tr("Sort devices")));
        results_header.append(&sort_dropdown);

        results_box.append(&results_header);

        let columns = gtk::Box::new(gtk::Orientation::Horizontal, CELL_SPACING);
        columns.add_css_class("scanner-column-header");
        columns.set_margin_start(12);
        columns.set_margin_end(12);

        let status_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_header.set_size_request(COL_STATUS, -1);
        columns.append(&status_header);

        let mut header_cells = Vec::with_capacity(DATA_COLUMN_COUNT);
        for column in DataColumn::ALL {
            let cell = self.create_column_header(column);
            if column == DataColumn::Services {
                cell.set_tooltip_text(Some(&tr(
                    "Services are detected by ports during a thorough scan",
                )));
            }
            header_cells.push(cell.clone());
            columns.append(&cell);
        }

        let actions_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions_spacer.set_size_request(COL_ACTIONS, -1);
        columns.append(&actions_spacer);

        // Everything stays left-aligned; leftover width is empty space.
        let trailing = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        trailing.set_hexpand(true);
        columns.append(&trailing);

        results_box.append(&columns);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_vexpand(true);
        // Never scroll sideways: the columns must stay aligned with the
        // header above the list, so long values ellipsize instead.
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_margin_start(12);
        scroller.set_margin_end(12);
        scroller.set_margin_bottom(12);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_activate_on_single_click(true);
        list.add_css_class("scanner-list");

        let page_weak = self.downgrade();
        list.connect_row_activated(move |_, row| {
            if let Ok(ip) = row.widget_name().parse::<Ipv4Addr>() {
                if let Some(page) = page_weak.upgrade() {
                    page.show_device_details(ip);
                }
            }
        });

        // Filter by the live search text and the status selector.
        let filter_data = imp.data.clone();
        let filter_weak = filter_dropdown.downgrade();
        let search_weak = search_entry.downgrade();
        list.set_filter_func(move |row| {
            let Ok(ip) = row.widget_name().parse::<Ipv4Addr>() else {
                return true;
            };
            let data = filter_data.borrow();
            let Some(slot) = data.get(&ip) else {
                return true;
            };

            let filter = filter_weak
                .upgrade()
                .map(|dropdown| dropdown.selected())
                .unwrap_or(0);
            let status_matches = match filter {
                1 => slot.as_ref().is_some_and(|result| result.online),
                2 => slot.as_ref().is_some_and(|result| !result.online),
                _ => true,
            };
            if !status_matches {
                return false;
            }

            let query = search_weak
                .upgrade()
                .map(|entry| entry.text().to_lowercase())
                .unwrap_or_default();
            if query.is_empty() {
                return true;
            }
            if ip.to_string().contains(&query) {
                return true;
            }
            slot.as_ref().is_some_and(|result| {
                result
                    .hostname
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&query))
                    || result
                        .mac
                        .as_deref()
                        .is_some_and(|mac| mac.to_lowercase().contains(&query))
                    || result
                        .vendor
                        .as_deref()
                        .is_some_and(|vendor| vendor.to_lowercase().contains(&query))
            })
        });

        // IP is the natural order; name and status are handy on busy networks.
        let sort_data = imp.data.clone();
        let sort_weak = sort_dropdown.downgrade();
        list.set_sort_func(move |a, b| {
            let Ok(ip_a) = a.widget_name().parse::<Ipv4Addr>() else {
                return gtk::Ordering::Equal;
            };
            let Ok(ip_b) = b.widget_name().parse::<Ipv4Addr>() else {
                return gtk::Ordering::Equal;
            };
            let mode = sort_weak
                .upgrade()
                .map(|dropdown| dropdown.selected())
                .unwrap_or(0);

            if mode == 1 || mode == 2 {
                let data = sort_data.borrow();
                let result_a = data.get(&ip_a).and_then(Option::as_ref);
                let result_b = data.get(&ip_b).and_then(Option::as_ref);
                if mode == 1 {
                    let name_a = result_a
                        .and_then(|result| result.hostname.as_deref())
                        .unwrap_or_default()
                        .to_lowercase();
                    let name_b = result_b
                        .and_then(|result| result.hostname.as_deref())
                        .unwrap_or_default()
                        .to_lowercase();
                    match name_a.cmp(&name_b) {
                        std::cmp::Ordering::Equal => {}
                        ordering => return ordering.into(),
                    }
                } else {
                    let online_a = result_a.is_some_and(|result| result.online);
                    let online_b = result_b.is_some_and(|result| result.online);
                    match online_b.cmp(&online_a) {
                        std::cmp::Ordering::Equal => {}
                        ordering => return ordering.into(),
                    }
                }
            }

            u32::from(ip_a).cmp(&u32::from(ip_b)).into()
        });

        let list_weak = list.downgrade();
        search_entry.connect_search_changed(move |_| {
            if let Some(list) = list_weak.upgrade() {
                list.invalidate_filter();
            }
        });
        let list_weak = list.downgrade();
        filter_dropdown.connect_selected_notify(move |_| {
            if let Some(list) = list_weak.upgrade() {
                list.invalidate_filter();
            }
        });
        let list_weak = list.downgrade();
        sort_dropdown.connect_selected_notify(move |_| {
            if let Some(list) = list_weak.upgrade() {
                list.invalidate_sort();
            }
        });

        scroller.set_child(Some(&list));
        results_box.append(&scroller);
        stack.add_named(&results_box, Some("results"));
        stack.set_visible_child_name("empty");

        self.append(&stack);

        // Store references.
        *imp.range_entry.borrow_mut() = Some(range_entry.clone());
        *imp.mode_dropdown.borrow_mut() = Some(mode_dropdown.clone());
        *imp.scan_button.borrow_mut() = Some(scan_button.clone());
        *imp.cancel_button.borrow_mut() = Some(cancel_button.clone());
        *imp.export_button.borrow_mut() = Some(export_button.clone());
        *imp.progress_row.borrow_mut() = Some(progress_row);
        *imp.progress_bar.borrow_mut() = Some(progress_bar);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.error_label.borrow_mut() = Some(error_label);
        *imp.count_label.borrow_mut() = Some(count_label);
        *imp.filter_dropdown.borrow_mut() = Some(filter_dropdown);
        *imp.sort_dropdown.borrow_mut() = Some(sort_dropdown);
        *imp.stack.borrow_mut() = Some(stack);
        *imp.list.borrow_mut() = Some(list);
        *imp.header_cells.borrow_mut() = header_cells;

        // Actions.
        let page_weak = self.downgrade();
        scan_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.start_scan();
            }
        });

        let page_weak = self.downgrade();
        empty_scan_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.start_scan();
            }
        });

        let page_weak = self.downgrade();
        cancel_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.stop_scan();
            }
        });

        let page_weak = self.downgrade();
        export_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.export_results();
            }
        });

        let range_weak = range_entry.downgrade();
        detect_button.connect_clicked(move |_| {
            if let Some(range) = scanner::detect_local_range() {
                if let Some(entry) = range_weak.upgrade() {
                    entry.set_text(&range);
                }
            }
        });

        // A fresh edit hides the previous validation message.
        let page_weak = self.downgrade();
        let range_weak = range_entry.downgrade();
        range_entry.connect_changed(move |_| {
            if let Some(entry) = range_weak.upgrade() {
                entry.remove_css_class("error");
            }
            if let Some(page) = page_weak.upgrade() {
                page.clear_error();
            }
        });

        range_entry.connect_activate({
            let page_weak = self.downgrade();
            move |_| {
                if let Some(page) = page_weak.upgrade() {
                    page.start_scan();
                }
            }
        });
    }

    /// Build a header cell with a drag handle at its trailing edge.
    fn create_column_header(&self, column: DataColumn) -> gtk::Box {
        let width = self.imp().column_widths.borrow()[column.index()];
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.set_size_request(width, -1);
        cell.set_valign(gtk::Align::Center);

        let label = header_label(&column.title());
        label.set_hexpand(true);
        cell.append(&label);

        let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        handle.set_size_request(RESIZE_HANDLE_WIDTH, -1);
        handle.set_valign(gtk::Align::Fill);
        handle.set_cursor_from_name(Some("ew-resize"));
        handle.add_css_class("scanner-resize-handle");
        handle.set_tooltip_text(Some(&tr("Drag to resize column")));
        self.install_column_resize(&handle, column);
        cell.append(&handle);

        cell
    }

    fn install_column_resize(&self, handle: &gtk::Box, column: DataColumn) {
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(1);

        let start_width = Rc::new(Cell::new(0));
        let begin_width = start_width.clone();
        let page_weak = self.downgrade();
        gesture.connect_drag_begin(move |_, _, _| {
            if let Some(page) = page_weak.upgrade() {
                begin_width.set(page.imp().column_widths.borrow()[column.index()]);
            }
        });

        let page_weak = self.downgrade();
        let update_width = start_width;
        gesture.connect_drag_update(move |_, offset_x, _| {
            if let Some(page) = page_weak.upgrade() {
                let target = update_width.get() + offset_x.round() as i32;
                page.resize_column(column, target);
            }
        });

        handle.add_controller(gesture);
    }

    /// Resize a column from a header drag.
    ///
    /// Growth uses empty space first and then borrows width from the next
    /// column; shrinking hands the width to the next column. This keeps the
    /// table inside the page instead of forcing a horizontal scrollbar.
    fn resize_column(&self, column: DataColumn, target: i32) {
        let imp = self.imp();
        let mut widths = imp.column_widths.borrow_mut();
        let index = column.index();
        let current = widths[index];
        let target = target.clamp(column.min_width(), MAX_COLUMN_WIDTH);
        let mut delta = target - current;
        if delta == 0 {
            return;
        }

        let snapshot = *widths;

        if delta > 0 {
            let spare = self.column_spare(&widths[..]);
            let from_spare = delta.min(spare).min(MAX_COLUMN_WIDTH - current);
            if from_spare > 0 {
                widths[index] += from_spare;
                delta -= from_spare;
            }

            if delta > 0 {
                if let Some(next) = column.next() {
                    let next_index = next.index();
                    let take = delta
                        .min(widths[next_index] - next.min_width())
                        .min(MAX_COLUMN_WIDTH - widths[index]);
                    if take > 0 {
                        widths[index] += take;
                        widths[next_index] -= take;
                    }
                }
            }
        } else {
            let shrink = (-delta).min(current - column.min_width());
            if shrink > 0 {
                if let Some(next) = column.next() {
                    let next_index = next.index();
                    let give = shrink.min(MAX_COLUMN_WIDTH - widths[next_index]);
                    if give > 0 {
                        widths[index] -= give;
                        widths[next_index] += give;
                    }
                } else {
                    widths[index] -= shrink;
                }
            }
        }

        let new_widths = *widths;
        drop(widths);

        for column in DataColumn::ALL {
            let index = column.index();
            if new_widths[index] != snapshot[index] {
                self.apply_column_width(column, new_widths[index]);
            }
        }
    }

    /// Free horizontal space left in the table, in pixels.
    fn column_spare(&self, widths: &[i32]) -> i32 {
        let page_width = self.width();
        if page_width <= 0 {
            return 0;
        }
        // Page margins (12px each) plus the column row's inner padding.
        let available = page_width - 40;
        let used = widths.iter().sum::<i32>()
            + COL_STATUS
            + COL_ACTIONS
            + CELL_SPACING * (DATA_COLUMN_COUNT as i32 + 2);
        (available - used).max(0)
    }

    fn apply_column_width(&self, column: DataColumn, width: i32) {
        let imp = self.imp();
        if let Some(cell) = imp.header_cells.borrow().get(column.index()) {
            cell.set_size_request(width, -1);
        }
        for row in imp.rows.borrow().values() {
            row.label(column).set_size_request(width, -1);
        }
    }

    /// Start a scan using the range and mode currently in the controls.
    pub fn start_scan(&self) {
        let imp = self.imp();
        if imp.scanning.get() {
            return;
        }

        let input = imp
            .range_entry
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default();
        let targets = match scanner::parse_targets(&input) {
            Ok(targets) => targets,
            Err(error) => {
                self.show_range_error(error);
                return;
            }
        };
        self.clear_error();

        let mode = if imp
            .mode_dropdown
            .borrow()
            .as_ref()
            .is_some_and(|dropdown| dropdown.selected() == 1)
        {
            ScanMode::Thorough
        } else {
            ScanMode::Fast
        };

        imp.total.set(targets.len());
        imp.scanned.set(0);
        imp.online.set(0);
        imp.cancelled.set(false);
        imp.scanning.set(true);
        imp.scan_started.set(Some(Instant::now()));
        let generation = imp.generation.get().wrapping_add(1);
        imp.generation.set(generation);

        self.reset_results(&targets);
        self.set_scanning_ui(true);

        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_fraction(0.0);
        }
        if let Some(stack) = imp.stack.borrow().as_ref() {
            stack.set_visible_child_name("results");
        }
        if let Some(export) = imp.export_button.borrow().as_ref() {
            export.set_sensitive(false);
        }
        self.update_count_label();

        let handle = scanner::start_scan(targets, mode);
        *imp.cancel_flag.borrow_mut() = Some(handle.cancel);
        let events = handle.events;

        let page_weak = self.downgrade();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            let Some(page) = page_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.imp().generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            let mut finished = false;
            loop {
                match events.try_recv() {
                    Ok(ScanEvent::Host(result)) => page.apply_host_result(result),
                    Ok(ScanEvent::Progress { completed, total }) => {
                        page.update_progress(completed, total);
                    }
                    Ok(ScanEvent::Finished) => {
                        finished = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }

            if finished {
                page.finish_scan();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Ask the running scan to stop after the addresses in flight.
    fn stop_scan(&self) {
        let imp = self.imp();
        if !imp.scanning.get() || imp.cancelled.get() {
            return;
        }
        imp.cancelled.set(true);
        if let Some(flag) = imp.cancel_flag.borrow().as_ref() {
            flag.store(true, AtomicOrdering::Relaxed);
        }
        if let Some(label) = imp.status_label.borrow().as_ref() {
            label.set_text(&tr("Stopping…"));
        }
        if let Some(button) = imp.cancel_button.borrow().as_ref() {
            button.set_sensitive(false);
        }
    }

    /// Toggle the controls between idle and scanning.
    fn set_scanning_ui(&self, scanning: bool) {
        let imp = self.imp();
        if let Some(entry) = imp.range_entry.borrow().as_ref() {
            entry.set_sensitive(!scanning);
        }
        if let Some(dropdown) = imp.mode_dropdown.borrow().as_ref() {
            dropdown.set_sensitive(!scanning);
        }
        if let Some(button) = imp.scan_button.borrow().as_ref() {
            button.set_sensitive(!scanning);
        }
        if let Some(button) = imp.cancel_button.borrow().as_ref() {
            button.set_visible(scanning);
            button.set_sensitive(scanning);
        }
        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_visible(scanning);
        }
        if scanning {
            if let Some(status) = imp.status_label.borrow().as_ref() {
                let range = imp
                    .range_entry
                    .borrow()
                    .as_ref()
                    .map(|entry| entry.text())
                    .unwrap_or_default();
                status.set_text(&tr("Scanning {range}…").replace("{range}", &range));
            }
            if let Some(label) = imp.error_label.borrow().as_ref() {
                label.set_visible(false);
            }
        }
    }

    /// Replace the result list with fresh waiting rows for `targets`.
    fn reset_results(&self, targets: &[Ipv4Addr]) {
        let imp = self.imp();
        let Some(list) = imp.list.borrow().clone() else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        imp.data.borrow_mut().clear();
        imp.rows.borrow_mut().clear();
        imp.order.borrow_mut().clear();

        for &ip in targets {
            let (row, widgets) = self.create_result_row(ip);
            imp.data.borrow_mut().insert(ip, None);
            imp.rows.borrow_mut().insert(ip, widgets);
            imp.order.borrow_mut().push(ip);
            // Appending may run the sort/filter functions; do it after the
            // maps are complete so those callbacks never re-enter a borrow.
            list.append(&row);
        }
    }

    fn create_result_row(&self, ip: Ipv4Addr) -> (gtk::ListBoxRow, RowWidgets) {
        let widths = *self.imp().column_widths.borrow();

        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&ip.to_string());
        row.set_activatable(true);
        row.set_selectable(false);
        row.add_css_class("scanner-row");
        row.set_cursor_from_name(Some("pointer"));
        row.set_tooltip_text(Some(&tr("Click for device details")));

        let line = gtk::Box::new(gtk::Orientation::Horizontal, CELL_SPACING);
        line.set_valign(gtk::Align::Center);

        let status_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_wrap.set_size_request(COL_STATUS, -1);
        status_wrap.set_halign(gtk::Align::Start);
        status_wrap.set_valign(gtk::Align::Center);

        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.set_size_request(9, 9);
        dot.set_valign(gtk::Align::Center);
        dot.add_css_class("scanner-dot");
        dot.add_css_class("waiting");
        dot.set_tooltip_text(Some(&tr("Waiting")));
        status_wrap.append(&dot);
        line.append(&status_wrap);

        let ip_label = column_label(&ip.to_string(), widths[DataColumn::Ip.index()], false);
        ip_label.add_css_class("scanner-ip");
        line.append(&ip_label);

        let hostname = placeholder_label(widths[DataColumn::Name.index()], false, None);
        line.append(&hostname);

        let mac = placeholder_label(widths[DataColumn::Mac.index()], false, Some("scanner-mac"));
        line.append(&mac);

        let vendor = placeholder_label(widths[DataColumn::Vendor.index()], false, None);
        line.append(&vendor);

        let latency = placeholder_label(
            widths[DataColumn::Ping.index()],
            false,
            Some("scanner-numeric"),
        );
        line.append(&latency);

        let services = placeholder_label(
            widths[DataColumn::Services.index()],
            false,
            Some("scanner-services"),
        );
        line.append(&services);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.set_size_request(COL_ACTIONS, -1);
        actions.set_halign(gtk::Align::End);
        actions.set_valign(gtk::Align::Center);

        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.add_css_class("flat");
        copy.set_tooltip_text(Some(&tr("Copy IP address")));
        copy.set_valign(gtk::Align::Center);
        let page_weak = self.downgrade();
        copy.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.copy_to_clipboard(
                    &ip.to_string(),
                    &tr("IP address {ip} copied").replace("{ip}", &ip.to_string()),
                );
            }
        });
        actions.append(&copy);

        let wake = gtk::Button::from_icon_name("system-reboot-symbolic");
        wake.add_css_class("flat");
        wake.set_tooltip_text(Some(&tr("Wake on LAN")));
        wake.set_valign(gtk::Align::Center);
        wake.set_sensitive(false);
        let page_weak = self.downgrade();
        wake.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                let mac = page
                    .imp()
                    .data
                    .borrow()
                    .get(&ip)
                    .and_then(|result| result.as_ref())
                    .and_then(|result| result.mac.clone());
                if let Some(mac) = mac {
                    page.wake_host(&mac);
                }
            }
        });
        actions.append(&wake);

        line.append(&actions);

        let trailing = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        trailing.set_hexpand(true);
        line.append(&trailing);

        row.set_child(Some(&line));

        (
            row,
            RowWidgets {
                dot,
                ip: ip_label,
                hostname,
                mac,
                vendor,
                latency,
                services,
                wake,
            },
        )
    }

    fn show_device_details(&self, ip: Ipv4Addr) {
        let imp = self.imp();
        let result = imp.data.borrow().get(&ip).cloned().flatten();
        let scanning = imp.scanning.get() && result.is_none();
        let online = result.as_ref().is_some_and(|result| result.online);
        let hostname = result.as_ref().and_then(|result| result.hostname.clone());
        let display_name = hostname.clone().unwrap_or_else(|| ip.to_string());
        let status_text = if scanning {
            tr("Scanning…")
        } else if online {
            tr("Online")
        } else {
            tr("Offline")
        };

        let summary = device_summary(result.as_ref(), ip, &status_text);
        let dialog = dialog_window::new(&tr("Device Details"), 560, 600);
        let toolbar = adw::ToolbarView::new();

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        let title = adw::WindowTitle::new(&tr("Device Details"), &ip.to_string());
        header.set_title_widget(Some(&title));

        let copy_summary_button = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_summary_button.add_css_class("flat");
        copy_summary_button.set_tooltip_text(Some(&tr("Copy details")));
        let page_weak = self.downgrade();
        let summary_for_header = summary.clone();
        copy_summary_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.copy_to_clipboard(&summary_for_header, &tr("Device details copied"));
            }
        });
        header.pack_end(&copy_summary_button);
        toolbar.add_top_bar(&header);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);

        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        hero.add_css_class("scanner-detail-hero");

        let icon = gtk::Image::from_icon_name(if scanning {
            "content-loading-symbolic"
        } else if online {
            "network-transmit-receive-symbolic"
        } else {
            "network-offline-symbolic"
        });
        icon.set_pixel_size(40);
        icon.set_valign(gtk::Align::Center);
        if scanning {
            icon.add_css_class("accent");
        } else if online {
            icon.add_css_class("success");
        } else {
            icon.add_css_class("dim-label");
        }
        hero.append(&icon);

        let hero_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        hero_text.set_hexpand(true);

        let name = gtk::Label::new(Some(&display_name));
        name.add_css_class("title-2");
        name.set_halign(gtk::Align::Start);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_tooltip_text(Some(&display_name));
        hero_text.append(&name);

        let subtitle = result
            .as_ref()
            .and_then(|result| result.vendor.as_deref())
            .map(|vendor| format!("{} • {}", ip, vendor))
            .unwrap_or_else(|| ip.to_string());
        let subtitle_label = gtk::Label::new(Some(&subtitle));
        subtitle_label.add_css_class("dim-label");
        subtitle_label.set_halign(gtk::Align::Start);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subtitle_label.set_tooltip_text(Some(&subtitle));
        hero_text.append(&subtitle_label);
        hero.append(&hero_text);

        let status_pill = gtk::Label::new(Some(&status_text));
        status_pill.add_css_class("status-pill");
        if scanning {
            status_pill.add_css_class("applying");
        } else if online {
            status_pill.add_css_class("active");
        } else {
            status_pill.add_css_class("inactive");
        }
        status_pill.set_valign(gtk::Align::Center);
        hero.append(&status_pill);
        content.append(&hero);

        let network_group = adw::PreferencesGroup::new();
        network_group.set_title(&tr("Network"));
        network_group.add(&self.detail_row(&tr("IP Address"), Some(&ip.to_string()), true));
        network_group.add(&self.detail_row(&tr("Hostname"), hostname.as_deref(), true));
        network_group.add(&self.detail_row(
            &tr("MAC Address"),
            result.as_ref().and_then(|result| result.mac.as_deref()),
            true,
        ));
        network_group.add(&self.detail_row(
            &tr("Vendor"),
            result.as_ref().and_then(|result| result.vendor.as_deref()),
            false,
        ));
        content.append(&network_group);

        let scan_group = adw::PreferencesGroup::new();
        scan_group.set_title(&tr("Scan Results"));

        let status_row = adw::ActionRow::new();
        status_row.set_title(&tr("Status"));
        let status_value = gtk::Label::new(Some(&status_text));
        status_value.add_css_class("status-pill");
        if scanning {
            status_value.add_css_class("applying");
        } else if online {
            status_value.add_css_class("active");
        } else {
            status_value.add_css_class("inactive");
        }
        status_value.set_valign(gtk::Align::Center);
        status_row.add_suffix(&status_value);
        scan_group.add(&status_row);

        let ping_text = if scanning {
            tr("Waiting for reply…")
        } else if result
            .as_ref()
            .and_then(|result| result.latency_ms)
            .is_some()
        {
            format_latency(result.as_ref().and_then(|result| result.latency_ms))
        } else if online {
            tr("No ICMP reply")
        } else {
            tr("No reply")
        };
        scan_group.add(&self.detail_row(&tr("Ping"), Some(&ping_text), false));
        content.append(&scan_group);

        let services_group = adw::PreferencesGroup::new();
        services_group.set_title(&tr("Open Services"));
        let ports: &[u16] = result
            .as_ref()
            .map(|result| result.open_ports.as_slice())
            .unwrap_or(&[]);
        if ports.is_empty() {
            services_group.set_description(Some(&tr(
                "Run a thorough scan to detect services on this device",
            )));
            let row = self.detail_row(&tr("No open services detected"), None, false);
            row.add_css_class("dim-label");
            services_group.add(&row);
        } else {
            services_group.set_description(Some(&tr("Detected by port during the scan")));
            for &port in ports {
                let row = adw::ActionRow::new();
                row.set_title(service_name(port));
                row.set_subtitle(&tr("Port {port}").replace("{port}", &port.to_string()));
                let port_icon = gtk::Image::from_icon_name("network-server-symbolic");
                row.add_prefix(&port_icon);
                services_group.add(&row);
            }
        }
        content.append(&services_group);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);

        if !online {
            if let Some(mac) = result.as_ref().and_then(|result| result.mac.clone()) {
                let wake_button = gtk::Button::with_label(&tr("Wake on LAN"));
                wake_button.set_icon_name("system-reboot-symbolic");
                let page_weak = self.downgrade();
                wake_button.connect_clicked(move |_| {
                    if let Some(page) = page_weak.upgrade() {
                        page.wake_host(&mac);
                    }
                });
                buttons.append(&wake_button);
            }
        }

        let copy_button = gtk::Button::with_label(&tr("Copy Details"));
        copy_button.set_icon_name("edit-copy-symbolic");
        let page_weak = self.downgrade();
        let summary_for_button = summary.clone();
        copy_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.copy_to_clipboard(&summary_for_button, &tr("Device details copied"));
            }
        });
        buttons.append(&copy_button);
        content.append(&buttons);

        scroll.set_child(Some(&content));
        toolbar.set_content(Some(&scroll));
        dialog.set_content(Some(&toolbar));
        dialog_window::present(&dialog, Some(self));
    }

    /// A titled preferences row for the details dialog.
    fn detail_row(&self, title: &str, value: Option<&str>, copyable: bool) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(title);
        let text = value.filter(|value| !value.is_empty());
        row.set_subtitle(text.unwrap_or(&tr("Unknown")));

        if copyable {
            if let Some(text) = text {
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.add_css_class("flat");
                copy.set_tooltip_text(Some(&tr("Copy")));
                copy.set_valign(gtk::Align::Center);
                let page_weak = self.downgrade();
                let text = text.to_string();
                copy.connect_clicked(move |_| {
                    if let Some(page) = page_weak.upgrade() {
                        page.copy_to_clipboard(&text, &tr("Copied to clipboard"));
                    }
                });
                row.add_suffix(&copy);
            }
        }

        row
    }

    fn apply_host_result(&self, result: HostResult) {
        let imp = self.imp();
        let ip = result.ip;
        let online = result.online;

        {
            let mut data = imp.data.borrow_mut();
            if let Some(slot) = data.get_mut(&ip) {
                *slot = Some(result.clone());
            }
        }
        if online {
            imp.online.set(imp.online.get() + 1);
        }

        let widgets = imp.rows.borrow().get(&ip).cloned();
        if let Some(widgets) = widgets {
            widgets.dot.remove_css_class("waiting");
            widgets.dot.remove_css_class("online");
            widgets.dot.remove_css_class("offline");
            if online {
                widgets.dot.add_css_class("online");
                widgets.dot.set_tooltip_text(Some(&tr("Online")));
            } else {
                widgets.dot.add_css_class("offline");
                widgets.dot.set_tooltip_text(Some(&tr("Offline")));
            }

            set_text_or_dash(&widgets.hostname, result.hostname.as_deref());
            set_text_or_dash(&widgets.mac, result.mac.as_deref());
            set_text_or_dash(&widgets.vendor, result.vendor.as_deref());

            if result.latency_ms.is_some() {
                widgets.latency.set_text(&format_latency(result.latency_ms));
                widgets.latency.remove_css_class("dim-label");
            } else {
                widgets.latency.set_text("—");
                widgets.latency.add_css_class("dim-label");
            }

            widgets
                .services
                .set_text(&services_text(&result.open_ports));
            widgets
                .services
                .set_tooltip_text(services_tooltip(&result.open_ports).as_deref());
            if result.open_ports.is_empty() {
                widgets.services.add_css_class("dim-label");
            } else {
                widgets.services.remove_css_class("dim-label");
            }

            widgets.wake.set_sensitive(!online && result.mac.is_some());
            let wake_tooltip = if online {
                tr("Device is online")
            } else if result.mac.is_some() {
                tr("Wake on LAN")
            } else {
                tr("Wake on LAN needs a MAC address")
            };
            widgets.wake.set_tooltip_text(Some(&wake_tooltip));
        }

        if let Some(list) = imp.list.borrow().as_ref() {
            list.invalidate_filter();
            if imp
                .sort_dropdown
                .borrow()
                .as_ref()
                .is_some_and(|dropdown| dropdown.selected() != 0)
            {
                list.invalidate_sort();
            }
        }
        self.update_count_label();
    }

    fn update_progress(&self, completed: usize, total: usize) {
        let imp = self.imp();
        imp.scanned.set(completed);
        imp.total.set(total);
        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            let fraction = if total == 0 {
                0.0
            } else {
                completed as f64 / total as f64
            };
            bar.set_fraction(fraction);
        }
        if let Some(label) = imp.status_label.borrow().as_ref() {
            label.set_text(
                &tr("Scanning… {completed} of {total}")
                    .replace("{completed}", &completed.to_string())
                    .replace("{total}", &total.to_string()),
            );
        }
        self.update_count_label();
    }

    fn finish_scan(&self) {
        let imp = self.imp();
        imp.scanning.set(false);
        imp.cancel_flag.borrow_mut().take();

        let elapsed = imp
            .scan_started
            .take()
            .map(|started| started.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let scanned = imp.scanned.get();
        let total = imp.total.get();
        let online = imp.online.get();
        let cancelled = imp.cancelled.get();

        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            let fraction = if total == 0 {
                0.0
            } else {
                scanned as f64 / total as f64
            };
            bar.set_fraction(fraction);
        }
        if let Some(label) = imp.status_label.borrow().as_ref() {
            let text = if cancelled {
                tr("Scan stopped after {scanned} of {total} addresses")
                    .replace("{scanned}", &scanned.to_string())
                    .replace("{total}", &total.to_string())
            } else {
                tr("Scan complete in {seconds} s").replace("{seconds}", &format!("{:.1}", elapsed))
            };
            label.set_text(&text);
        }

        self.set_scanning_ui(false);
        if let Some(button) = imp.cancel_button.borrow().as_ref() {
            button.set_label(&tr("Stop"));
        }
        if let Some(button) = imp.export_button.borrow().as_ref() {
            button.set_sensitive(scanned > 0);
        }
        self.update_count_label();

        // TEMP DEBUG
        {
            let widths: Vec<i32> = imp
                .header_cells
                .borrow()
                .iter()
                .map(|cell| cell.width())
                .collect();
            eprintln!("SCANNER header widths: {:?}", widths);
            if let Some((_, row)) = imp.rows.borrow().iter().next() {
                eprintln!(
                    "SCANNER row widths: ip={} name={} mac={} vendor={} ping={} services={}",
                    row.ip.width(),
                    row.hostname.width(),
                    row.mac.width(),
                    row.vendor.width(),
                    row.latency.width(),
                    row.services.width()
                );
            }
            if let Some(label) = imp.status_label.borrow().as_ref() {
                eprintln!("SCANNER status label width: {}", label.width());
            }
        }

        tracing::info!(
            "Network scan finished: {} online of {} addresses in {:.1}s",
            online,
            scanned,
            elapsed
        );
    }

    fn update_count_label(&self) {
        let imp = self.imp();
        let Some(label) = imp.count_label.borrow().as_ref().cloned() else {
            return;
        };
        let total = imp.total.get();
        let scanned = imp.scanned.get();
        let online = imp.online.get();

        let text = if total == 0 {
            tr("No scan yet")
        } else if imp.scanning.get() {
            tr("{online} online • {scanned} of {total} scanned")
                .replace("{online}", &online.to_string())
                .replace("{scanned}", &scanned.to_string())
                .replace("{total}", &total.to_string())
        } else {
            tr("{online} online • {total} scanned")
                .replace("{online}", &online.to_string())
                .replace("{total}", &total.to_string())
        };
        label.set_text(&text);
    }

    fn show_range_error(&self, error: RangeError) {
        let text = match error {
            RangeError::Empty => tr("Enter an IP range or subnet to scan"),
            RangeError::Invalid => tr("Enter a valid IPv4 address, subnet (CIDR), or range"),
            RangeError::TooMany(_) => {
                tr("That range is too large. Scan at most {max} addresses at once.")
                    .replace("{max}", &scanner::MAX_TARGETS.to_string())
            }
        };
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_text(&text);
            label.set_visible(true);
        }
        if let Some(entry) = self.imp().range_entry.borrow().as_ref() {
            entry.add_css_class("error");
            entry.grab_focus();
        }
    }

    fn clear_error(&self) {
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_visible(false);
        }
    }

    /// Re-detect the local subnet while the page is idle and untouched.
    pub fn refresh(&self) {
        let imp = self.imp();
        if imp.scanning.get() || imp.total.get() > 0 {
            return;
        }
        if let Some(range) = scanner::detect_local_range() {
            if let Some(entry) = imp.range_entry.borrow().as_ref() {
                entry.set_text(&range);
            }
        }
    }

    fn copy_to_clipboard(&self, text: &str, message: &str) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
            self.show_toast(message);
        }
    }

    fn wake_host(&self, mac: &str) {
        match scanner::send_wake_on_lan(mac) {
            Ok(()) => {
                self.show_toast(&tr("Wake-on-LAN packet sent to {mac}").replace("{mac}", mac))
            }
            Err(error) => self
                .show_toast(&tr("Could not send Wake-on-LAN: {error}").replace("{error}", &error)),
        }
    }

    fn export_results(&self) {
        let imp = self.imp();
        let order = imp.order.borrow();
        let data = imp.data.borrow();

        let mut csv =
            String::from("IP Address,Status,Hostname,MAC Address,Vendor,Latency (ms),Open Ports\n");
        let mut exported = 0usize;
        for ip in order.iter() {
            let Some(Some(result)) = data.get(ip) else {
                continue;
            };
            let status = if result.online { "Online" } else { "Offline" };
            let latency = result
                .latency_ms
                .map(|ms| format!("{:.1}", ms))
                .unwrap_or_default();
            let ports = result
                .open_ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                csv_field(&result.ip.to_string()),
                csv_field(status),
                csv_field(result.hostname.as_deref().unwrap_or("")),
                csv_field(result.mac.as_deref().unwrap_or("")),
                csv_field(result.vendor.as_deref().unwrap_or("")),
                csv_field(&latency),
                csv_field(&ports),
            ));
            exported += 1;
        }
        drop(order);
        drop(data);

        if exported == 0 {
            self.show_toast(&tr("No scan results to export"));
            return;
        }

        let dialog = gtk::FileDialog::builder()
            .title(tr("Export Scan Results"))
            .initial_name(format!(
                "network-scan-{}.csv",
                chrono::Local::now().format("%Y-%m-%d-%H%M")
            ))
            .build();

        let window = self.root().and_downcast::<gtk::Window>();
        let page_weak = self.downgrade();
        dialog.save(window.as_ref(), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                return;
            };
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            match std::fs::write(&path, csv.as_bytes()) {
                Ok(()) => page.show_toast(&tr("Scan results exported")),
                Err(error) => page.show_toast(
                    &tr("Could not export scan results: {error}")
                        .replace("{error}", &error.to_string()),
                ),
            }
        });
    }

    fn show_toast(&self, message: &str) {
        if let Some(overlay) = self
            .ancestor(adw::ToastOverlay::static_type())
            .and_downcast::<adw::ToastOverlay>()
        {
            overlay.add_toast(adw::Toast::new(message));
        }
    }
}

impl Default for ScannerPage {
    fn default() -> Self {
        Self::new()
    }
}
