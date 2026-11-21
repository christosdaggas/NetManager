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

use crate::services::port_scanner;
use crate::services::scanner::{self, HostResult, RangeError, ScanEvent, ScanMode};
use crate::tr;
use crate::ui::dialog_window;

/// A text column of the results table.
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
    const ALL: [Self; 6] = [
        Self::Ip,
        Self::Name,
        Self::Mac,
        Self::Vendor,
        Self::Ping,
        Self::Services,
    ];

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

    /// Width the column asks for, in characters of its own font.
    ///
    /// Sizing by characters instead of pixels keeps full addresses visible
    /// at any font size. The value is constant per column so the table does
    /// not shift as results arrive or the list scrolls.
    fn width_chars(self) -> i32 {
        match self {
            Self::Ip => 15,
            Self::Name => 12,
            Self::Mac => 17,
            Self::Vendor => 12,
            Self::Ping => 8,
            Self::Services => 10,
        }
    }

    /// Free-text columns share the width left over by the fixed ones.
    fn expands(self) -> bool {
        matches!(self, Self::Name | Self::Vendor | Self::Services)
    }

    fn css_class(self) -> Option<&'static str> {
        match self {
            Self::Ip => Some("scanner-ip"),
            Self::Mac => Some("scanner-mac"),
            Self::Ping => Some("scanner-numeric"),
            Self::Services => Some("scanner-services"),
            Self::Name | Self::Vendor => None,
        }
    }

    /// Cell text and tooltip for a scanned address; `None` shows a dash.
    fn value(self, ip: Ipv4Addr, result: Option<&HostResult>) -> Option<(String, Option<String>)> {
        let own_tooltip = |text: &str| (text.to_string(), Some(text.to_string()));
        match self {
            // Never truncated, so the row's own tooltip can show through.
            Self::Ip => Some((ip.to_string(), None)),
            Self::Name => result?
                .hostname
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(own_tooltip),
            Self::Mac => result?
                .mac
                .as_deref()
                .filter(|mac| !mac.is_empty())
                .map(own_tooltip),
            Self::Vendor => result?
                .vendor
                .as_deref()
                .filter(|vendor| !vendor.is_empty())
                .map(own_tooltip),
            Self::Ping => result?
                .latency_ms
                .map(|latency| (format_latency(Some(latency)), None)),
            Self::Services => {
                let ports = &result?.open_ports;
                let text = services_text(ports);
                if ports.is_empty() || text.is_empty() {
                    None
                } else {
                    Some((text, services_tooltip(ports)))
                }
            }
        }
    }
}

/// The address a list item of the results table stands for.
fn item_ip(item: &glib::Object) -> Option<Ipv4Addr> {
    item.downcast_ref::<glib::BoxedAnyObject>()
        .and_then(|boxed| boxed.try_borrow::<Ipv4Addr>().ok().map(|ip| *ip))
}

/// The address shown by a table cell.
fn cell_ip(cell: &gtk::ColumnViewCell) -> Option<Ipv4Addr> {
    cell.item().as_ref().and_then(item_ip)
}

/// A button showing both an icon and a label.
pub(super) fn labelled_icon_button(icon_name: &str, label: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon_name)
        .label(label)
        .build();
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    button
}

/// Status text shown next to the progress bar during a scan.
pub(super) fn progress_text(completed: usize, total: usize) -> String {
    tr("Scanning… {completed} of {total}")
        .replace("{completed}", &completed.to_string())
        .replace("{total}", &total.to_string())
}

