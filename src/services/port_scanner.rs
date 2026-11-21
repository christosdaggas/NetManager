// Network Manager - Port Scanner
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! TCP port scanning for a single host.
//!
//! A scan resolves the host once, then tries a plain TCP connection to each
//! requested port from a pool of worker threads. A port counts as open when
//! the connection is accepted; the connection is closed again straight away
//! and nothing is sent over it. Service names come from a built-in table of
//! familiar names, with the system's `/etc/services` as a fallback.

use once_cell::sync::Lazy;
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WORKER_LIMIT: usize = 128;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// The ports people most often want to know about, in ascending order.
pub const TOP_PORTS: &[u16] = &[
    20, 21, 22, 23, 25, 53, 80, 88, 110, 111, 135, 139, 143, 179, 389, 443, 445, 465, 515, 548,
    554, 587, 631, 636, 873, 993, 995, 1080, 1433, 1521, 1723, 1883, 2049, 2375, 2376, 3000, 3128,
    3306, 3389, 3689, 5000, 5060, 5222, 5432, 5672, 5900, 5985, 5986, 6379, 6443, 8000, 8008, 8080,
    8081, 8123, 8443, 8883, 8888, 9000, 9090, 9100, 9200, 9418, 10000, 11211, 25565, 27017, 32400,
    62078,
];

/// Last port of the well-known range offered as a preset.
pub const WELL_KNOWN_LAST: u16 = 1024;

/// Why a port list could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortListError {
    Empty,
    /// The part of the input that is not a port or a range of ports.
    Invalid(String),
}

/// A port that accepted a connection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenPort {
    pub port: u16,
    /// Time the connection took to open, in milliseconds.
    pub latency_ms: f64,
}

/// What happened to the ports that were tried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// Ports tried before the scan ended or was cancelled.
    pub scanned: usize,
    /// Ports the host actively refused, which shows the host is up.
    pub closed: usize,
    /// Ports that gave no answer in time.
    pub unanswered: usize,
}

/// Messages sent from the scan workers to the UI.
#[derive(Debug)]
pub enum PortScanEvent {
    /// The address the host name stands for; the scan starts after this.
    Resolved(IpAddr),
    /// The host name has no address; `Finished` follows.
    ResolveFailed,
    Open(OpenPort),
    Progress {
        completed: usize,
        total: usize,
    },
    /// Every worker has stopped; no further events follow.
    Finished(ScanSummary),
}

/// A running scan: cancel it with the flag, consume events from the receiver.
pub struct PortScanHandle {
    pub cancel: Arc<AtomicBool>,
    pub events: Receiver<PortScanEvent>,
}

/// Parse a list of ports and port ranges, such as `22, 80, 8000-8100`.
///
/// Items are separated by commas or spaces. The result is sorted and free of
/// duplicates.
pub fn parse_ports(input: &str) -> Result<Vec<u16>, PortListError> {
    let mut ports = BTreeSet::new();

    for item in input
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|item| !item.is_empty())
    {
        let invalid = || PortListError::Invalid(item.to_string());
        let parse = |text: &str| match text.parse::<u16>() {
            Ok(port) if port > 0 => Ok(port),
            _ => Err(invalid()),
        };

        match item.split_once('-') {
            Some((first, last)) => {
                let (first, last) = (parse(first)?, parse(last)?);
                if first > last {
                    return Err(invalid());
                }
                ports.extend(first..=last);
            }
            None => {
                ports.insert(parse(item)?);
            }
        }
    }

    if ports.is_empty() {
        return Err(PortListError::Empty);
    }
    Ok(ports.into_iter().collect())
}

/// Every port from 1 up to and including `last`.
pub fn port_range(last: u16) -> Vec<u16> {
    (1..=last).collect()
}

