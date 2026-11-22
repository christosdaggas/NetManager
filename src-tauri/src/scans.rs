//! The two manual diagnostic tools share the GTK workers. Drain batches rather
//! than sending one IPC message per address or port.
use netmanager_core::{port_scanner as ports, scanner as lan};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Mutex};
use tauri::State;

#[derive(Default)]
pub struct Scans {
    network: Mutex<Option<lan::ScanHandle>>,
    ports: Mutex<Option<ports::PortScanHandle>>,
}

#[tauri::command]
pub async fn detect_range() -> Result<String, String> {
    lan::detect_local_range()
        .ok_or_else(|| "No local IPv4 range detected. Enter a range manually.".into())
}

#[tauri::command]
pub async fn scan_start(
    state: State<'_, Scans>,
    kind: String,
    target: String,
    mode: String,
    custom: String,
) -> Result<usize, String> {
    match kind.as_str() {
        "scanner" => {
            let targets = lan::parse_targets(&target).map_err(|e| match e {
                lan::RangeError::TooMany(_) => "Scan at most 1024 addresses at once.",
                _ => "Enter a valid IPv4 address, CIDR subnet, or address range.",
            })?;
            let mode = match mode.as_str() {
                "fast" => lan::ScanMode::Fast,
                "thorough" => lan::ScanMode::Thorough,
                _ => return Err("Unknown discovery mode.".into()),
            };
            let mut job = state.network.lock().map_err(|_| "Scanner unavailable.")?;
            if job.is_some() {
                return Err("A network scan is already running.".into());
            }
            let n = targets.len();
            *job = Some(lan::start_scan(targets, mode));
            Ok(n)
        }
        "ports" => {
            if !ports::is_valid_host(&target) {
                return Err("Enter a valid IP address or host name.".into());
            }
            let list = match mode.as_str() {
                "common" => ports::TOP_PORTS.to_vec(),
                "known" => ports::port_range(1024),
                "all" => ports::port_range(u16::MAX),
                "custom" => ports::parse_ports(&custom)
                    .map_err(|_| "Enter ports or ranges from 1 to 65535.")?,
                _ => return Err("Unknown port selection.".into()),
            };
            let mut job = state.ports.lock().map_err(|_| "Scanner unavailable.")?;
            if job.is_some() {
                return Err("A port scan is already running.".into());
            }
            let n = list.len();
            *job = Some(ports::start_scan(target, list));
            Ok(n)
        }
        _ => Err("Unknown scanner.".into()),
    }
}

#[tauri::command]
pub fn scan_stop(state: State<'_, Scans>, kind: String) {
    if kind == "scanner" {
        if let Ok(g) = state.network.lock() {
            if let Some(h) = g.as_ref() {
                h.cancel.store(true, Ordering::Relaxed);
            }
        }
    } else if let Ok(g) = state.ports.lock() {
        if let Some(h) = g.as_ref() {
            h.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[tauri::command]
pub async fn scan_events(state: State<'_, Scans>, kind: String) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut done = false;
    let mut progress = 0;
    let mut address = None;
    let mut failed = false;
    let mut summary = Value::Null;
    if kind == "scanner" {
        let mut g = state.network.lock().map_err(|_| "Scanner unavailable.")?;
        let h = g.as_ref().ok_or("Scan is no longer running.")?;
        for event in h.events.try_iter() {
            match event {
            lan::ScanEvent::Host(r)=>rows.push(json!({"ip":r.ip,"online":r.online,"hostname":r.hostname,"mac":r.mac,"vendor":r.vendor,"latency":r.latency_ms,"ports":r.open_ports})),
            lan::ScanEvent::Progress{completed,..}=>progress=progress.max(completed),
            lan::ScanEvent::Finished=>done=true,
        }
        }
        if done {
            g.take();
        }
    } else {
        let mut g = state.ports.lock().map_err(|_| "Scanner unavailable.")?;
        let h = g.as_ref().ok_or("Scan is no longer running.")?;
        for event in h.events.try_iter().take(4096) {
            match event {
            ports::PortScanEvent::Resolved(ip)=>address=Some(ip),
            ports::PortScanEvent::ResolveFailed=>failed=true,
            ports::PortScanEvent::Open(r)=>rows.push(json!({"port":r.port,"service":ports::service_name(r.port),"latency":r.latency_ms})),
            ports::PortScanEvent::Progress{completed,..}=>progress=progress.max(completed),
            ports::PortScanEvent::Finished(s)=>{done=true; progress=s.scanned; summary=json!({"closed":s.closed,"unanswered":s.unanswered});}
        }
        }
        if done {
            g.take();
        }
    }
    Ok(
        json!({ "rows":rows,"done":done,"progress":progress,"address":address,"failed":failed,"summary":summary }),
    )
}

#[tauri::command]
pub async fn wake_device(mac: String) -> Result<(), String> {
    lan::send_wake_on_lan(&mac)
}
