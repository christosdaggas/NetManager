// Network Manager - Port Scanner Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Port scanner page.
//!
//! Tries a TCP connection to each chosen port of one device and lists the
//! ports that accept it, with the service usually found there and the time
//! the connection took. It shares its look with the network scanner page.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::scanner_page::{csv_field, format_latency, labelled_icon_button, progress_text};
use crate::services::port_scanner::{self, OpenPort, PortListError, PortScanEvent, ScanSummary};
use crate::tr;

/// The choices of the "which ports" selector, in display order.
const PORTS_COMMON: u32 = 0;
const PORTS_WELL_KNOWN: u32 = 1;
const PORTS_ALL: u32 = 2;
const PORTS_CUSTOM: u32 = 3;

/// A text column of the results table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortColumn {
    Port,
    Service,
    Response,
}

impl PortColumn {
    const ALL: [Self; 3] = [Self::Port, Self::Service, Self::Response];

    fn title(self) -> String {
        match self {
            Self::Port => tr("Port"),
            Self::Service => tr("Service"),
            Self::Response => tr("Response"),
        }
    }

    /// Width the column asks for, in characters of its own font.
    fn width_chars(self) -> i32 {
        match self {
            Self::Port => 7,
            Self::Service => 28,
            Self::Response => 9,
        }
    }

    fn css_class(self) -> Option<&'static str> {
        match self {
            Self::Port => Some("scanner-ip"),
            Self::Service => None,
            Self::Response => Some("scanner-numeric"),
        }
    }

    /// Cell text for an open port; `None` shows a dash.
    fn value(self, open: &OpenPort) -> Option<String> {
        match self {
            Self::Port => Some(open.port.to_string()),
            Self::Service => port_scanner::service_name(open.port),
            Self::Response => Some(format_latency(Some(open.latency_ms))),
        }
    }
}

/// The open port shown by a table cell.
fn cell_port(cell: &gtk::ColumnViewCell) -> Option<OpenPort> {
    cell.item().as_ref().and_then(item_port)
}

fn item_port(item: &glib::Object) -> Option<OpenPort> {
    item.downcast_ref::<glib::BoxedAnyObject>()
        .and_then(|boxed| boxed.try_borrow::<OpenPort>().ok().map(|open| *open))
}