/// The default gateway's IPv4 address, usually the local router.
pub fn default_gateway() -> Option<String> {
    let output = std::process::Command::new("ip")
        .env("LC_ALL", "C")
        .args(["-4", "route", "show", "default"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_default_gateway(&String::from_utf8_lossy(&output.stdout))
}

fn parse_default_gateway(routes: &str) -> Option<String> {
    routes.lines().find_map(|line| {
        let gateway = line.split_whitespace().skip_while(|t| *t != "via").nth(1)?;
        gateway
            .parse::<std::net::Ipv4Addr>()
            .ok()
            .map(|ip| ip.to_string())
    })
}

/// Start a port scan in the background.
///
/// Returns immediately. Open ports and progress arrive on
/// [`PortScanHandle::events`]; the final [`PortScanEvent::Finished`] is
/// always sent, including after cancellation or a failed name lookup.
pub fn start_scan(host: String, ports: Vec<u16>) -> PortScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    let manager_cancel = cancel.clone();
    std::thread::Builder::new()
        .name("port-scanner".to_string())
        .spawn(move || {
            let Some(ip) = resolve_unless_cancelled(&host, &manager_cancel) else {
                // A cancelled lookup is not a failed one.
                if !manager_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(PortScanEvent::ResolveFailed);
                }
                let _ = tx.send(PortScanEvent::Finished(ScanSummary::default()));
                return;
            };
            if tx.send(PortScanEvent::Resolved(ip)).is_err() {
                return;
            }

            let total = ports.len();
            let ports = Arc::new(ports);
            let next = Arc::new(AtomicUsize::new(0));
            let completed = Arc::new(AtomicUsize::new(0));
            let closed = Arc::new(AtomicUsize::new(0));
            let unanswered = Arc::new(AtomicUsize::new(0));

            let worker_count = total.clamp(1, WORKER_LIMIT);
            let mut workers = Vec::with_capacity(worker_count);
            for _ in 0..worker_count {
                let ports = ports.clone();
                let next = next.clone();
                let completed = completed.clone();
                let closed = closed.clone();
                let unanswered = unanswered.clone();
                let cancel = manager_cancel.clone();
                let tx = tx.clone();
                workers.push(std::thread::spawn(move || loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(&port) = ports.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };

                    match probe(ip, port) {
                        Probe::Open(latency_ms) => {
                            let open = OpenPort { port, latency_ms };
                            if tx.send(PortScanEvent::Open(open)).is_err() {
                                break;
                            }
                        }
                        Probe::Closed => {
                            closed.fetch_add(1, Ordering::Relaxed);
                        }
                        Probe::Unanswered => {
                            unanswered.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if tx
                        .send(PortScanEvent::Progress {
                            completed: done,
                            total,
                        })
                        .is_err()
                    {
                        break;
                    }
                }));
            }

            for worker in workers {
                let _ = worker.join();
            }

            let _ = tx.send(PortScanEvent::Finished(ScanSummary {
                scanned: completed.load(Ordering::Relaxed),
                closed: closed.load(Ordering::Relaxed),
                unanswered: unanswered.load(Ordering::Relaxed),
            }));
        })
        .ok();

    PortScanHandle { cancel, events: rx }
}

