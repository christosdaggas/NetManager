// Network Manager - Profile execution
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Profile execution shared by the GTK and Tauri windows.

use crate::models::Profile;
use glib::variant::ToVariant;

/// Outcome of a single profile action that did not fail.
enum ActionOutcome {
    Applied,
    /// Nothing was changed (unsupported action type or nothing to act on).
    Skipped(String),
}

type ActionResult = std::result::Result<ActionOutcome, String>;

/// Summary of a profile apply, built off the UI thread.
#[derive(Default)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
    /// A pre-script failed, so the remaining actions were not run.
    pub aborted: bool,
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

/// Run a profile's actions in order: pre-scripts, network actions, system
/// actions, then post-scripts and programs. Runs on a worker thread.
pub fn execute_profile_actions(
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
        if !report.record(run_automation_action(action, sandbox_mode))
            && !continues_on_error(action)
        {
            report.aborted = true;
            return report;
        }
    }

    for action in &profile.network_actions {
        report.record(apply_network_action(action));
    }

    for action in &profile.system_actions {
        report.record(apply_system_action(action));
    }

    for action in post_actions {
        tracing::info!("Running automation action for profile '{}'", profile.name());
        if !report.record(run_automation_action(action, sandbox_mode))
            && !continues_on_error(action)
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
            let Some(conn_name) = get_connection_for_interface(iface) else {
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
            reapply_connection(iface, &conn_name)?;
            Ok(ActionOutcome::Applied)
        }

        NetworkAction::DnsServers { interface, servers } => {
            let iface = interface.as_deref().unwrap_or("eth0");

            // Only configure DNS if there's an actual NetworkManager connection
            let Some(conn_name) = get_connection_for_interface(iface) else {
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
            reapply_connection(iface, &conn_name)?;
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
            match device_state(&state.interface).as_deref() {
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
            let Some(conn_name) = get_connection_for_interface(interface) else {
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
            reapply_connection(interface, &conn_name)?;
            Ok(ActionOutcome::Applied)
        }

        NetworkAction::SetMacAddress {
            interface,
            mac_address,
        } => {
            let Some(conn_name) = get_connection_for_interface(interface) else {
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
            reapply_connection(interface, &conn_name)?;
            Ok(ActionOutcome::Applied)
        }

        NetworkAction::DnsSearchDomains { interface, domains } => {
            let iface = interface.as_deref().unwrap_or("eth0");
            let Some(conn_name) = get_connection_for_interface(iface) else {
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
            reapply_connection(iface, &conn_name)?;
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
                        parse_proxy_address(value)
                            .map(|(host, port)| (schema, host, port))
                            .ok_or_else(|| format!("Invalid proxy '{}': expected host:port", value))
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