/// `host:port`, with the brackets an IPv6 address needs.
fn endpoint(host: &str, port: u16) -> String {
    if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PortScannerPage {
        pub host_entry: RefCell<Option<gtk::Entry>>,
        pub ports_dropdown: RefCell<Option<gtk::DropDown>>,
        pub custom_entry: RefCell<Option<gtk::Entry>>,
        pub scan_button: RefCell<Option<gtk::Button>>,
        pub cancel_button: RefCell<Option<gtk::Button>>,
        pub export_button: RefCell<Option<gtk::Button>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub error_label: RefCell<Option<gtk::Label>>,
        pub count_label: RefCell<Option<gtk::Label>>,
        /// Start page vs. results.
        pub stack: RefCell<Option<gtk::Stack>>,
        /// Table vs. the note shown while it has no rows.
        pub results_stack: RefCell<Option<gtk::Stack>>,
        pub placeholder: RefCell<Option<adw::StatusPage>>,
        /// One item per open port, in the order they were found.
        pub store: RefCell<Option<gio::ListStore>>,

        /// The host as it was typed for the current results.
        pub target: RefCell<String>,
        pub resolved: Cell<Option<IpAddr>>,
        pub resolve_failed: Cell<bool>,
        pub summary: Cell<Option<ScanSummary>>,

        pub cancel_flag: RefCell<Option<Arc<AtomicBool>>>,
        pub generation: Cell<u64>,
        pub scanning: Cell<bool>,
        pub cancelled: Cell<bool>,
        pub scanned: Cell<usize>,
        pub total: Cell<usize>,
        pub scan_started: Cell<Option<Instant>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PortScannerPage {
        const NAME: &'static str = "CdNetworkManagerPortScannerPage";
        type Type = super::PortScannerPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for PortScannerPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for PortScannerPage {}
    impl BoxImpl for PortScannerPage {}
}

glib::wrapper! {
    pub struct PortScannerPage(ObjectSubclass<imp::PortScannerPage>)
        @extends gtk::Widget, gtk::Box;
}

impl PortScannerPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Page header: title and a quiet export action.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.set_margin_top(18);
        header.set_margin_bottom(12);
        header.set_margin_start(12);
        header.set_margin_end(12);

        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        title_box.set_hexpand(true);

        let title = gtk::Label::new(Some(&tr("Port Scanner")));
        title.add_css_class("title-2");
        title.set_halign(gtk::Align::Start);
        title_box.append(&title);

        let subtitle = gtk::Label::new(Some(&tr("Find the open TCP ports of a device")));
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

        // Scan controls: the device, the ports, and the actions.
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.add_css_class("scanner-controls");
        controls.set_margin_start(12);
        controls.set_margin_end(12);
        controls.set_margin_bottom(12);

        let control_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let host_icon = gtk::Image::from_icon_name("network-server-symbolic");
        host_icon.add_css_class("accent");
        host_icon.set_valign(gtk::Align::Center);
        control_row.append(&host_icon);

        let host_entry = gtk::Entry::new();
        host_entry.set_hexpand(true);
        host_entry.set_valign(gtk::Align::Center);
        host_entry.set_placeholder_text(Some(&tr("IP address or host name")));
        host_entry.set_tooltip_text(Some(&tr("Examples: 192.168.1.1 or printer.local")));
        // The router is the device most people check first.
        if let Some(gateway) = port_scanner::default_gateway() {
            host_entry.set_text(&gateway);
        }
        control_row.append(&host_entry);

        let ports_labels = [
            tr("Common ports"),
            tr("Ports 1–1024"),
            tr("All ports"),
            tr("Custom"),
        ];
        let ports_refs: Vec<&str> = ports_labels.iter().map(String::as_str).collect();
        let ports_dropdown = gtk::DropDown::from_strings(&ports_refs);
        ports_dropdown.set_selected(PORTS_COMMON);
        ports_dropdown.set_valign(gtk::Align::Center);
        ports_dropdown.set_tooltip_text(Some(&tr("Which ports to scan")));
        control_row.append(&ports_dropdown);

        let custom_entry = gtk::Entry::new();
        custom_entry.set_valign(gtk::Align::Center);
        custom_entry.set_width_chars(22);
        custom_entry.set_placeholder_text(Some("22, 80, 8000-8100"));
        custom_entry.set_tooltip_text(Some(&tr(
            "Ports and ranges, separated by commas: 22, 80, 8000-8100",
        )));
        custom_entry.set_visible(false);
        control_row.append(&custom_entry);

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

        // Start page vs. results.
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(150);

        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("network-server-symbolic"));
        empty_state.set_title(&tr("Scan a Device's Ports"));
        empty_state.set_description(Some(&tr(
            "See which services a computer, router, or printer accepts \
             connections on. Enter its IP address or host name to begin.",
        )));

        let empty_scan_button = gtk::Button::with_label(&tr("Start Scan"));
        empty_scan_button.add_css_class("suggested-action");
        empty_scan_button.add_css_class("pill");
        empty_scan_button.set_halign(gtk::Align::Center);
        empty_state.set_child(Some(&empty_scan_button));

        stack.add_named(&empty_state, Some("empty"));

        let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let count_label = gtk::Label::new(None);
        count_label.add_css_class("caption");
        count_label.add_css_class("dim-label");
        count_label.add_css_class("numeric");
        count_label.set_xalign(0.0);
        count_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        count_label.set_margin_start(12);
        count_label.set_margin_end(12);
        count_label.set_margin_bottom(8);
        results_box.append(&count_label);

        // The results table: open ports only, lowest port first.
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let sorter = gtk::CustomSorter::new(|a, b| {
            let port = |item: &glib::Object| item_port(item).map(|open| open.port);
            port(a).cmp(&port(b)).into()
        });
        let sort_model = gtk::SortListModel::new(Some(store.clone()), Some(sorter));
        let selection = gtk::NoSelection::new(Some(sort_model));

        let table = gtk::ColumnView::new(Some(selection));
        table.add_css_class("scanner-table");
        table.add_css_class("data-table");
        table.set_reorderable(false);
        for column in PortColumn::ALL {
            table.append_column(&self.text_column(column));
        }
        table.append_column(&self.actions_column());

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroller.set_child(Some(&table));

        // Stands in for the table while it has no rows: a table without
        // rows has nothing to size its columns by.
        let placeholder = adw::StatusPage::new();
        placeholder.add_css_class("compact");

        let results_stack = gtk::Stack::new();
        results_stack.add_css_class("scanner-table-frame");
        results_stack.set_overflow(gtk::Overflow::Hidden);
        results_stack.set_vexpand(true);
        results_stack.set_margin_start(12);
        results_stack.set_margin_end(12);
        results_stack.set_margin_bottom(12);
        results_stack.add_named(&scroller, Some("table"));
        results_stack.add_named(&placeholder, Some("placeholder"));

        results_box.append(&results_stack);
        stack.add_named(&results_box, Some("results"));
        stack.set_visible_child_name("empty");

        self.append(&stack);

        // Store references.
        *imp.host_entry.borrow_mut() = Some(host_entry.clone());
        *imp.ports_dropdown.borrow_mut() = Some(ports_dropdown.clone());
        *imp.custom_entry.borrow_mut() = Some(custom_entry.clone());
        *imp.scan_button.borrow_mut() = Some(scan_button.clone());
        *imp.cancel_button.borrow_mut() = Some(cancel_button.clone());
        *imp.export_button.borrow_mut() = Some(export_button.clone());
        *imp.progress_bar.borrow_mut() = Some(progress_bar);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.error_label.borrow_mut() = Some(error_label);
        *imp.count_label.borrow_mut() = Some(count_label);
        *imp.stack.borrow_mut() = Some(stack);
        *imp.results_stack.borrow_mut() = Some(results_stack);
        *imp.placeholder.borrow_mut() = Some(placeholder);
        *imp.store.borrow_mut() = Some(store);

        // Actions.
        for button in [&scan_button, &empty_scan_button] {
            let page_weak = self.downgrade();
            button.connect_clicked(move |_| {
                if let Some(page) = page_weak.upgrade() {
                    page.start_scan();
                }
            });
        }

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

        // The custom list only takes up room when it is in use.
        let custom_weak = custom_entry.downgrade();
        let page_weak = self.downgrade();
        ports_dropdown.connect_selected_notify(move |dropdown| {
            let custom = dropdown.selected() == PORTS_CUSTOM;
            if let Some(entry) = custom_weak.upgrade() {
                entry.set_visible(custom);
                entry.remove_css_class("error");
                if custom {
                    entry.grab_focus();
                }
            }
            if let Some(page) = page_weak.upgrade() {
                page.clear_error();
            }
        });

        // A fresh edit hides the previous validation message; Enter scans.
        for entry in [&host_entry, &custom_entry] {
            let page_weak = self.downgrade();
            entry.connect_changed(move |entry| {
                entry.remove_css_class("error");
                if let Some(page) = page_weak.upgrade() {
                    page.clear_error();
                }
            });

            let page_weak = self.downgrade();
            entry.connect_activate(move |_| {
                if let Some(page) = page_weak.upgrade() {
                    page.start_scan();
                }
            });
        }
    }

    /// A resizable text column showing one field of each open port.
    fn text_column(&self, column: PortColumn) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            // A fixed request in characters gives every cell of a column the
            // same width, so the table does not shift as ports are found.
            label.set_width_chars(column.width_chars());
            label.set_max_width_chars(column.width_chars());
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_single_line_mode(true);
            if let Some(class) = column.css_class() {
                label.add_css_class(class);
            }
            cell.set_child(Some(&label));
        });

        factory.connect_bind(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            let (Some(label), Some(open)) =
                (cell.child().and_downcast::<gtk::Label>(), cell_port(cell))
            else {
                return;
            };
            match column.value(&open) {
                Some(text) => {
                    label.set_text(&text);
                    label.set_tooltip_text(Some(&text));
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
        view_column
    }

    /// The trailing column: copy the port's address.
    fn actions_column(&self) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();

        let page_weak = self.downgrade();
        factory.connect_setup(move |_, object| {
            let Some(cell) = object.downcast_ref::<gtk::ColumnViewCell>() else {
                return;
            };
            // Cells are reused as the list scrolls, so the handler reads the
            // port its cell shows at the time of the click.
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.add_css_class("flat");
            copy.set_halign(gtk::Align::End);
            copy.set_valign(gtk::Align::Center);
            copy.set_tooltip_text(Some(&tr("Copy address and port")));
            copy.connect_clicked({
                let cell = cell.downgrade();
                let page_weak = page_weak.clone();
                move |_| {
                    let open = cell.upgrade().as_ref().and_then(cell_port);
                    if let (Some(open), Some(page)) = (open, page_weak.upgrade()) {
                        let address = endpoint(&page.imp().target.borrow(), open.port);
                        page.copy_to_clipboard(
                            &address,
                            &tr("{address} copied").replace("{address}", &address),
                        );
                    }
                }
            });
            cell.set_child(Some(&copy));
        });

        // The three text columns stay together at the left where they read
        // as one line; this column takes the rest of the width and keeps
        // its button at the right edge.
        let column = gtk::ColumnViewColumn::new(None, Some(factory));
        column.set_resizable(false);
        column.set_expand(true);
        column
    }

    /// The ports chosen in the controls.
    fn selected_ports(&self) -> Result<Vec<u16>, PortListError> {
        let imp = self.imp();
        let choice = imp
            .ports_dropdown
            .borrow()
            .as_ref()
            .map(|dropdown| dropdown.selected())
            .unwrap_or(PORTS_COMMON);
        match choice {
            PORTS_WELL_KNOWN => Ok(port_scanner::port_range(port_scanner::WELL_KNOWN_LAST)),
            PORTS_ALL => Ok(port_scanner::port_range(u16::MAX)),
            PORTS_CUSTOM => {
                let text = imp
                    .custom_entry
                    .borrow()
                    .as_ref()
                    .map(|entry| entry.text().to_string())
                    .unwrap_or_default();
                port_scanner::parse_ports(&text)
            }
            _ => Ok(port_scanner::TOP_PORTS.to_vec()),
        }
    }

    /// Scan `host` with the ports currently chosen, replacing the host in
    /// the controls. Does nothing while another scan is running.
    pub fn scan_host(&self, host: &str) {
        let imp = self.imp();
        if imp.scanning.get() {
            self.show_toast(&tr("A port scan is already running"));
            return;
        }
        if let Some(entry) = imp.host_entry.borrow().as_ref() {
            entry.set_text(host);
        }
        self.start_scan();
    }

    /// Start a scan using the host and ports currently in the controls.
    pub fn start_scan(&self) {
        let imp = self.imp();
        if imp.scanning.get() {
            return;
        }

        let host = imp
            .host_entry
            .borrow()
            .as_ref()
            .map(|entry| entry.text().trim().to_string())
            .unwrap_or_default();
        if host.is_empty() {
            self.show_error(
                &tr("Enter the IP address or host name of the device to scan"),
                imp.host_entry.borrow().as_ref(),
            );
            return;
        }

        if !port_scanner::is_valid_host(&host) {
            self.show_error(
                &tr("“{host}” is not an IP address or a host name").replace("{host}", &host),
                imp.host_entry.borrow().as_ref(),
            );
            return;
        }

        let ports = match self.selected_ports() {
            Ok(ports) => ports,
            Err(error) => {
                let text = match error {
                    PortListError::Empty => {
                        tr("Enter the ports to scan, for example 22, 80, 8000-8100")
                    }
                    PortListError::Invalid(item) => {
                        tr("“{item}” is not a port or a range. Use numbers from 1 to 65535.")
                            .replace("{item}", &item)
                    }
                };
                self.show_error(&text, imp.custom_entry.borrow().as_ref());
                return;
            }
        };
        self.clear_error();

        imp.total.set(ports.len());
        imp.scanned.set(0);
        imp.cancelled.set(false);
        imp.scanning.set(true);
        imp.scan_started.set(Some(Instant::now()));
        imp.resolved.set(None);
        imp.resolve_failed.set(false);
        imp.summary.set(None);
        *imp.target.borrow_mut() = host.clone();
        let generation = imp.generation.get().wrapping_add(1);
        imp.generation.set(generation);

        if let Some(store) = imp.store.borrow().as_ref() {
            store.remove_all();
        }
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
        self.update_results_view();

        let handle = port_scanner::start_scan(host, ports);
        *imp.cancel_flag.borrow_mut() = Some(handle.cancel);
        let events = handle.events;

        let page_weak = self.downgrade();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            let Some(page) = page_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = page.imp();
            if imp.generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            // A scan of every port reports tens of thousands of steps, so
            // each tick applies only the latest count and adds the ports it
            // found in one go.
            let mut finished = false;
            let mut found = Vec::new();
            let mut progress = None;
            loop {
                match events.try_recv() {
                    Ok(PortScanEvent::Resolved(ip)) => {
                        imp.resolved.set(Some(ip));
                        page.update_progress(0, imp.total.get());
                    }
                    Ok(PortScanEvent::ResolveFailed) => imp.resolve_failed.set(true),
                    Ok(PortScanEvent::Open(open)) => found.push(glib::BoxedAnyObject::new(open)),
                    Ok(PortScanEvent::Progress { completed, total }) => {
                        // Workers report out of order; keep the furthest.
                        let furthest = progress.map(|(latest, _)| latest).unwrap_or(0);
                        if completed >= furthest {
                            progress = Some((completed, total));
                        }
                    }
                    Ok(PortScanEvent::Finished(summary)) => {
                        imp.summary.set(Some(summary));
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

            if let Some((completed, total)) = progress {
                page.update_progress(completed, total);
            }
            if !found.is_empty() {
                if let Some(store) = imp.store.borrow().as_ref() {
                    store.extend_from_slice(&found);
                }
            }

            if finished {
                page.finish_scan();
                glib::ControlFlow::Break
            } else {
                page.update_results_view();
                glib::ControlFlow::Continue
            }
        });
    }

    /// Ask the running scan to stop after the ports in flight.
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
        if let Some(entry) = imp.host_entry.borrow().as_ref() {
            entry.set_sensitive(!scanning);
        }
        if let Some(dropdown) = imp.ports_dropdown.borrow().as_ref() {
            dropdown.set_sensitive(!scanning);
        }
        if let Some(entry) = imp.custom_entry.borrow().as_ref() {
            entry.set_sensitive(!scanning);
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
                // A name is looked up first, which can take a moment.
                let host = imp.target.borrow();
                if host.parse::<IpAddr>().is_ok() {
                    status.set_text(&progress_text(0, total));
                } else {
                    status.set_text(&tr("Looking up {host}…").replace("{host}", &host));
                }
            } else {
                status.set_width_chars(-1);
            }
        }
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
        // "Stopping…" stays up until the workers have wound down.
        if !imp.cancelled.get() {
            if let Some(label) = imp.status_label.borrow().as_ref() {
                label.set_text(&progress_text(completed, total));
            }
        }
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
        let open = self.open_count();
        let host = imp.target.borrow().clone();

        if let Some(label) = imp.status_label.borrow().as_ref() {
            let text = if imp.resolve_failed.get() {
                tr("Scan not started")
            } else if imp.cancelled.get() {
                tr("Scan stopped after {scanned} of {total} ports")
                    .replace("{scanned}", &scanned.to_string())
                    .replace("{total}", &total.to_string())
            } else {
                tr("Scan complete in {seconds} s").replace("{seconds}", &format!("{:.1}", elapsed))
            };
            label.set_text(&text);
        }

        self.set_scanning_ui(false);
        if let Some(button) = imp.export_button.borrow().as_ref() {
            button.set_sensitive(open > 0);
        }
        if imp.resolve_failed.get() {
            self.show_error(
                &tr("Could not find “{host}”. Check the address or name and try again.")
                    .replace("{host}", &host),
                imp.host_entry.borrow().as_ref(),
            );
        }
        self.update_results_view();

        tracing::info!(
            "Port scan of {} finished: {} open of {} ports in {:.1}s",
            host,
            open,
            scanned,
            elapsed
        );
    }

    fn open_count(&self) -> usize {
        self.imp()
            .store
            .borrow()
            .as_ref()
            .map(|store| store.n_items() as usize)
            .unwrap_or(0)
    }

    /// The scanned device as shown above the results: the name that was
    /// typed, with its address when that differs.
    fn target_label(&self) -> String {
        let imp = self.imp();
        let host = imp.target.borrow().clone();
        match imp.resolved.get() {
            Some(ip) if ip.to_string() != host => format!("{} ({})", host, ip),
            _ => host,
        }
    }

    /// Bring the count line, and the table or the note that stands in for
    /// it, in step with the scan.
    fn update_results_view(&self) {
        let imp = self.imp();
        let open = self.open_count();
        let scanned = imp.scanned.get();
        let total = imp.total.get();
        let scanning = imp.scanning.get();

        if let Some(label) = imp.count_label.borrow().as_ref() {
            let text = if imp.resolve_failed.get() {
                String::new()
            } else if scanning {
                tr("{host} • {open} open • {scanned} of {total} ports scanned")
                    .replace("{scanned}", &scanned.to_string())
                    .replace("{total}", &total.to_string())
            } else {
                tr("{host} • {open} open • {scanned} ports scanned")
                    .replace("{scanned}", &scanned.to_string())
            };
            label.set_text(
                &text
                    .replace("{host}", &self.target_label())
                    .replace("{open}", &open.to_string()),
            );
        }

        let Some(results_stack) = imp.results_stack.borrow().clone() else {
            return;
        };
        if open > 0 {
            results_stack.set_visible_child_name("table");
            return;
        }

        let summary = imp.summary.get().unwrap_or_default();
        let (icon, title, description) = if imp.resolve_failed.get() {
            (
                "dialog-question-symbolic",
                tr("Device Not Found"),
                tr("The name does not lead to a device. Check the spelling, or use the IP address."),
            )
        } else if scanning {
            (
                "system-search-symbolic",
                tr("Scanning…"),
                tr("No open ports found so far."),
            )
        } else if imp.cancelled.get() {
            (
                "process-stop-symbolic",
                tr("No Open Ports Found"),
                tr("The scan was stopped before it found an open port."),
            )
        } else if summary.closed > 0 {
            (
                "channel-secure-symbolic",
                tr("No Open Ports"),
                tr("The device answered, but none of the scanned ports accepts connections."),
            )
        } else {
            (
                "network-offline-symbolic",
                tr("No Answer"),
                tr(
                    "The device did not answer on any scanned port. It may be switched off, \
                    out of reach, or blocking connections.",
                ),
            )
        };
        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_icon_name(Some(icon));
            placeholder.set_title(&title);
            placeholder.set_description(Some(&description));
        }
        results_stack.set_visible_child_name("placeholder");
    }

    /// Show a validation message and point at the entry it is about.
    fn show_error(&self, text: &str, entry: Option<&gtk::Entry>) {
        if let Some(entry) = entry {
            entry.add_css_class("error");
            entry.grab_focus();
        }
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_text(text);
            label.set_visible(true);
        }
    }

    fn clear_error(&self) {
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_visible(false);
        }
    }

    /// Offer the router again while the page is idle and its host is empty.
    pub fn refresh(&self) {
        let imp = self.imp();
        if imp.scanning.get() {
            return;
        }
        if let Some(entry) = imp.host_entry.borrow().as_ref() {
            if entry.text().trim().is_empty() {
                if let Some(gateway) = port_scanner::default_gateway() {
                    entry.set_text(&gateway);
                }
            }
        }
    }

    fn copy_to_clipboard(&self, text: &str, message: &str) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
            self.show_toast(message);
        }
    }

    fn export_results(&self) {
        let imp = self.imp();
        let host = imp.target.borrow().clone();
        let address = imp
            .resolved
            .get()
            .map(|ip| ip.to_string())
            .unwrap_or_default();

        let mut open_ports: Vec<OpenPort> = imp
            .store
            .borrow()
            .as_ref()
            .map(|store| {
                (0..store.n_items())
                    .filter_map(|position| store.item(position))
                    .filter_map(|item| item_port(&item))
                    .collect()
            })
            .unwrap_or_default();
        if open_ports.is_empty() {
            self.show_toast(&tr("No scan results to export"));
            return;
        }
        open_ports.sort_by_key(|open| open.port);

        let mut csv = String::from("Host,Address,Port,Protocol,Service,Response (ms)\n");
        for open in &open_ports {
            csv.push_str(&format!(
                "{},{},{},TCP,{},{:.1}\n",
                csv_field(&host),
                csv_field(&address),
                open.port,
                csv_field(&port_scanner::service_name(open.port).unwrap_or_default()),
                open.latency_ms,
            ));
        }

        // Keep the host recognisable in the file name without path characters.
        let safe_host: String = host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dialog = gtk::FileDialog::builder()
            .title(tr("Export Scan Results"))
            .initial_name(format!(
                "port-scan-{}-{}.csv",
                safe_host,
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

impl Default for PortScannerPage {
    fn default() -> Self {
        Self::new()
    }
}