/// Whether `host` is an IP address or could be a host name.
///
/// Catches typos such as spaces or commas before the resolver is asked,
/// which can take seconds to give up on them.
pub fn is_valid_host(host: &str) -> bool {
    let host = host.trim();
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    let name = host.strip_suffix('.').unwrap_or(host);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Resolve `host`, giving up early when the scan is cancelled.
///
/// The system resolver cannot be interrupted, so it runs on a thread of its
/// own, which is left to finish by itself if the scan is cancelled first.
fn resolve_unless_cancelled(host: &str, cancel: &AtomicBool) -> Option<IpAddr> {
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return Some(ip);
    }

    let (tx, rx) = mpsc::channel();
    let name = host.to_string();
    std::thread::Builder::new()
        .name("port-scanner-lookup".to_string())
        .spawn(move || {
            let _ = tx.send(resolve(&name));
        })
        .ok()?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(address) => return address,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Turn an address or a host name into the address to scan, preferring IPv4.
fn resolve(host: &str) -> Option<IpAddr> {
    let host = host.trim();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let addresses: Vec<IpAddr> = (host, 0)
        .to_socket_addrs()
        .ok()?
        .map(|address| address.ip())
        .collect();
    addresses
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
}

enum Probe {
    /// Connected; holds the time it took in milliseconds.
    Open(f64),
    Closed,
    Unanswered,
}

fn probe(ip: IpAddr, port: u16) -> Probe {
    let started = Instant::now();
    match TcpStream::connect_timeout(&SocketAddr::new(ip, port), CONNECT_TIMEOUT) {
        Ok(_) => Probe::Open(started.elapsed().as_secs_f64() * 1000.0),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => Probe::Closed,
        Err(_) => Probe::Unanswered,
    }
}

/// Name of the service usually found on a TCP port.
pub fn service_name(port: u16) -> Option<String> {
    known_service(port)
        .map(str::to_string)
        .or_else(|| SYSTEM_SERVICES.get(&port).cloned())
}

/// Familiar names for the ports people run into most.
pub fn known_service(port: u16) -> Option<&'static str> {
    Some(match port {
        20 => "FTP data",
        21 => "FTP",
        22 => "SSH",
        23 => "Telnet",
        25 => "SMTP",
        53 => "DNS",
        80 => "HTTP",
        88 => "Kerberos",
        110 => "POP3",
        111 => "RPC",
        135 => "Windows RPC",
        139 => "NetBIOS",
        143 => "IMAP",
        179 => "BGP",
        389 => "LDAP",
        443 => "HTTPS",
        445 => "SMB",
        465 => "SMTPS",
        515 => "LPD printing",
        548 => "AFP",
        554 => "RTSP",
        587 => "SMTP submission",
        631 => "IPP",
        636 => "LDAPS",
        873 => "rsync",
        993 => "IMAPS",
        995 => "POP3S",
        1080 => "SOCKS proxy",
        1433 => "Microsoft SQL Server",
        1521 => "Oracle Database",
        1723 => "PPTP",
        1883 => "MQTT",
        2049 => "NFS",
        2375 => "Docker",
        2376 => "Docker (TLS)",
        3128 => "HTTP proxy",
        3306 => "MySQL",
        3389 => "RDP",
        3689 => "DAAP",
        5060 => "SIP",
        5222 => "XMPP",
        5432 => "PostgreSQL",
        5672 => "AMQP",
        5900 => "VNC",
        5985 => "WinRM",
        5986 => "WinRM (HTTPS)",
        6379 => "Redis",
        6443 => "Kubernetes API",
        8000 | 8008 | 8080 | 8081 | 8888 => "HTTP",
        8123 => "Home Assistant",
        8443 => "HTTPS",
        8883 => "MQTT (TLS)",
        9090 => "Cockpit",
        9100 => "Printer (JetDirect)",
        9200 => "Elasticsearch",
        9418 => "Git",
        10000 => "Webmin",
        11211 => "Memcached",
        25565 => "Minecraft",
        27017 => "MongoDB",
        32400 => "Plex",
        62078 => "iPhone sync",
        _ => return None,
    })
}

static SYSTEM_SERVICES: Lazy<HashMap<u16, String>> = Lazy::new(|| {
    std::fs::read_to_string("/etc/services")
        .map(|text| parse_services(&text))
        .unwrap_or_default()
});