pub(super) fn format_latency(latency: Option<f64>) -> String {
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
    port_scanner::known_service(port).unwrap_or_default()
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

pub(super) fn csv_field(value: &str) -> String {
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
        pub stack: RefCell<Option<gtk::Stack>>,
        /// One item per scanned address, in target order.
        pub store: RefCell<Option<gio::ListStore>>,

        /// Results by address; `None` while the address is still being scanned.
        pub data: Rc<RefCell<HashMap<Ipv4Addr, Option<HostResult>>>>,
        /// Target order, used for CSV export and to find an address in `store`.
        pub order: RefCell<Vec<Ipv4Addr>>,
        pub positions: RefCell<HashMap<Ipv4Addr, u32>>,

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

        // Page header: title and a quiet export action. Every block on the
        // page shares the same 12px side margin so their edges line up.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.set_margin_top(18);
        header.set_margin_bottom(12);
        header.set_margin_start(12);
        header.set_margin_end(12);

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

        // `Button::set_icon_name` replaces a button's label, so the icon and
        // the text are set together as the button's content.
        let scan_button = labelled_icon_button("system-search-symbolic", &tr("Scan"));
        scan_button.add_css_class("suggested-action");
        scan_button.set_valign(gtk::Align::Center);
        control_row.append(&scan_button);

        let cancel_button = labelled_icon_button("process-stop-symbolic", &tr("Stop"));
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
        // Tabular digits keep the progress bar from twitching as the count
        // next to it changes.
        status_label.add_css_class("numeric");
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

        // The results table. A column view gives the header and every cell
        // of a column the same width by construction, lets the user resize
        // columns from the header, and only builds the rows on screen.
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();

        // Filter by the live search text and the status selector.
        let filter = gtk::CustomFilter::new({
            let data = imp.data.clone();
            let filter_weak = filter_dropdown.downgrade();
            let search_weak = search_entry.downgrade();
            move |item| {
                let Some(ip) = item_ip(item) else {
                    return true;
                };
                let data = data.borrow();
                let Some(slot) = data.get(&ip) else {
                    return true;
                };

                let status = filter_weak
                    .upgrade()
                    .map(|dropdown| dropdown.selected())
                    .unwrap_or(0);
                let status_matches = match status {
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
            }
        });

        // IP is the natural order; name and status are handy on busy networks.
        let sorter = gtk::CustomSorter::new({
            let data = imp.data.clone();
            let sort_weak = sort_dropdown.downgrade();
            move |a, b| {
                let (Some(ip_a), Some(ip_b)) = (item_ip(a), item_ip(b)) else {
                    return gtk::Ordering::Equal;
                };
                let mode = sort_weak
                    .upgrade()
                    .map(|dropdown| dropdown.selected())
                    .unwrap_or(0);

                if mode == 1 || mode == 2 {
                    let data = data.borrow();
                    let result_a = data.get(&ip_a).and_then(Option::as_ref);
                    let result_b = data.get(&ip_b).and_then(Option::as_ref);
                    if mode == 1 {
                        // Named devices first; the unnamed ones keep IP order.
                        let name = |result: Option<&HostResult>| {
                            result
                                .and_then(|result| result.hostname.as_deref())
                                .filter(|name| !name.is_empty())
                                .map(str::to_lowercase)
                        };
                        let ordering = match (name(result_a), name(result_b)) {
                            (Some(name_a), Some(name_b)) => name_a.cmp(&name_b),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        };
                        if ordering != std::cmp::Ordering::Equal {
                            return ordering.into();
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
            }
        });

        let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let sort_model = gtk::SortListModel::new(Some(filter_model), Some(sorter.clone()));
        let selection = gtk::NoSelection::new(Some(sort_model));

        let table = gtk::ColumnView::new(Some(selection.clone()));
        table.add_css_class("scanner-table");
        table.add_css_class("data-table");
        table.set_single_click_activate(true);
        table.set_reorderable(false);

        table.append_column(&self.status_column());
        let mut stretchy = Vec::new();
        for column in DataColumn::ALL {
            let view_column = self.text_column(column);
            table.append_column(&view_column);
            if column.expands() {
                stretchy.push(view_column);
            }
        }
        let actions_column = self.actions_column();
        table.append_column(&actions_column);

        // Dragging a column's edge gives it a fixed width. A column that
        // also stretches keeps taking a share of the leftover space on top
        // of that width, so its edge would not follow the pointer: once the
        // user sizes a column, it stops stretching. If none is left to
        // stretch, the trailing column takes the slack so the row buttons
        // stay at the right edge.
        let stretchy_weak: Vec<_> = stretchy.iter().map(|column| column.downgrade()).collect();
        for view_column in &stretchy {
            let stretchy_weak = stretchy_weak.clone();
            let actions_weak = actions_column.downgrade();
            view_column.connect_fixed_width_notify(move |resized| {
                if resized.fixed_width() < 0 {
                    return;
                }
                resized.set_expand(false);
                let any_stretchy = stretchy_weak
                    .iter()
                    .filter_map(|column| column.upgrade())
                    .any(|column| column.expands());
                if !any_stretchy {
                    if let Some(actions) = actions_weak.upgrade() {
                        actions.set_expand(true);
                    }
                }
            });
        }

        let page_weak = self.downgrade();
        table.connect_activate(move |table, position| {
            let ip = table
                .model()
                .and_then(|model| model.item(position))
                .as_ref()
                .and_then(item_ip);
            if let (Some(ip), Some(page)) = (ip, page_weak.upgrade()) {
                page.show_device_details(ip);
            }
        });

        // The rows live in the view's inner list. Marking that list, and not
        // the whole view, as clickable leaves the header with its own cursor.
        if let Some(rows) = table.last_child() {
            rows.set_cursor_from_name(Some("pointer"));
            rows.set_tooltip_text(Some(&tr("Click for device details")));
        }

        search_entry.connect_search_changed({
            let filter = filter.clone();
            move |_| filter.changed(gtk::FilterChange::Different)
        });
        filter_dropdown.connect_selected_notify({
            let filter = filter.clone();
            move |_| filter.changed(gtk::FilterChange::Different)
        });
        sort_dropdown.connect_selected_notify({
            let sorter = sorter.clone();
            move |_| sorter.changed(gtk::SorterChange::Different)
        });

        let scroller = gtk::ScrolledWindow::new();
        // The view scrolls its header together with its rows, so a narrow
        // window can scroll sideways without the columns drifting apart.
        scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroller.set_child(Some(&table));

        // Shown in place of the table when the search or the status filter
        // hides every device. A table without rows has nothing to size its
        // columns by, so it steps aside instead of collapsing.
        let no_match = adw::StatusPage::new();
        no_match.add_css_class("compact");
        no_match.set_icon_name(Some("edit-find-symbolic"));
        no_match.set_title(&tr("No Matching Devices"));
        no_match.set_description(Some(&tr("No device matches the current search or filter.")));

        let table_frame = gtk::Stack::new();
        table_frame.add_css_class("scanner-table-frame");
        table_frame.set_overflow(gtk::Overflow::Hidden);
        table_frame.set_vexpand(true);
        table_frame.set_margin_start(12);
        table_frame.set_margin_end(12);
        table_frame.set_margin_bottom(12);
        table_frame.add_named(&scroller, Some("table"));
        table_frame.add_named(&no_match, Some("no-match"));

        selection.connect_items_changed({
            let table_frame = table_frame.downgrade();
            let store = store.downgrade();
            move |visible, _, _, _| {
                if let Some(table_frame) = table_frame.upgrade() {
                    let has_targets = store.upgrade().is_some_and(|store| store.n_items() > 0);
                    let page = if has_targets && visible.n_items() == 0 {
                        "no-match"
                    } else {
                        "table"
                    };
                    table_frame.set_visible_child_name(page);
                }
            }
        });

        results_box.append(&table_frame);
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
        *imp.stack.borrow_mut() = Some(stack);
        *imp.store.borrow_mut() = Some(store);

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

    /// The leading column: a dot coloured by the address's scan state.
    fn status_column(&self) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.set_halign(gtk::Align::Center);
            dot.set_valign(gtk::Align::Center);
            dot.add_css_class("scanner-dot");
            cell.set_child(Some(&dot));
        });

        let data = self.imp().data.clone();
        factory.connect_bind(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let (Some(dot), Some(ip)) = (cell.child(), cell_ip(cell)) else {
                return;
            };
            let online = data
                .borrow()
                .get(&ip)
                .and_then(Option::as_ref)
                .map(|result| result.online);
            let (class, tooltip) = match online {
                None => ("waiting", tr("Waiting")),
                Some(true) => ("online", tr("Online")),
                Some(false) => ("offline", tr("Offline")),
            };
            for stale in ["waiting", "online", "offline"] {
                dot.remove_css_class(stale);
            }
            dot.add_css_class(class);
            dot.set_tooltip_text(Some(&tooltip));
        });

        let column = gtk::ColumnViewColumn::new(None, Some(factory));
        column.set_resizable(false);
        column
    }

    /// A resizable text column showing one field of each result.
    fn text_column(&self, column: DataColumn) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            // A fixed request keeps long hostnames and vendors from widening
            // the column; the label ellipsizes and its tooltip has the rest.
            label.set_width_chars(column.width_chars());
            label.set_max_width_chars(column.width_chars());
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_single_line_mode(true);
            // The column's font is set once, here: the request above is in
            // characters, so every cell of a column must share one font to
            // ask for the same width.
            if let Some(class) = column.css_class() {
                label.add_css_class(class);
            }
            cell.set_child(Some(&label));
        });

        let data = self.imp().data.clone();
        factory.connect_bind(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let (Some(label), Some(ip)) =
                (cell.child().and_downcast::<gtk::Label>(), cell_ip(cell))
            else {
                return;
            };
            let value = column.value(ip, data.borrow().get(&ip).and_then(Option::as_ref));
            match value {
                Some((text, tooltip)) => {
                    label.set_text(&text);
                    label.set_tooltip_text(tooltip.as_deref());
                    label.remove_css_class("dim-label");
                }
                None => {
                    label.set_text("—");
                    label.set_tooltip_text(None);
                    label.add_css_class("dim-label");
                }
            }
        });

        let view_column = gtk::ColumnViewColumn::new(Some(&column.title()), Some(factory));
        view_column.set_resizable(true);
        view_column.set_expand(column.expands());
        view_column
    }

    /// The trailing column: per-device copy, port scan, and Wake-on-LAN
    /// buttons.
    fn actions_column(&self) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();

        let page_weak = self.downgrade();
        factory.connect_setup(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            actions.set_halign(gtk::Align::End);
            actions.set_valign(gtk::Align::Center);

            // Cells are reused as the list scrolls, so each handler reads
            // the address its cell shows at the time of the click.
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.add_css_class("flat");
            copy.set_tooltip_text(Some(&tr("Copy IP address")));
            copy.connect_clicked({
                let cell = cell.downgrade();
                let page_weak = page_weak.clone();
                move |_| {
                    let ip = cell.upgrade().as_ref().and_then(cell_ip);
                    if let (Some(ip), Some(page)) = (ip, page_weak.upgrade()) {
                        page.copy_to_clipboard(
                            &ip.to_string(),
                            &tr("IP address {ip} copied").replace("{ip}", &ip.to_string()),
                        );
                    }
                }
            });
            actions.append(&copy);

            let ports = gtk::Button::from_icon_name("network-server-symbolic");
            ports.add_css_class("flat");
            ports.set_tooltip_text(Some(&tr("Scan ports")));
            ports.connect_clicked({
                let cell = cell.downgrade();
                let page_weak = page_weak.clone();
                move |_| {
                    let ip = cell.upgrade().as_ref().and_then(cell_ip);
                    if let (Some(ip), Some(page)) = (ip, page_weak.upgrade()) {
                        page.scan_ports(ip);
                    }
                }
            });
            actions.append(&ports);

            let wake = gtk::Button::from_icon_name("system-reboot-symbolic");
            wake.add_css_class("flat");
            wake.connect_clicked({
                let cell = cell.downgrade();
                let page_weak = page_weak.clone();
                move |_| {
                    let ip = cell.upgrade().as_ref().and_then(cell_ip);
                    let (Some(ip), Some(page)) = (ip, page_weak.upgrade()) else {
                        return;
                    };
                    let mac = page
                        .imp()
                        .data
                        .borrow()
                        .get(&ip)
                        .and_then(Option::as_ref)
                        .and_then(|result| result.mac.clone());
                    if let Some(mac) = mac {
                        page.wake_host(&mac);
                    }
                }
            });
            actions.append(&wake);

            cell.set_child(Some(&actions));
        });

        let data = self.imp().data.clone();
        factory.connect_bind(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let (Some(wake), Some(ip)) = (
                cell.child().and_then(|actions| actions.last_child()),
                cell_ip(cell),
            ) else {
                return;
            };
            let data = data.borrow();
            let result = data.get(&ip).and_then(Option::as_ref);
            let online = result.is_some_and(|result| result.online);
            let has_mac = result.is_some_and(|result| result.mac.is_some());

            wake.set_sensitive(result.is_some() && !online && has_mac);
            let tooltip = if online {
                tr("Device is online")
            } else if has_mac || result.is_none() {
                tr("Wake on LAN")
            } else {
                tr("Wake on LAN needs a MAC address")
            };
            wake.set_tooltip_text(Some(&tooltip));
        });

        let column = gtk::ColumnViewColumn::new(None, Some(factory));
        column.set_resizable(false);
        column
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
        // Stop takes the place of Scan, so the row keeps its layout.
        if let Some(button) = imp.scan_button.borrow().as_ref() {
            button.set_visible(!scanning);
        }
        if let Some(button) = imp.cancel_button.borrow().as_ref() {
            button.set_visible(scanning);
            button.set_sensitive(scanning);
        }
        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_visible(scanning);
        }
        if let Some(status) = imp.status_label.borrow().as_ref() {
            // While scanning, the label keeps the width of its longest
            // text so the progress bar beside it does not change size.
            let total = imp.total.get();
            if scanning {
                status.set_width_chars(progress_text(total, total).chars().count() as i32);
                status.set_text(&progress_text(0, total));
            } else {
                status.set_width_chars(-1);
            }
        }
        if scanning {
            if let Some(label) = imp.error_label.borrow().as_ref() {
                label.set_visible(false);
            }
        }
    }

    /// Replace the results with fresh waiting rows for `targets`.
    fn reset_results(&self, targets: &[Ipv4Addr]) {
        let imp = self.imp();

        {
            let mut data = imp.data.borrow_mut();
            data.clear();
            data.extend(targets.iter().map(|&ip| (ip, None)));
        }
        *imp.order.borrow_mut() = targets.to_vec();
        *imp.positions.borrow_mut() = targets
            .iter()
            .enumerate()
            .map(|(position, &ip)| (ip, position as u32))
            .collect();

        // The filter and the sorter read `data` while the store changes, so
        // it is complete, and no longer borrowed, before the rows go in.
        let store = imp.store.borrow().clone();
        if let Some(store) = store {
            let items: Vec<glib::BoxedAnyObject> = targets
                .iter()
                .map(|&ip| glib::BoxedAnyObject::new(ip))
                .collect();
            store.splice(0, store.n_items(), &items);
        }
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

        // A device without a name already shows its IP as the title, so the
        // line below does not repeat it.
        let vendor = result.as_ref().and_then(|result| result.vendor.clone());
        let subtitle = match (hostname.is_some(), vendor) {
            (true, Some(vendor)) => format!("{} • {}", ip, vendor),
            (true, None) => ip.to_string(),
            (false, Some(vendor)) => vendor,
            (false, None) => String::new(),
        };
        let subtitle_label = gtk::Label::new(Some(&subtitle));
        subtitle_label.add_css_class("dim-label");
        subtitle_label.set_halign(gtk::Align::Start);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subtitle_label.set_tooltip_text(Some(&subtitle));
        subtitle_label.set_visible(!subtitle.is_empty());
        hero_text.set_valign(gtk::Align::Center);
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
            let row = adw::ActionRow::new();
            row.set_title(&tr("No open services detected"));
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
                let page_weak = self.downgrade();
                wake_button.connect_clicked(move |_| {
                    if let Some(page) = page_weak.upgrade() {
                        page.wake_host(&mac);
                    }
                });
                buttons.append(&wake_button);
            }
        }

        // Hands the device to the port scanner page, then gets out of its way.
        let ports_button = gtk::Button::with_label(&tr("Scan Ports"));
        let page_weak = self.downgrade();
        let dialog_weak = dialog.downgrade();
        ports_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.scan_ports(ip);
            }
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.close();
            }
        });
        buttons.append(&ports_button);

        let copy_button = gtk::Button::with_label(&tr("Copy Details"));
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
        // Values are plain text: a ping of "< 0.1 ms" is not valid markup
        // and would otherwise leave the row blank.
        row.set_use_markup(false);
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
                *slot = Some(result);
            }
        }
        if online {
            imp.online.set(imp.online.get() + 1);
        }

        // Replacing the address's item makes the table redraw its row and
        // run the filter and the sorter on it again.
        let position = imp.positions.borrow().get(&ip).copied();
        let store = imp.store.borrow().clone();
        if let (Some(position), Some(store)) = (position, store) {
            if position < store.n_items() {
                store.splice(position, 1, &[glib::BoxedAnyObject::new(ip)]);
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
            label.set_text(&progress_text(completed, total));
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
            // A stopped scan covered only part of the range.
            tr("{online} online • {total} scanned")
                .replace("{online}", &online.to_string())
                .replace("{total}", &scanned.to_string())
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

    /// Open the port scanner page on a device and start scanning it.
    fn scan_ports(&self, ip: Ipv4Addr) {
        // The window owns the pages, so it does the handover.
        let _ = self.activate_action("win.scan-ports", Some(&ip.to_string().to_variant()));
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
