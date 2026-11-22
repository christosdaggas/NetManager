# The release profile in Cargo.toml strips the binary, so there is no
# debug information for rpmbuild to extract.
%global debug_package %{nil}

%global app_id com.chrisdaggas.network-manager

Name:           network-manager
Version:        1.1.0
Release:        1%{?dist}
Summary:        Network and system profile manager for GNOME

License:        MIT
URL:            https://chrisdaggas.com
Source0:        %{name}-%{version}.tar.gz

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc-c++
BuildRequires:  python3
BuildRequires:  gettext
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  pkgconfig(openssl)
BuildRequires:  gettext-devel
BuildRequires:  desktop-file-utils
BuildRequires:  appstream

Requires:       NetworkManager
Requires:       iproute
Requires:       polkit
Requires:       hicolor-icon-theme
Recommends:     bubblewrap

%description
Network Manager lets you save your network and system settings as
profiles and switch between them in one click. A profile can change
IP addresses, DNS servers, routes, Wi-Fi and VPN connections, the
proxy, the hostname, the firewall zone, the default printer and the
timezone, and can switch automatically based on the Wi-Fi network,
the gateway or the time of day.

Made for Fedora with the GNOME desktop.

%prep
%autosetup

%build
cargo build --release --locked -p network-manager

%install
install -Dpm 0755 "${CARGO_TARGET_DIR:-target}/release/network-manager" \
    %{buildroot}%{_bindir}/network-manager
install -Dpm 0644 data/%{app_id}.desktop \
    %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dpm 0644 data/%{app_id}.metainfo.xml \
    %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
install -Dpm 0644 data/icons/hicolor/scalable/apps/%{app_id}.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg
install -Dpm 0644 data/icons/hicolor/symbolic/apps/%{app_id}-symbolic.svg \
    %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/%{app_id}-symbolic.svg
bash scripts/install-catalogs.sh %{buildroot}%{_datadir}/locale

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
cargo test --release --locked -p netmanager-core
test "$(%{buildroot}%{_bindir}/network-manager --version)" = "Network Manager %{version}"

%files
%license LICENSE
%doc README.md
%{_bindir}/network-manager
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg
%{_datadir}/icons/hicolor/symbolic/apps/%{app_id}-symbolic.svg
%{_datadir}/locale/*/LC_MESSAGES/network-manager.mo

%changelog
* Mon Nov 23 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.1.0-1
- Replace the GTK4 interface with the Tauri desktop and retain the existing launcher and profile library.

* Thu Oct 22 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.0.0-1
- First release, made for Fedora with the GNOME desktop
