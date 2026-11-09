// Network Manager - Network Scanner
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Local network discovery.
//!
//! A scan sweeps an IPv4 range in parallel. Hosts are detected with ICMP echo
//! (via the system `ping`), the kernel neighbour table, and optionally TCP
//! connection attempts to common service ports. Names come from reverse DNS,
//! MAC addresses from the neighbour table, and vendors from the system IEEE
//! OUI database with a small built-in fallback.

use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Largest range a single scan accepts (a /22).
pub const MAX_TARGETS: usize = 1024;

const WORKER_LIMIT: usize = 64;
const PING_TIMEOUT_SECS: u32 = 1;
const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(150);
const REVERSE_DNS_TIMEOUT_SECS: u32 = 1;

/// Ports probed in thorough mode (and when ping is unavailable).
pub const COMMON_PORTS: &[u16] = &[
    22, 53, 80, 443, 445, 631, 3306, 3389, 5432, 8080, 8123, 9090, 32400,
];

/// How hard a scan works to find a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// ICMP and neighbour-table discovery only.
    Fast,
    /// Also try common TCP ports, which finds hosts that block ICMP.
    Thorough,
}

/// Why a range string could not be turned into scan targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    Empty,
    Invalid,
    TooMany(usize),
}

/// One scanned address.
#[derive(Debug, Clone)]
pub struct HostResult {
    pub ip: Ipv4Addr,
    pub online: bool,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    /// ICMP round-trip time in milliseconds, when a ping reply arrived.
    pub latency_ms: Option<f64>,
    /// Service ports that accepted a TCP connection.
    pub open_ports: Vec<u16>,
}

/// Messages sent from the scan workers to the UI.
#[derive(Debug)]
pub enum ScanEvent {
    Host(HostResult),
    Progress {
        completed: usize,
        total: usize,
    },
    /// Every worker has stopped; no further events follow.
    Finished,
}

/// A running scan: cancel it with the flag, consume events from the receiver.
pub struct ScanHandle {
    pub cancel: Arc<AtomicBool>,
    pub events: Receiver<ScanEvent>,
}

/// Parse a CIDR block, a full range, a last-octet range, or a single address.
///
/// Accepted forms:
/// - `192.168.1.0/24`
/// - `192.168.1.1-192.168.1.254`
/// - `192.168.1.1-254`
/// - `192.168.1.10`
pub fn parse_targets(input: &str) -> Result<Vec<Ipv4Addr>, RangeError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(RangeError::Empty);
    }

    if let Some((base, prefix)) = input.split_once('/') {
        let base: Ipv4Addr = base.trim().parse().map_err(|_| RangeError::Invalid)?;
        let prefix: u32 = prefix.trim().parse().map_err(|_| RangeError::Invalid)?;
        if prefix > 32 {
            return Err(RangeError::Invalid);
        }

        let total = if prefix == 32 {
            1u64
        } else {
            1u64 << (32 - prefix)
        };
        if total > MAX_TARGETS as u64 {
            return Err(RangeError::TooMany(total as usize));
        }

        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        let network = u32::from(base) & mask;

        // A /24 and wider contain network and broadcast addresses that no
        // host can answer on; skip them when the block is wide enough.
        let (first, last) = if prefix <= 30 {
            (network + 1, network + total as u32 - 2)
        } else {
            (network, network + total as u32 - 1)
        };

        return Ok((first..=last).map(Ipv4Addr::from).collect());
    }

    if let Some((start, end)) = input.split_once('-') {
        let start: Ipv4Addr = start.trim().parse().map_err(|_| RangeError::Invalid)?;

        // `192.168.1.10-50` is shorthand for the last octet.
        let end = if end.trim().contains('.') {
            end.trim()
                .parse::<Ipv4Addr>()
                .map_err(|_| RangeError::Invalid)?
        } else {
            let last: u8 = end.trim().parse().map_err(|_| RangeError::Invalid)?;
            let [a, b, c, _] = start.octets();
            Ipv4Addr::new(a, b, c, last)
        };

        if start > end {
            return Err(RangeError::Invalid);
        }

        let count = u32::from(end) - u32::from(start) + 1;
        if count as usize > MAX_TARGETS {
            return Err(RangeError::TooMany(count as usize));
        }

        return Ok((u32::from(start)..=u32::from(end))
            .map(Ipv4Addr::from)
            .collect());
    }

    let ip: Ipv4Addr = input.parse().map_err(|_| RangeError::Invalid)?;
    Ok(vec![ip])
}

