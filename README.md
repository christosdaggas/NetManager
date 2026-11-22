# Network Manager

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/workspace-dark.webp">
  <img width="100%" alt="Tauri workspace showing connection status, live traffic, and saved profiles" src="docs/workspace-light.webp">
</picture>

**Save your network settings as profiles and switch between them in one click.**

Made for Fedora with the GNOME desktop (tested on Fedora 44).

</div>

---

## Tauri desktop workspace (this branch)

The Linux desktop UI opens on a live traffic graph with download and upload
rates. Use the pause control to inspect a reading. The connection inspector
shows the active interface, IPv4 address, gateway, resolver, and link speed.
Use the left sidebar to open **Profiles** and
create a DHCP or static IPv4 configuration, set DNS, edit a profile, or review
its actions before applying. **Interfaces** shows link state and hardware
details; **Activity log** searches the most recent 500 entries.

### Run it on Fedora

```bash
sudo dnf install cargo gcc gcc-c++ pkgconf-pkg-config gtk3-devel webkit2gtk4.1-devel glib2-devel gettext-devel openssl-devel librsvg2-devel
cargo run --locked --manifest-path src-tauri/Cargo.toml
```

No Node.js build or development server is needed. `iproute` supplies route and
address information; profile activation uses NetworkManager's `nmcli` and the
same Linux helpers as the GTK edition. To build an optimized executable, add
`--release` to `cargo build --locked --manifest-path src-tauri/Cargo.toml`.

The two editions share the profile library, encryption format, and activity log
under `~/.local/share/daggas/network-manager/`. Profile activation uses the
same Rust executor. The Tauri editor preserves settings outside its IPv4/DNS
form. **Classic tools** opens the separately installed or locally built GTK
app for advanced editing, encryption setup, scanners, and background services
(tray, watchdog, scheduler, and automatic switching). These services run only
in the GTK edition. Finish editing in one window before editing in the other;
use **Refresh** after returning to the workspace.

The sidebar fills the window height. Use its bottom-left button or **Ctrl+B**
to switch between labels and an [icon rail](docs/workspace-compact.webp). The workspace remembers that choice,
window placement, last page, theme, and sampling
interval. Traffic history is session-only and sampling stops when Overview is
not visible. Press **Ctrl+K** for commands, **Ctrl+1–5** to navigate, **Ctrl+N**
to create a profile, **Ctrl+F** to filter profiles or activity, and **F5** to
refresh. **Preferences** offers system, light, and dark appearances.

### Verification

```bash
cargo test --locked
cargo build --locked --manifest-path src-tauri/Cargo.toml
python3 scripts/test-tauri-workspace.py
```

The UI smoke test needs a graphical Linux session, `dbus-run-session`, and
`WebKitWebDriver` (on Fedora, find its package with
`dnf provides '*/WebKitWebDriver'`). It opens the actual Tauri window using
a temporary home directory and a recording `nmcli` stub. The screenshots
above use sample profiles and anonymized network identifiers.

## What is it?

Do you move between home, the office and a café, and need different settings in
each place? Network Manager lets you save everything a place needs (IP address,
DNS, proxy, VPN, printer and more) as a **profile**. Pick a profile and the app
applies it for you. It can even switch profiles on its own when you arrive
somewhere.

## What it can do

- **Network**: IP addresses (automatic or fixed), DNS servers, routes, Wi-Fi and VPN connections
- **System**: hostname, proxy, firewall zone, default printer, timezone
- **Automatic switching**: change profile based on the Wi-Fi network, your router, or the time of day
- **Scheduling**: activate a profile at a set time
- **Connection watchdog**: get notified, or reconnect, when the connection drops
- **System tray**: show an icon and switch profiles from the tray (GNOME needs the AppIndicator extension)
- **Scripts**: run your own scripts before or after a profile is applied, optionally sandboxed
- **Privacy**: your profiles are stored only on your computer, optionally encrypted with a passphrase
- **Dashboard**: see your current connection at a glance
- **Network scanner**: discover devices on your LAN with names, MAC vendors, open services, Wake-on-LAN, and CSV export
- **Port scanner**: list the open TCP ports of a device, by address or name or straight from the network scanner
- Follows your GNOME light or dark style

## Install on Fedora

1. Download the latest `.rpm` file from the [Releases page](https://github.com/christosdaggas/Network-Manager/releases).
2. Install it:

   ```bash
   sudo dnf install ./network-manager-*.rpm
   ```

3. Open **Network Manager** from your app grid.

To remove it later: `sudo dnf remove network-manager`

## Getting started

1. Open **Profiles** and click **New Profile**.
2. Give it a name (for example "Office") and add the settings you need.
3. Click **Apply** to switch to it.
4. Optional: add rules so the profile turns on by itself when you connect to a certain Wi-Fi network.

## Build it yourself

```bash
sudo dnf install cargo rust gtk4-devel libadwaita-devel glib2-devel gettext-devel rpm-build
cargo build --release
./target/release/network-manager
```

To build your own RPM package, run `./scripts/package-rpm.sh`. The package is saved in `dist/rpm/`.

## Updates and privacy

The app checks GitHub once at startup to see if a newer version is available
and shows a small notice if there is one. Connection checks ping 8.8.8.8 (the
watchdog target can be changed in Settings), and the optional speed test
downloads a file from Cloudflare. The network scanner sends pings and port
probes only to the address range you choose, and the port scanner connects
only to the device and ports you choose. The app does not collect or send
any personal data.

## Feedback and contributing

Found a bug or have an idea? [Open an issue](https://github.com/christosdaggas/Network-Manager/issues).
Want to help with the code? See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).

## Author

**Christos A. Daggas**: [chrisdaggas.com](https://chrisdaggas.com) · [info@chrisdaggas.com](mailto:info@chrisdaggas.com)
