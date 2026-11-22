use chrono::Utc;
use netmanager_core::{
    encryption::make_check_token,
    models::{AppConfig, Ipv4Method, NetworkAction, Profile, ProfileTemplate},
    profile_apply::execute_profile_actions,
    storage::{write_private_atomic, DataStore},
};
use serde_json::{json, Value};
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroizing;

#[derive(Default)]
pub struct Library {
    key: Mutex<Option<Zeroizing<String>>>,
    applying: Mutex<()>,
}
impl Library {
    pub fn guard(&self) -> Result<MutexGuard<'_, Option<Zeroizing<String>>>, String> {
        self.key
            .lock()
            .map_err(|_| "Profile library unavailable. Restart Network Manager.".into())
    }
}
pub fn config(store: &DataStore, key: Option<&String>) -> Result<AppConfig, String> {
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
pub fn open(key: Option<&String>) -> Result<(DataStore, AppConfig, Vec<Profile>), String> {
    let store = DataStore::new();
    let cfg = config(&store, key)?;
    if cfg.encrypt_profiles && key.is_none() {
        return Err("Unlock profiles first.".into());
    }
    let profiles = store.read_profiles(&cfg)?;
    Ok((store, cfg, profiles))
}
fn lookup(list: &[Profile], id: &str, revision: Option<&str>) -> Result<usize, String> {
    let i = list
        .iter()
        .position(|p| p.id().to_string() == id)
        .ok_or("Profile no longer exists.")?;
    if revision.is_some_and(|r| list[i].metadata.updated_at.to_rfc3339() != r) {
        return Err("This profile changed. Refresh before continuing.".into());
    }
    Ok(i)
}
#[tauri::command]
pub async fn library_view(
    state: State<'_, Library>,
    services: State<'_, crate::background::Services>,
    hotkeys: State<'_, crate::hotkeys::Hotkeys>,
) -> Result<Value, String> {
    let key = state.guard()?;
    let store = DataStore::new();
    let cfg = config(&store, key.as_deref())?;
    let locked = cfg.encrypt_profiles && key.is_none();
    let mut list = if locked {
        vec![]
    } else {
        store.read_profiles(&cfg)?
    };
    list.sort_by_key(|p| p.name().to_lowercase());
    let summaries:Vec<_>=list.iter().map(|p|json!({"id":p.id(),"name":p.name(),"description":p.metadata.description,
        "group":p.metadata.group.as_ref().map(|g|&g.name),"revision":p.metadata.updated_at.to_rfc3339(),"applied_at":p.metadata.last_applied_at,
        "automation":!p.automation_actions.is_empty(),"actions":p.all_actions().map(|a|a.description()).collect::<Vec<_>>() })).collect();
    services.update(&cfg, &list);
    hotkeys.update(&cfg.hotkeys, cfg.hotkeys_enabled && !locked);
    let mut public = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
    public.as_object_mut().unwrap().remove("encryption_check");
    public["autostart_on_login"] = json!(netmanager_core::autostart::is_autostart_enabled());
    Ok(
        json!({"profiles":summaries,"locked":locked,"settings":public,"storage":store.config_dir(),"version":env!("CARGO_PKG_VERSION"),
        "logs":store.logs().into_iter().rev().collect::<Vec<_>>() }),
    )
}
#[tauri::command]
pub async fn profile_form(
    state: State<'_, Library>,
    id: Option<String>,
    template: Option<usize>,
) -> Result<Profile, String> {
    if let Some(id) = id {
        let k = state.guard()?;
        let (_, _, list) = open(k.as_deref())?;
        return Ok(list[lookup(&list, &id, None)?].clone());
    }
    let templates = [
        ProfileTemplate::Blank,
        ProfileTemplate::HomeNetwork,
        ProfileTemplate::OfficeNetwork,
        ProfileTemplate::PublicWifi,
        ProfileTemplate::VpnOnly,
        ProfileTemplate::Development,
    ];
    let t = templates
        .get(template.unwrap_or(0))
        .ok_or("Unknown template.")?;
    if *t == ProfileTemplate::Blank {
        Ok(Profile::new(""))
    } else {
        Ok(t.create_profile(t.name()))
    }
}
#[tauri::command]
pub async fn save_profile(
    state: State<'_, Library>,
    id: Option<String>,
    revision: Option<String>,
    mut draft: Profile,
) -> Result<(), String> {
    let k = state.guard()?;
    let (s, c, mut list) = open(k.as_deref())?;
    let name = draft.name().trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("Use a name of 1–100 characters.".into());
    }
    for action in &draft.network_actions {
        if let NetworkAction::Ipv4Config {
            method, addresses, ..
        } = action
        {
            if matches!(method, Ipv4Method::Manual) && addresses.is_empty()
                || addresses.iter().any(|a| a.prefix > 32)
            {
                return Err("Invalid static IPv4 configuration.".into());
            }
        }
    }
    let editing = id.is_some();
    let i = if let Some(id) = id {
        lookup(&list, &id, Some(revision.as_deref().unwrap_or("")))?
    } else {
        list.push(Profile::new(&name));
        list.len() - 1
    };
    draft.metadata.id = list[i].id();
    draft.metadata.created_at = list[i].metadata.created_at;
    draft.metadata.updated_at = Utc::now();
    draft.metadata.last_applied_at = list[i].metadata.last_applied_at;
    draft.metadata.name = name;
    draft.approved_for_users = false;
    list[i] = draft;
    s.persist_profiles(&list, &c)?;
    s.append_log(
        "INFO",
        if editing {
            "Profile updated."
        } else {
            "Profile created."
        },
    );
    Ok(())
}
#[tauri::command]
pub async fn profile_operation(
    state: State<'_, Library>,
    id: String,
    revision: String,
    kind: String,
) -> Result<(), String> {
    let k = state.guard()?;
    let (s, c, mut list) = open(k.as_deref())?;
    let i = lookup(&list, &id, Some(&revision))?;
    match kind.as_str() {
        "duplicate" => {
            let mut p = list[i].clone();
            p.metadata.id = uuid::Uuid::new_v4();
            p.metadata.name.push_str(" (Copy)");
            p.metadata.last_applied_at = None;
            p.metadata.created_at = Utc::now();
            p.metadata.updated_at = p.metadata.created_at;
            list.push(p);
        }
        "delete" => {
            list.remove(i);
        }
        _ => return Err("Unknown operation.".into()),
    }
    s.persist_profiles(&list, &c)?;
    s.append_log("INFO", &format!("Profile operation: {kind}."));
    Ok(())
}
#[tauri::command]
pub async fn unlock_profiles(state: State<'_, Library>, passphrase: String) -> Result<(), String> {
    let mut key = state.guard()?;
    let passphrase = Zeroizing::new(passphrase);
    let s = DataStore::new();
    let c = config(&s, Some(&passphrase))?;
    if !s.has_encrypted_cache()
        && !s.verify_encryption_passphrase(&passphrase, c.encryption_check.as_deref())
    {
        return Err("Incorrect passphrase.".into());
    }
    s.read_profiles(&c)
        .map_err(|_| "Could not unlock this profile library.")?;
    *key = Some(passphrase);
    Ok(())
}
#[tauri::command]
pub async fn encryption(
    state: State<'_, Library>,
    current: String,
    new_key: Option<String>,
) -> Result<(), String> {
    let mut key = state.guard()?;
    let s = DataStore::new();
    let current = Zeroizing::new(current);
    let mut c = config(&s, Some(&current))?;
    if c.encrypt_profiles
        && !s.has_encrypted_cache()
        && !s.verify_encryption_passphrase(&current, c.encryption_check.as_deref())
    {
        return Err("Incorrect current passphrase.".into());
    }
    let list = s
        .read_profiles(&c)
        .map_err(|_| "Could not unlock the existing library.")?;
    let next = new_key.map(Zeroizing::new);
    if next.as_ref().is_some_and(|k| k.len() < 8) {
        return Err("Use a passphrase of at least eight characters.".into());
    }
    c.encryption_key = next.as_deref().cloned();
    c.encrypt_profiles = next.is_some();
    c.encryption_check = next
        .as_ref()
        .map(|k| make_check_token(k))
        .transpose()
        .map_err(|e| e.to_string())?;
    s.persist_profiles(&list, &c)?;
    *key = next;
    s.save_config(&c).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn save_setting(
    app: AppHandle,
    state: State<'_, Library>,
    field: String,
    value: Value,
) -> Result<(), String> {
    let k = state.guard()?;
    let s = DataStore::new();
    let cfg = config(&s, k.as_deref())?;
    let allowed = [
        "theme",
        "auto_switch_interval_secs",
        "auto_switch_enabled",
        "auto_switch_allow_scripts",
        "show_notifications",
        "start_minimized",
        "show_tray_icon",
        "autostart_on_login",
        "confirm_profile_switch",
        "watchdog",
        "schedules",
        "scheduling_enabled",
        "sandbox_mode",
        "hotkeys",
        "hotkeys_enabled",
    ];
    if !allowed.contains(&field.as_str()) {
        return Err("Unknown setting.".into());
    }
    let mut v = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
    v[&field] = value;
    let next: AppConfig = serde_json::from_value(v).map_err(|e| e.to_string())?;
    if !(5..=300).contains(&next.auto_switch_interval_secs)
        || next.watchdog.check_interval_secs < 5
        || next.watchdog.failure_threshold == 0
        || !netmanager_core::port_scanner::is_valid_host(&next.watchdog.ping_target)
    {
        return Err("Invalid interval, threshold or ping target.".into());
    }
    if field == "autostart_on_login" {
        if next.autostart_on_login {
            let exe = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .display()
                .to_string();
            let quoted = exe
                .replace('\\', "\\\\\\\\")
                .replace('"', "\\\\\"")
                .replace('`', "\\\\`")
                .replace('$', "\\\\$")
                .replace('%', "%%");
            netmanager_core::autostart::enable_autostart_for(&format!("\"{quoted}\""))?;
        } else {
            netmanager_core::autostart::disable_autostart()?;
        }
    }
    s.save_config(&next).map_err(|e| e.to_string())?;
    if field == "theme" {
        app.emit("theme-changed", next.theme)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub async fn clear_logs(state: State<'_, Library>) -> Result<(), String> {
    let _guard = state.guard()?;
    let s = DataStore::new();
    write_private_atomic(&s.config_dir().join("logs.json"), b"[]").map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn apply_profile(
    state: State<'_, Library>,
    id: String,
    revision: String,
    allow_automation: bool,
    automatic: bool,
) -> Result<String, String> {
    let _apply = state
        .applying
        .try_lock()
        .map_err(|_| "Another profile is being applied.")?;
    let k = state.guard()?;
    let (_, c, list) = open(k.as_deref())?;
    drop(k);
    let i = lookup(&list, &id, Some(&revision))?;
    let p = &list[i];
    let run = !automatic || c.auto_switch_allow_scripts;
    if run && !p.automation_actions.is_empty() && !allow_automation {
        return Err("Review automation actions before applying.".into());
    }
    let report = execute_profile_actions(p, run && allow_automation, c.sandbox_mode);
    let k = state.guard()?;
    if report.aborted || !report.errors.is_empty() || !report.skipped.is_empty() {
        let reason = report
            .errors
            .into_iter()
            .chain(report.skipped)
            .collect::<Vec<_>>()
            .join("\n");
        DataStore::new().append_log(
            "WARN",
            if c.encrypt_profiles {
                "Encrypted profile not fully applied."
            } else {
                &reason
            },
        );
        return Err(format!("{} actions applied. {reason}", report.applied));
    }
    let (s, c, mut latest) = open(k.as_deref())?;
    let i = lookup(&latest, &id, Some(&revision))?;
    latest[i].mark_applied();
    s.persist_profiles(&latest, &c)?;
    let message = format!("Applied profile: {}", p.name());
    s.append_log(
        "INFO",
        if c.encrypt_profiles {
            "Applied an encrypted profile."
        } else {
            &message
        },
    );
    if c.show_notifications {
        let _ = std::process::Command::new("notify-send")
            .args([
                "--app-name=Network Manager",
                "--",
                "Network Manager",
                &message,
            ])
            .output();
    }
    Ok(message)
}
