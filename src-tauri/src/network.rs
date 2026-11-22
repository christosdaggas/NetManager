use netmanager_core::{models::AdapterInfo, network_utils::detect_network_adapters};
use serde::Serialize;
use serde_json::Value;
use std::{fs, process::Command};

#[derive(Serialize)]
pub struct Snapshot {
    host: String,
    adapters: Vec<AdapterInfo>,
    primary: Option<String>,
    address: Option<String>,
    gateway: Option<String>,
    dns: Vec<String>,
}

fn ip(args: &[&str]) -> Vec<Value> {
    Command::new("ip")
        .arg("-j")
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| serde_json::from_slice(&out.stdout).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn network_snapshot() -> Snapshot {
    let adapters = detect_network_adapters();
    let route = ip(&["-4", "route", "show", "default"])
        .into_iter()
        .min_by_key(|r| r["metric"].as_u64().unwrap_or(0));
    let field = |key: &str| route.as_ref()?.get(key)?.as_str().map(String::from);
    let primary = field("dev").or_else(|| {
        adapters
            .iter()
            .find(|a| a.is_connected)
            .map(|a| a.name.clone())
    });
    let address = primary.as_deref().and_then(|name| {
        ip(&["-4", "address", "show", "dev", name])
            .first()?
            .get("addr_info")?
            .as_array()?
            .iter()
            .find(|a| a["scope"] == "global")?["local"]
            .as_str()
            .map(String::from)
    });
    let dns = fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next()? == "nameserver")
                .then(|| fields.next().map(String::from))
                .flatten()
        })
        .collect();
    Snapshot {
        host: fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_default()
            .trim()
            .into(),
        adapters,
        primary,
        address,
        gateway: field("gateway"),
        dns,
    }
}

#[derive(Serialize)]
pub struct Counters {
    interface: String,
    rx: u64,
    tx: u64,
}

#[tauri::command]
pub async fn traffic_counters() -> Result<Vec<Counters>, String> {
    let raw = fs::read_to_string("/proc/net/dev").map_err(|e| e.to_string())?;
    Ok(raw
        .lines()
        .filter_map(|line| {
            let (name, values) = line.split_once(':')?;
            let fields: Vec<_> = values.split_whitespace().collect();
            Some(Counters {
                interface: name.trim().into(),
                rx: fields.first()?.parse().ok()?,
                tx: fields.get(8)?.parse().ok()?,
            })
        })
        .collect())
}
