// Network Manager - Help Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Help Page - Application documentation and guidance.

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::glib;
use gtk4::subclass::prelude::*;

use crate::tr;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HelpPage {}

    #[glib::object_subclass]
    impl ObjectSubclass for HelpPage {
        const NAME: &'static str = "HelpPage";
        type Type = super::HelpPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for HelpPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for HelpPage {}
    impl BoxImpl for HelpPage {}
}

glib::wrapper! {
    pub struct HelpPage(ObjectSubclass<imp::HelpPage>)
        @extends gtk::Widget, gtk::Box;
}

impl HelpPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        // Page header
        let header_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        header_box.set_margin_start(24);
        header_box.set_margin_end(24);
        header_box.set_margin_top(24);
        header_box.set_margin_bottom(12);

        let title = gtk::Label::new(Some(&tr("Help")));
        title.add_css_class("title-1");
        title.set_halign(gtk::Align::Start);
        header_box.append(&title);

        let subtitle = gtk::Label::new(Some(&tr("Learn how to use Network Manager")));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(gtk::Align::Start);
        header_box.append(&subtitle);

        self.append(&header_box);

        // Scrollable content
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content_box.set_margin_start(24);
        content_box.set_margin_end(24);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(24);

        // About section
        content_box.append(&self.create_section(
            &tr("About Network Manager"),
            &tr("Network Manager is a comprehensive system and network profile manager for Linux. \
             It allows you to create, manage, and quickly switch between different network and system \
             configuration profiles. Perfect for users who need different settings for home, work, \
             or different network environments.")
        ));

        // Dashboard section
        content_box.append(&self.create_section(
            &tr("Dashboard"),
            &tr("The Dashboard provides an overview of your current network configuration. \
             View your active network connections, IP addresses, and connection status. \
             The active profile is displayed prominently, and you can quickly switch \
             between profiles using the profile selector. Network statistics and \
             connection health are updated in real-time.")
        ));

        // Scanner section
        content_box.append(&self.create_section(
            &tr("Network Scanner"),
            &tr("The Network Scanner finds devices on your local network. Enter a \
             subnet (for example 192.168.1.0/24), a range (192.168.1.1-254), or a \
             single address, then press Scan.\n\n\
             Fast mode uses ping and the system neighbour table. Thorough mode also \
             probes common service ports, which can find devices that ignore ping and \
             shows services such as SSH, HTTP, SMB, Plex, or Cockpit. Results include \
             the device name, MAC address, vendor, reply time, and open services. You \
             can search, filter, sort, copy addresses, wake a sleeping device over the \
             LAN, and export the list as a CSV file.")
        ));

        // Profiles section
        content_box.append(&self.create_section(
            &tr("Profiles"),
            &tr("Profiles are the core feature of Network Manager. Each profile stores a complete \
             set of network and system settings including:\n\n\
             • Network configuration (IP, DNS, gateway)\n\
             • Proxy settings\n\
             • Firewall rules\n\
             • System services to enable/disable\n\
             • Custom scripts to run on activation\n\n\
             Create profiles for different locations or use cases, then switch between them \
             with a single click. Profiles can be exported and imported for backup or sharing.")
        ));

        // Logs section
        content_box.append(&self.create_section(
            &tr("Logs"),
            &tr("The Logs page shows network-related system events and profile switch history. \
             View connection changes, profile activations, and any errors that occurred. \
             Filter logs by type or search for specific events. Logs help you troubleshoot \
             connectivity issues and track configuration changes over time.")
        ));

        // Settings section
        content_box.append(&self.create_section(
            &tr("Settings"),
            &tr("The Settings page allows you to configure application preferences. \
             Set the default profile to activate at startup, configure automatic \
             profile switching based on network detection, enable or disable \
             system tray integration, and customize notification preferences. \
             Advanced options include daemon configuration and privilege settings.")
        ));

        // Profile Features section
        content_box.append(&self.create_section(
            &tr("Profile Features"),
            &tr("Each profile can include:\n\n\
             • Static or DHCP IP configuration\n\
             • Custom DNS servers\n\
             • HTTP/HTTPS/SOCKS proxy settings\n\
             • Firewall zone assignment\n\
             • VPN connection settings\n\
             • System services to start/stop\n\
             • Pre/post activation scripts\n\
             • Network interface selection")
        ));

        // Tips section
        content_box.append(&self.create_section(
            &tr("Tips"),
            &tr("• Create a 'Default' profile as a baseline configuration.\n\
             • Use descriptive profile names like 'Home WiFi' or 'Office Wired'.\n\
             • Test profiles before relying on them in critical situations.\n\
             • Export important profiles for backup.\n\
             • Use the system tray for quick profile switching.\n\
             • Check logs if a profile switch doesn't work as expected.")
        ));

        scroll.set_child(Some(&content_box));
        self.append(&scroll);
    }

    fn create_section(&self, title: &str, description: &str) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);

        let title_label = gtk::Label::new(Some(title));
        title_label.add_css_class("title-3");
        title_label.set_halign(gtk::Align::Start);
        section.append(&title_label);

        let desc_label = gtk::Label::new(Some(description));
        desc_label.set_wrap(true);
        desc_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        desc_label.set_xalign(0.0);
        desc_label.set_halign(gtk::Align::Start);
        desc_label.add_css_class("body");
        section.append(&desc_label);

        section
    }
}