/// Read the TCP entries of a services file; the first name of a port wins.
fn parse_services(text: &str) -> HashMap<u16, String> {
    let mut services = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default();
        let mut fields = line.split_whitespace();
        let (Some(name), Some(port)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Some((port, "tcp")) = port.split_once('/') else {
            continue;
        };
        if let Ok(port) = port.parse::<u16>() {
            services.entry(port).or_insert_with(|| name.to_string());
        }
    }
    services
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn parses_ports_and_ranges() {
        assert_eq!(parse_ports("22"), Ok(vec![22]));
        assert_eq!(parse_ports("80, 22,443"), Ok(vec![22, 80, 443]));
        assert_eq!(parse_ports("8000-8002 22"), Ok(vec![22, 8000, 8001, 8002]));
        assert_eq!(parse_ports("1-65535").map(|ports| ports.len()), Ok(65535));
    }

    #[test]
    fn merges_overlapping_ports() {
        assert_eq!(parse_ports("80,80,79-81"), Ok(vec![79, 80, 81]));
    }

    #[test]
    fn rejects_invalid_port_lists() {
        assert_eq!(parse_ports(""), Err(PortListError::Empty));
        assert_eq!(parse_ports(" , "), Err(PortListError::Empty));
        for bad in ["0", "65536", "http", "90-80", "1-", "-5", "1-2-3"] {
            assert_eq!(
                parse_ports(bad),
                Err(PortListError::Invalid(bad.to_string())),
                "{bad}"
            );
        }
        assert_eq!(
            parse_ports("22, x"),
            Err(PortListError::Invalid("x".to_string()))
        );
    }

    #[test]
    fn presets_are_sorted_and_unique() {
        assert!(TOP_PORTS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(port_range(WELL_KNOWN_LAST).len(), 1024);
        assert_eq!(port_range(u16::MAX).last(), Some(&65535));
    }

    #[test]
    fn names_services() {
        assert_eq!(known_service(22), Some("SSH"));
        assert_eq!(known_service(8080), Some("HTTP"));
        assert_eq!(known_service(4), None);

        let services =
            parse_services("# comment\nssh 22/tcp # remote login\nssh 22/udp\ndomain 53/tcp dns\n");
        assert_eq!(services.get(&22).map(String::as_str), Some("ssh"));
        assert_eq!(services.get(&53).map(String::as_str), Some("domain"));
        assert_eq!(services.len(), 2);
    }

    #[test]
    fn reads_the_default_gateway() {
        let routes = "default via 192.168.1.1 dev wlan0 proto dhcp src 192.168.1.20 metric 600\n";
        assert_eq!(
            parse_default_gateway(routes),
            Some("192.168.1.1".to_string())
        );
        assert_eq!(parse_default_gateway("default dev ppp0 scope link\n"), None);
        assert_eq!(parse_default_gateway(""), None);
    }

    #[test]
    fn finds_an_open_port_and_skips_a_closed_one() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let open = listener.local_addr().expect("listener address").port();
        // A port that was just released is closed again.
        let closed = {
            let other = TcpListener::bind("127.0.0.1:0").expect("bind a second port");
            other.local_addr().expect("second address").port()
        };

        let handle = start_scan("127.0.0.1".to_string(), vec![open, closed]);
        let mut found = Vec::new();
        let mut summary = None;
        for event in handle.events.iter() {
            match event {
                PortScanEvent::Open(port) => found.push(port.port),
                PortScanEvent::Finished(result) => summary = Some(result),
                PortScanEvent::ResolveFailed => panic!("loopback must resolve"),
                PortScanEvent::Resolved(_) | PortScanEvent::Progress { .. } => {}
            }
        }

        assert_eq!(found, vec![open]);
        let summary = summary.expect("a finished event");
        assert_eq!(summary.scanned, 2);
        assert_eq!(summary.closed, 1);
    }

    #[test]
    fn tells_hosts_from_typos() {
        for good in [
            "192.168.1.1",
            "::1",
            "fe80::1",
            "router",
            "printer.local",
            "my-nas.home.arpa.",
            " example.com ",
        ] {
            assert!(is_valid_host(good), "{good}");
        }
        for bad in [
            "",
            " ",
            "two words",
            "10.0.0.1,443",
            "host:80",
            "-dash.example",
            "a..b",
            ".",
        ] {
            assert!(!is_valid_host(bad), "{bad}");
        }
    }

    #[test]
    fn a_cancelled_lookup_is_not_a_failure() {
        let cancel = AtomicBool::new(true);
        // No address, whatever the resolver would have said.
        assert_eq!(resolve_unless_cancelled("localhost", &cancel), None);
        // Addresses need no lookup, so they resolve even then.
        assert_eq!(
            resolve_unless_cancelled("127.0.0.1", &cancel),
            Some(IpAddr::from([127, 0, 0, 1]))
        );
    }

    #[test]
    fn reports_a_host_that_does_not_resolve() {
        // `.invalid` is reserved and never resolves.
        let handle = start_scan("no-such-host.invalid".to_string(), vec![80]);
        let events: Vec<PortScanEvent> = handle.events.iter().collect();
        assert!(matches!(events.first(), Some(PortScanEvent::ResolveFailed)));
        assert!(matches!(events.last(), Some(PortScanEvent::Finished(_))));
    }
}
