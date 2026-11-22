//! IPC deals in summaries and a small editor DTO. Executable actions never
//! come from the webview; apply resolves an existing profile from disk by ID.
use chrono::Utc;
use netmanager_core::{
    models::{AppConfig, Ipv4Address, Ipv4Method, NetworkAction, Profile},
    profile_apply::execute_profile_actions,
    storage::{DataStore, LogEntry},
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Mutex, MutexGuard},
};
use tauri::State;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct Library(Mutex<Option<Zeroizing<String>>>);

impl Library {
    fn guard(&self) -> Result<MutexGuard<'_, Option<Zeroizing<String>>>, String> {
        self.0
            .try_lock()
            .map_err(|_| "Another profile operation is in progress. Try again shortly.".into())
    }
}

// The shared activity log is plaintext, including when the profile cache is
// encrypted. Keep names, script paths and command output out of it in that mode.
fn log(store: &DataStore, cfg: &AppConfig, level: &str, detail: &str, private: &str) {
    store.append_log(
        level,
        if cfg.encrypt_profiles {
            private
        } else {
            detail
        },
    );
}

fn config(store: &DataStore, key: Option<&String>) -> Result<AppConfig, String> {
    let path = store.config_dir().join("settings.toml");
    let mut cfg = if path.try_exists().map_err(|e| e.to_string())? {
        AppConfig::load_from_file(&path).map_err(|e| e.to_string())?
    } else {
        AppConfig::default()
    };
    cfg.encrypt_profiles |= store.has_encrypted_cache();
    cfg.encryption_key = key.cloned();
    Ok(cfg)
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Connection {
    interface: String,
    method: String,
    address: String,
    gateway: String,
    dns: String,
}

impl Connection {
    // Only offer the compact editor when all network settings fit in it.
    // Advanced profiles can still be renamed and applied without data loss.
    fn from_profile(p: &Profile) -> Option<Self> {
        let mut c = Self {
            method: "unchanged".into(),
            ..Self::default()
        };
        let mut ipv4 = false;
        let mut dns = false;
        for a in &p.network_actions {
            let interface = match a {
                NetworkAction::Ipv4Config {
                    interface,
                    method,
                    addresses,
                    gateway,
                } if !ipv4 => {
                    ipv4 = true;
                    c.method = match method {
                        Ipv4Method::Auto if addresses.is_empty() && gateway.is_none() => "auto",
                        Ipv4Method::Manual if addresses.len() == 1 => "manual",
                        _ => return None,
                    }
                    .into();
                    if let Some(a) = addresses.first() {
                        c.address = format!("{}/{}", a.address, a.prefix);
                    }
                    c.gateway = gateway.map(|g| g.to_string()).unwrap_or_default();
                    interface
                }
                NetworkAction::DnsServers { interface, servers } if !dns && !servers.is_empty() => {
                    dns = true;
                    c.dns = servers
                        .iter()
                        .map(IpAddr::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    interface
                }
                _ => return None,
            };
            let name = interface.as_ref()?;
            if !c.interface.is_empty() && c.interface != *name {
                return None;
            }
            c.interface = name.clone();
        }
        Some(c)
    }

    fn actions(&self) -> Result<Vec<NetworkAction>, String> {
        let mut actions = Vec::new();
        let dev = self.interface.trim();
        let interface = Some(dev.to_string());
        let needs_device = self.method != "unchanged" || !self.dns.trim().is_empty();
        let valid_device = !dev.is_empty()
            && dev.len() <= 15
            && !dev.starts_with('-')
            && dev
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
        if needs_device && !valid_device {
            return Err("Choose a valid network interface.".into());
        }
        match self.method.as_str() {
            "auto" => actions.push(NetworkAction::Ipv4Config {
                interface: interface.clone(),
                method: Ipv4Method::Auto,
                addresses: vec![],
                gateway: None,
            }),
            "manual" => {
                let (addr, bits) = self
                    .address
                    .trim()
                    .split_once('/')
                    .ok_or("Use address/prefix, for example 192.168.1.20/24.")?;
                let address = addr
                    .parse::<Ipv4Addr>()
                    .map_err(|_| "Invalid IPv4 address.")?;
                let prefix = bits
                    .parse::<u8>()
                    .ok()
                    .filter(|p| *p <= 32)
                    .ok_or("Prefix must be 0–32.")?;
                let gateway = (!self.gateway.trim().is_empty())
                    .then(|| self.gateway.trim().parse::<Ipv4Addr>())
                    .transpose()
                    .map_err(|_| "Invalid IPv4 gateway.")?;
                actions.push(NetworkAction::Ipv4Config {
                    interface: interface.clone(),
                    method: Ipv4Method::Manual,
                    addresses: vec![Ipv4Address { address, prefix }],
                    gateway,
                });
            }
            "unchanged" => (),
            _ => return Err("Unknown IPv4 mode.".into()),
        }
        let servers = self
            .dns
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<IpAddr>()
                    .map_err(|_| format!("Invalid DNS address: {s}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !servers.is_empty() {
            actions.push(NetworkAction::DnsServers { interface, servers });
        }
        Ok(actions)
    }
}

#[derive(Serialize)]
pub struct Summary {
    id: String,
    name: String,
    description: String,
    revision: String,
    applied_at: Option<String>,
    actions: Vec<String>,
    connection: Option<Connection>,
    automation: bool,
}
impl From<&Profile> for Summary {
    fn from(p: &Profile) -> Self {
        Self {
            id: p.id().to_string(),
            name: p.name().into(),
            description: p.metadata.description.clone().unwrap_or_default(),
            revision: p.metadata.updated_at.to_rfc3339(),
            applied_at: p.metadata.last_applied_at.map(|t| t.to_rfc3339()),
            actions: p.all_actions().map(|a| a.description()).collect(),
            connection: Connection::from_profile(p),
            automation: !p.automation_actions.is_empty(),
        }
    }
}

#[derive(Serialize)]
pub struct LibraryView {
    profiles: Vec<Summary>,
    locked: bool,
    logs: Vec<LogEntry>,
}

#[tauri::command]
pub async fn library_view(state: State<'_, Library>) -> Result<LibraryView, String> {
    let key = state.guard()?;
    let store = DataStore::new();
    let cfg = config(&store, key.as_deref())?;
    let locked = cfg.encrypt_profiles && key.is_none();
    let mut profiles = if locked {
        vec![]
    } else {
        store.read_profiles(&cfg)?
    };
    profiles.sort_by_key(|p| p.name().to_lowercase());
    Ok(LibraryView {
        profiles: profiles.iter().map(Summary::from).collect(),
        locked,
        logs: store.logs().into_iter().rev().take(500).collect(),
    })
}

#[tauri::command]
pub async fn unlock_profiles(state: State<'_, Library>, passphrase: String) -> Result<(), String> {
    let mut key = state.guard()?;
    let passphrase = Zeroizing::new(passphrase);
    let store = DataStore::new();
    let cfg = config(&store, Some(&passphrase))?;
    if !store.verify_encryption_passphrase(&passphrase, cfg.encryption_check.as_deref()) {
        return Err("The passphrase did not unlock this library.".into());
    }
    store.read_profiles(&cfg)?;
    *key = Some(passphrase);
    Ok(())
}

#[derive(Deserialize)]
pub struct Draft {
    id: Option<String>,
    revision: Option<String>,
    name: String,
    description: String,
    connection: Option<Connection>,
}

fn find(profiles: &[Profile], id: &str, revision: &str) -> Result<usize, String> {
    let i = profiles
        .iter()
        .position(|p| p.id().to_string() == id)
        .ok_or("Profile no longer exists. Refresh the library.")?;
    if profiles[i].metadata.updated_at.to_rfc3339() != revision {
        return Err("Profile changed since you opened it. Refresh before continuing.".into());
    }
    Ok(i)
}

fn open(key: Option<&String>) -> Result<(DataStore, AppConfig, Vec<Profile>), String> {
    let store = DataStore::new();
    let cfg = config(&store, key)?;
    if cfg.encrypt_profiles && key.is_none() {
        return Err("Unlock profiles first.".into());
    }
    let profiles = store.read_profiles(&cfg)?;
    Ok((store, cfg, profiles))
}

#[tauri::command]
pub async fn save_profile(state: State<'_, Library>, draft: Draft) -> Result<(), String> {
    let key = state.guard()?;
    let (store, cfg, mut profiles) = open(key.as_deref())?;
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("Use a profile name of 1–100 characters.".into());
    }
    let p = if let Some(id) = draft.id {
        let i = find(&profiles, &id, draft.revision.as_deref().unwrap_or(""))?;
        &mut profiles[i]
    } else {
        profiles.push(Profile::new(name));
        profiles.last_mut().ok_or("Could not create the profile.")?
    };
    if let Some(c) = draft.connection {
        if Connection::from_profile(p).is_none() {
            return Err("Advanced network actions must be edited in the classic editor.".into());
        }
        p.network_actions = c.actions()?;
    }
    p.metadata.name = name.into();
    p.metadata.description =
        (!draft.description.trim().is_empty()).then(|| draft.description.trim().into());
    p.metadata.updated_at = Utc::now();
    store.persist_profiles(&profiles, &cfg)?;
    log(
        &store,
        &cfg,
        "INFO",
        &format!("Saved profile: {name}"),
        "Saved an encrypted profile.",
    );
    Ok(())
}

#[tauri::command]
pub async fn delete_profile(
    state: State<'_, Library>,
    id: String,
    revision: String,
) -> Result<(), String> {
    let key = state.guard()?;
    let (store, cfg, mut profiles) = open(key.as_deref())?;
    let i = find(&profiles, &id, &revision)?;
    let removed = profiles.remove(i);
    store.persist_profiles(&profiles, &cfg)?;
    log(
        &store,
        &cfg,
        "INFO",
        &format!("Deleted profile: {}", removed.name()),
        "Deleted an encrypted profile.",
    );
    Ok(())
}

#[tauri::command]
pub async fn apply_profile(
    state: State<'_, Library>,
    id: String,
    revision: String,
    allow_automation: bool,
) -> Result<String, String> {
    let key = state.guard()?;
    let (store, cfg, profiles) = open(key.as_deref())?;
    let i = find(&profiles, &id, &revision)?;
    let p = &profiles[i];
    if !p.automation_actions.is_empty() && !allow_automation {
        return Err("Review the automation actions before applying.".into());
    }
    let report = execute_profile_actions(p, allow_automation, cfg.sandbox_mode);
    if report.aborted || !report.errors.is_empty() || !report.skipped.is_empty() {
        let reason = report
            .errors
            .into_iter()
            .chain(report.skipped)
            .collect::<Vec<_>>()
            .join("\n");
        let message = format!(
            "{}: {} action(s) applied. {reason}",
            p.name(),
            report.applied
        );
        log(
            &store,
            &cfg,
            "WARN",
            &message,
            "An encrypted profile was not fully applied.",
        );
        return Err(message);
    }
    let message = format!("Applied profile: {}", p.name());
    // Commands may take a while. Merge the timestamp into a fresh library
    // instead of overwriting intervening changes from the classic editor.
    let record = || -> Result<(), String> {
        let (store, cfg, mut latest) = open(key.as_deref())?;
        let index = find(&latest, &id, &revision)?;
        latest[index].mark_applied();
        store.persist_profiles(&latest, &cfg)?;
        log(
            &store,
            &cfg,
            "INFO",
            &message,
            "Applied an encrypted profile.",
        );
        Ok(())
    };
    record().map_err(|e| format!("Actions applied, but the result could not be recorded: {e}"))?;
    Ok(message)
}