/// The local IPv4 range in CIDR form, e.g. `192.168.1.0/24`.
pub fn detect_local_range() -> Option<String> {
    let output = std::process::Command::new("ip")
        .env("LC_ALL", "C")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some(rest) = line.split_once(':').map(|(_, rest)| rest.trim()) else {
            continue;
        };
        let iface = rest.split_whitespace().next().unwrap_or_default();
        if iface.is_empty() || is_virtual_interface(iface) {
            continue;
        }
        let Some(inet) = line.split_whitespace().skip_while(|t| *t != "inet").nth(1) else {
            continue;
        };
        let Some((address, prefix)) = inet.split_once('/') else {
            continue;
        };
        let Ok(address) = address.parse::<Ipv4Addr>() else {
            continue;
        };
        let Ok(prefix) = prefix.parse::<u32>() else {
            continue;
        };
        if address.is_loopback() || address.is_link_local() || prefix == 32 {
            continue;
        }

        // Never auto-select a range larger than the scanner accepts.
        let prefix = prefix.clamp(22, 30);
        let mask = u32::MAX << (32 - prefix);
        let network = Ipv4Addr::from(u32::from(address) & mask);
        return Some(format!("{}/{}", network, prefix));
    }

    None
}

fn is_virtual_interface(name: &str) -> bool {
    name.starts_with("docker")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("br-")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("vnet")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
}

/// Start a scan in the background.
///
/// Returns immediately. Host results and progress arrive on
/// [`ScanHandle::events`]; the final [`ScanEvent::Finished`] is always sent,
/// including after cancellation.
pub fn start_scan(targets: Vec<Ipv4Addr>, mode: ScanMode) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let total = targets.len();

    let queue = Arc::new(Mutex::new(VecDeque::from(targets)));
    let completed = Arc::new(AtomicUsize::new(0));
    let worker_count = total.min(WORKER_LIMIT).max(1);
    // A present binary is not enough: unprivileged ICMP may be denied. Probe
    // loopback once so fallback port discovery kicks in when ping is unusable.
    let ping_available = ping(Ipv4Addr::LOCALHOST).is_some();

    let manager_cancel = cancel.clone();
    let manager_tx = tx.clone();
    std::thread::Builder::new()
        .name("network-scanner".to_string())
        .spawn(move || {
            let mut workers = Vec::with_capacity(worker_count);
            for _ in 0..worker_count {
                let queue = queue.clone();
                let completed = completed.clone();
                let cancel = manager_cancel.clone();
                let tx = manager_tx.clone();
                workers.push(std::thread::spawn(move || loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(ip) = queue.lock().ok().and_then(|mut q| q.pop_front()) else {
                        break;
                    };

                    let result = scan_host(ip, mode, ping_available, &cancel);
                    let cancelled = cancel.load(Ordering::Relaxed);
                    if tx.send(ScanEvent::Host(result)).is_err() {
                        break;
                    }

                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if tx
                        .send(ScanEvent::Progress {
                            completed: done,
                            total,
                        })
                        .is_err()
                    {
                        break;
                    }
                    if cancelled {
                        break;
                    }
                }));
            }

            for worker in workers {
                let _ = worker.join();
            }
            let _ = manager_tx.send(ScanEvent::Finished);
        })
        .ok();

    ScanHandle { cancel, events: rx }
}

fn scan_host(
    ip: Ipv4Addr,
    mode: ScanMode,
    ping_available: bool,
    cancel: &AtomicBool,
) -> HostResult {
    let (latency, ping_reply) = if ping_available {
        match ping(ip) {
            Some(latency) => (Some(latency), true),
            None => (None, false),
        }
    } else {
        (None, false)
    };

    let (mac, neighbour_alive) = neighbour_entry(ip);
    let mut online = ping_reply || neighbour_alive;
    let mut open_ports = Vec::new();

    // Thorough mode probes every host so the services column is filled in.
    // Without a working ping, use the same probe as a discovery fallback.
    let probe_ports = mode == ScanMode::Thorough || !ping_available;
    if probe_ports && !cancel.load(Ordering::Relaxed) {
        for &port in COMMON_PORTS {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if tcp_probe(ip, port) {
                open_ports.push(port);
                online = true;
            }
        }
    }

    let hostname = if online { reverse_dns(ip) } else { None };
    let vendor = mac.as_deref().and_then(lookup_vendor);

    HostResult {
        ip,
        online,
        hostname,
        mac,
        vendor,
        latency_ms: latency,
        open_ports,
    }
}

/// Ping once and return the reply time in milliseconds.
fn ping(ip: Ipv4Addr) -> Option<f64> {
    let started = Instant::now();
    let output = std::process::Command::new("ping")
        .env("LC_ALL", "C")
        .args([
            "-n",
            "-c",
            "1",
            "-W",
            &PING_TIMEOUT_SECS.to_string(),
            &ip.to_string(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let rtt = stdout
        .split("time=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse::<f64>().ok());
    Some(rtt.unwrap_or_else(|| started.elapsed().as_secs_f64() * 1000.0))
}

/// Look up the neighbour table entry for a host.
///
/// Returns the MAC address and whether the entry proves the host is reachable.
/// A stale entry is kept for the MAC but does not count as proof of life.
fn neighbour_entry(ip: Ipv4Addr) -> (Option<String>, bool) {
    let output = std::process::Command::new("ip")
        .env("LC_ALL", "C")
        .args(["neigh", "show", &ip.to_string()])
        .output();
    let Ok(output) = output else {
        return (None, false);
    };
    if !output.status.success() {
        return (None, false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let mac = tokens
            .iter()
            .position(|t| *t == "lladdr")
            .and_then(|index| tokens.get(index + 1))
            .map(|mac| mac.to_uppercase())
            .filter(|mac| is_valid_mac(mac));
        let Some(mac) = mac else {
            continue;
        };
        let state = tokens.last().copied().unwrap_or_default();
        let reachable = matches!(state, "REACHABLE" | "DELAY" | "PROBE");
        return (Some(mac), reachable);
    }

    (None, false)
}

fn is_valid_mac(mac: &str) -> bool {
    mac.len() == 17
        && mac
            .split(':')
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
        && mac != "00:00:00:00:00:00"
}

/// Try a TCP connection. A refusal still proves the host is up.
fn tcp_probe(ip: Ipv4Addr, port: u16) -> bool {
    let address = SocketAddr::new(IpAddr::V4(ip), port);
    match TcpStream::connect_timeout(&address, TCP_PROBE_TIMEOUT) {
        Ok(_) => true,
        Err(error) => error.kind() == std::io::ErrorKind::ConnectionRefused,
    }
}

/// Reverse-resolve a host using the system resolver, with a hard timeout.
fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    let output = std::process::Command::new("timeout")
        .env("LC_ALL", "C")
        .args([
            &REVERSE_DNS_TIMEOUT_SECS.to_string(),
            "getent",
            "hosts",
            &ip.to_string(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        for name in line.split_whitespace().skip(1) {
            if name.parse::<IpAddr>().is_err() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Return the vendor for a MAC address, if a database entry exists.
pub fn lookup_vendor(mac: &str) -> Option<String> {
    let prefix: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect::<String>()
        .to_uppercase();
    if prefix.len() != 6 {
        return None;
    }

    if let Some(vendor) = BUILTIN_OUI.get(prefix.as_str()) {
        return Some((*vendor).to_string());
    }
    SYSTEM_OUI.get(&prefix).cloned()
}

/// A handful of common vendors so the column is useful without the optional
/// IEEE database package.
static BUILTIN_OUI: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    HashMap::from([
        ("B827EB", "Raspberry Pi Foundation"),
        ("DCA632", "Raspberry Pi Trading"),
        ("E45F01", "Raspberry Pi Trading"),
        ("D83ADD", "Raspberry Pi Trading"),
        ("001C42", "Parallels"),
        ("005056", "VMware"),
        ("000C29", "VMware"),
        ("000569", "VMware"),
        ("001C14", "VMware"),
        ("080027", "VirtualBox"),
        ("00155D", "Microsoft (Hyper-V)"),
        ("60706C", "Google"),
        ("842859", "Amazon"),
        ("34F716", "TP-Link"),
        ("F09FC2", "Ubiquiti"),
        ("D48AFC", "Espressif"),
        ("F0EE7A", "Apple"),
        ("58AD12", "Apple"),
        ("60FDA6", "Apple"),
    ])
});

/// Vendors from the system IEEE OUI database, loaded on first use.
static SYSTEM_OUI: Lazy<HashMap<String, String>> = Lazy::new(load_system_oui);

fn load_system_oui() -> HashMap<String, String> {
    const DATABASES: &[&str] = &[
        "/usr/share/ieee-data/oui.txt",
        "/usr/share/hwdata/oui.txt",
        "/usr/share/misc/oui.txt",
    ];

    let Some(contents) = DATABASES
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
    else {
        return HashMap::new();
    };

    // Formats seen in the wild:
    //   00-00-0C   (hex)        Cisco Systems, Inc
    //   00-00-0C   Cisco Systems, Inc
    let mut vendors = HashMap::new();
    for line in contents.lines() {
        let mut tokens = line.split_whitespace();
        let Some(prefix) = tokens.next() else {
            continue;
        };
        let prefix: String = prefix.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if prefix.len() != 6 {
            continue;
        }
        let rest: Vec<&str> = tokens.collect();
        let vendor = if rest.first() == Some(&"(hex)") {
            rest[1..].join(" ")
        } else {
            rest.join(" ")
        };
        if !vendor.is_empty() {
            vendors
                .entry(prefix.to_uppercase())
                .or_insert_with(|| vendor.to_string());
        }
    }
    vendors
}

/// Send a Wake-on-LAN magic packet for `mac`.
pub fn send_wake_on_lan(mac: &str) -> Result<(), String> {
    let cleaned: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.len() != 12 {
        return Err("Invalid MAC address".to_string());
    }
    let bytes: Vec<u8> = (0..6)
        .map(|i| u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid MAC address".to_string())?;

    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&bytes);
    }

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    // Port 9 is the canonical WOL port; 7 is accepted by older devices.
    socket
        .send_to(&packet, "255.255.255.255:9")
        .map_err(|e| e.to_string())?;
    let _ = socket.send_to(&packet, "255.255.255.255:7");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cidr_and_skips_network_and_broadcast() {
        let targets = parse_targets("192.168.1.0/30").unwrap();
        assert_eq!(
            targets,
            vec![Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(192, 168, 1, 2)]
        );

        let targets = parse_targets("192.168.1.0/24").unwrap();
        assert_eq!(targets.len(), 254);
        assert_eq!(targets.first(), Some(&Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(targets.last(), Some(&Ipv4Addr::new(192, 168, 1, 254)));
    }

    #[test]
    fn parses_last_octet_range() {
        let targets = parse_targets("192.168.1.10-12").unwrap();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0], Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(targets[2], Ipv4Addr::new(192, 168, 1, 12));
    }

    #[test]
    fn parses_full_range() {
        let targets = parse_targets("10.0.0.5-10.0.0.7").unwrap();
        assert_eq!(
            targets,
            vec![
                Ipv4Addr::new(10, 0, 0, 5),
                Ipv4Addr::new(10, 0, 0, 6),
                Ipv4Addr::new(10, 0, 0, 7)
            ]
        );
    }

    #[test]
    fn parses_single_address() {
        assert_eq!(
            parse_targets("10.0.0.1").unwrap(),
            vec![Ipv4Addr::new(10, 0, 0, 1)]
        );
    }

    #[test]
    fn rejects_invalid_ranges() {
        assert_eq!(parse_targets(""), Err(RangeError::Empty));
        assert_eq!(parse_targets("nope"), Err(RangeError::Invalid));
        assert_eq!(parse_targets("10.0.0.9-2"), Err(RangeError::Invalid));
        assert_eq!(
            parse_targets("10.0.0.0/8"),
            Err(RangeError::TooMany(16_777_216))
        );
    }

    #[test]
    fn finds_vendor_prefixes() {
        assert_eq!(
            lookup_vendor("B8:27:EB:12:34:56").as_deref(),
            Some("Raspberry Pi Foundation")
        );
        assert_eq!(lookup_vendor("not a mac"), None);
    }
}
