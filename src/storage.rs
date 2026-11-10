// Network Manager - Local Storage
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Local data storage for the GUI application.
//!
//! Handles:
//! - Profile listing (from the local cache)
//! - Application settings
//! - Log history
//!
//! This module uses RwLock for thread-safe access. Lock poisoning is handled
//! gracefully by recovering the inner value, as poison indicates a panic
//! in another thread but the data itself may still be valid.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;
use tracing::{error, info, warn};

use crate::models::{AppConfig, Profile, CONFIG_DIR_NAME};
use crate::services::ProfileEncryption;

/// Write `contents` to `path` atomically with owner-only (0600) permissions.
///
/// The data goes to a temporary file in the same directory, is flushed to
/// disk, and then renamed over `path`, so readers see either the old or the
/// new file and never a truncated one.
pub fn write_private_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp_path = path.with_file_name(tmp_name);

    let result = write_and_rename(&tmp_path, path, contents);
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Write `contents` to a new owner-only file at `tmp_path`, flush it, and
/// rename it to `path`.
fn write_and_rename(tmp_path: &Path, path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(tmp_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp_path, path)
}

/// Copy a file that failed to load to `<name>.corrupt-<timestamp>` next to it,
/// so that defaults saved later cannot overwrite the only copy.
fn backup_unreadable_file(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = path.with_file_name(format!("{}.corrupt-{}", file_name, stamp));

    match fs::copy(path, &backup) {
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&backup, fs::Permissions::from_mode(0o600));
            }
            warn!("Backed up unreadable {:?} to {:?}", path, backup);
            Some(backup)
        }
        Err(e) => {
            error!("Failed to back up unreadable {:?}: {}", path, e);
            None
        }
    }
}

/// A log entry with timestamp, level, and message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Local data store for the GUI application.
#[derive(Debug)]
pub struct DataStore {
    /// Configuration directory path.
    config_dir: PathBuf,
    /// Settings file path.
    settings_file: PathBuf,
    /// Local profile cache file.
    profiles_cache_file: PathBuf,
    /// Log file path.
    logs_file: PathBuf,

    /// In-memory profile cache (loaded from disk).
    profiles: RwLock<HashMap<String, Profile>>,
    /// Application settings.
    settings: RwLock<AppConfig>,
    /// In-memory log entries (also persisted to disk).
    logs: RwLock<Vec<LogEntry>>,
    /// Whether an unreadable profiles cache was already backed up this session.
    profiles_cache_backed_up: AtomicBool,
    /// Whether the encrypted cache on disk was decrypted or written this
    /// session, so its profiles are in memory and may replace it.
    encrypted_cache_loaded: AtomicBool,
}

impl DataStore {
    /// Create a new data store with default config directory.
    pub fn new() -> Self {
        let config_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/daggas")
            .join(CONFIG_DIR_NAME);
        Self::with_config_dir(config_dir)
    }

    /// Create a new data store with a specific config directory.
    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        // Create directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&config_dir) {
            error!("Failed to create config directory: {}", e);
        }
        // Set restrictive permissions on the config directory (0700)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700));
        }

        let store = Self {
            settings_file: config_dir.join("settings.toml"),
            profiles_cache_file: config_dir.join("profiles_cache.json"),
            logs_file: config_dir.join("logs.json"),
            config_dir,
            profiles: RwLock::new(HashMap::new()),
            settings: RwLock::new(AppConfig::default()),
            logs: RwLock::new(Vec::new()),
            profiles_cache_backed_up: AtomicBool::new(false),
            encrypted_cache_loaded: AtomicBool::new(false),
        };

        store.load_settings();
        store.load_logs();
        store
    }

    // ========================================================================
    // RwLock Helper Methods (handle poisoning gracefully)
    // ========================================================================

    /// Read from RwLock, recovering from poison if needed.
    fn read_lock<T, F, R>(&self, lock: &RwLock<T>, context: &str, reader: F) -> R
    where
        F: FnOnce(&T) -> R,
        T: Default,
        R: Default,
    {
        match lock.read() {
            Ok(guard) => reader(&*guard),
            Err(poisoned) => {
                warn!("RwLock poisoned reading {}, recovering", context);
                reader(&*poisoned.into_inner())
            }
        }
    }

    /// Write to RwLock, recovering from poison if needed.
    fn write_lock<T, F>(&self, lock: &RwLock<T>, context: &str, writer: F)
    where
        F: FnOnce(&mut T),
    {
        match lock.write() {
            Ok(mut guard) => writer(&mut *guard),
            Err(poisoned) => {
                warn!("RwLock poisoned writing {}, recovering", context);
                writer(&mut *poisoned.into_inner())
            }
        }
    }

    // ========================================================================
    // Settings
    // ========================================================================

    /// Load the application configuration.
    pub fn load_config() -> Option<AppConfig> {
        let config_dir = dirs::home_dir()?.join(".local/share/daggas").join(CONFIG_DIR_NAME);
        let settings_file = config_dir.join("settings.toml");

        if !settings_file.exists() {
            return None;
        }

        match AppConfig::load_from_file(&settings_file) {
            Ok(config) => Some(config),
            Err(e) => {
                // Defaults are written back on the next save, so keep a copy
                // of the broken file first.
                warn!(
                    "Failed to load settings from {:?}, using defaults: {}",
                    settings_file, e
                );
                backup_unreadable_file(&settings_file);
                None
            }
        }
    }

    /// Load settings from disk.
    fn load_settings(&self) {
        if self.settings_file.exists() {
            match AppConfig::load_from_file(&self.settings_file) {
                Ok(config) => {
                    self.write_lock(&self.settings, "settings", |s| {
                        *s = config;
                    });
                    info!("Loaded settings from {:?}", self.settings_file);
                }
                Err(e) => {
                    error!("Failed to load settings: {}", e);
                }
            }
        }
    }

    /// Save settings to disk.
    fn save_settings(&self) {
        let settings = self.settings();
        if let Err(e) = settings.save_to_file(&self.settings_file) {
            error!("Failed to save settings: {}", e);
        }
    }

    /// Get the current settings.
    pub fn settings(&self) -> AppConfig {
        self.read_lock(&self.settings, "settings", |s| s.clone())
    }

    /// Update settings.
    pub fn update_settings(&self, settings: AppConfig) {
        self.write_lock(&self.settings, "settings", |s| {
            *s = settings;
        });
        self.save_settings();
    }

    /// Save configuration.
    pub fn save_config(&self, config: &AppConfig) -> Result<(), crate::models::Error> {
        config.save_to_file(&self.settings_file)
    }

    // ========================================================================
    // Profiles
    // ========================================================================

    /// Get all profiles, oldest first.
    pub fn profiles(&self) -> Vec<Profile> {
        let mut profiles: Vec<Profile> = self.read_lock(&self.profiles, "profiles", |p| {
            p.values().cloned().collect()
        });
        // HashMap order is random per process; sort so lists, the saved cache
        // and auto-switch priority ties are stable across launches.
        profiles.sort_by(|a, b| {
            a.metadata
                .created_at
                .cmp(&b.metadata.created_at)
                .then_with(|| a.name().cmp(b.name()))
                .then_with(|| a.id().cmp(&b.id()))
        });
        profiles
    }

    /// Get a profile by ID.
    pub fn profile(&self, id: &str) -> Option<Profile> {
        self.read_lock(&self.profiles, "profiles", |p| p.get(id).cloned())
    }

    /// Update the local profile cache.
    pub fn update_profiles_cache(&self, profiles: Vec<Profile>) {
        self.write_lock(&self.profiles, "profiles", |cache| {
            cache.clear();
            for profile in profiles {
                cache.insert(profile.id().to_string(), profile);
            }
        });
        self.save_profiles_cache();
    }

    // ========================================================================
    // Profiles Cache
    // ========================================================================

    /// Load profiles cache from disk.
    pub fn load_profiles_cache(&self) {
        let settings = self.settings();

        // Check for encrypted file first
        let enc_file = self.profiles_cache_file.with_extension("json.enc");
        if enc_file.exists() {
            let Some(ref key) = settings.encryption_key else {
                warn!("Encrypted profiles file exists but no encryption key is unlocked");
                return;
            };

            match fs::read_to_string(&enc_file) {
                Ok(encrypted) => {
                    let encryption = ProfileEncryption::with_key(key);
                    match encryption.decrypt_json::<Vec<Profile>>(&encrypted) {
                        Ok(profiles) => {
                            let profile_count = profiles.len();
                            self.write_lock(&self.profiles, "profiles_cache", |cache| {
                                for profile in profiles {
                                    cache.insert(profile.id().to_string(), profile);
                                }
                            });
                            self.encrypted_cache_loaded.store(true, Ordering::SeqCst);
                            info!("Loaded {} encrypted profiles from cache", profile_count);
                        }
                        Err(e) => {
                            // Do not silently fall back to a stale plaintext
                            // cache when an encrypted one exists. The next save
                            // replaces the file, so keep a copy first.
                            warn!("Failed to decrypt profiles: {:?}", e);
                            if !self.profiles_cache_backed_up.swap(true, Ordering::SeqCst) {
                                backup_unreadable_file(&enc_file);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read encrypted profiles: {}", e);
                }
            }
            return;
        }

        // Fall back to unencrypted file
        if !self.profiles_cache_file.exists() {
            return;
        }

        match File::open(&self.profiles_cache_file) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match serde_json::from_reader::<_, Vec<Profile>>(reader) {
                    Ok(profiles) => {
                        let profile_count = profiles.len();
                        self.write_lock(&self.profiles, "profiles_cache", |cache| {
                            for profile in profiles {
                                cache.insert(profile.id().to_string(), profile);
                            }
                        });
                        info!("Loaded {} profiles from cache", profile_count);
                    }
                    Err(e) => {
                        error!("Failed to parse profiles cache: {}", e);
                        // The next save replaces the file, so keep a copy first.
                        if !self.profiles_cache_backed_up.swap(true, Ordering::SeqCst) {
                            backup_unreadable_file(&self.profiles_cache_file);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to open profiles cache: {}", e);
            }
        }
    }

    /// Save profiles cache to disk.
    fn save_profiles_cache(&self) {
        let profiles: Vec<Profile> = self.profiles();
        let enc_file = self.profiles_cache_file.with_extension("json.enc");

        // Check if encryption is enabled
        let settings = self.settings();

        if settings.encrypt_profiles {
            if let Some(ref key) = settings.encryption_key {
                // Save encrypted
                let encryption = ProfileEncryption::with_key(key);
                match encryption.encrypt_json(&profiles) {
                    Ok(encrypted) => {
                        // Write atomically with 0600 permissions.
                        if let Err(e) = write_private_atomic(&enc_file, encrypted.as_bytes()) {
                            error!("Failed to write encrypted profiles: {}", e);
                        } else {
                            self.encrypted_cache_loaded.store(true, Ordering::SeqCst);
                            // Remove unencrypted file if it exists
                            let _ = fs::remove_file(&self.profiles_cache_file);
                            info!("Saved {} encrypted profiles", profiles.len());
                        }
                        return;
                    }
                    Err(e) => {
                        error!("Encryption failed; profiles were not saved: {:?}", e);
                        return;
                    }
                }
            } else {
                error!("Profile encryption is enabled but no passphrase is unlocked");
                return;
            }
        }

        // An encrypted cache that was never unlocked this session was not
        // loaded, so replacing it would discard every profile in it.
        if enc_file.exists() && !self.encrypted_cache_loaded.load(Ordering::SeqCst) {
            error!("Encrypted profiles exist but are not unlocked; profiles were not saved");
            return;
        }

        // Save unencrypted (default), and drop any stale encrypted cache.
        match serde_json::to_vec_pretty(&profiles) {
            Ok(json) => {
                if let Err(e) = write_private_atomic(&self.profiles_cache_file, &json) {
                    error!("Failed to write profiles cache: {}", e);
                } else if enc_file.exists() {
                    if let Err(e) = fs::remove_file(&enc_file) {
                        warn!("Failed to remove stale encrypted cache: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to serialize profiles cache: {}", e);
            }
        }
    }

    /// Save the profiles cache immediately (for example after changing
    /// encryption settings).
    pub fn save_profiles_cache_now(&self) {
        self.save_profiles_cache();
    }

    /// Whether an encrypted profile cache exists on disk.
    pub fn has_encrypted_cache(&self) -> bool {
        self.profiles_cache_file.with_extension("json.enc").exists()
    }

    /// Whether profiles are encrypted (by setting, or because an encrypted
    /// cache exists on disk) but no passphrase is unlocked.
    pub fn encryption_locked(&self) -> bool {
        let settings = self.settings();
        settings.encryption_key.is_none()
            && (settings.encrypt_profiles
                || (self.has_encrypted_cache()
                    && !self.encrypted_cache_loaded.load(Ordering::SeqCst)))
    }

    /// Verify a passphrase against the stored check token, or (for older
    /// installations that have no token) against the encrypted cache itself.
    pub fn verify_encryption_passphrase(&self, passphrase: &str, check: Option<&str>) -> bool {
        if let Some(token) = check {
            return crate::services::verify_check_token(token, passphrase);
        }

        let enc_file = self.profiles_cache_file.with_extension("json.enc");
        if !enc_file.exists() {
            return true;
        }

        fs::read_to_string(&enc_file)
            .ok()
            .map(|encrypted| {
                ProfileEncryption::with_key(passphrase)
                    .decrypt_json::<Vec<Profile>>(&encrypted)
                    .is_ok()
            })
            .unwrap_or(false)
    }

    /// Replace in-memory profiles with the ones loaded from disk.
    pub fn reload_profiles_cache(&self) {
        self.write_lock(&self.profiles, "profiles_cache", |cache| cache.clear());
        self.load_profiles_cache();
    }

    // ========================================================================
    // Logs
    // ========================================================================

    /// Load logs from disk.
    fn load_logs(&self) {
        if !self.logs_file.exists() {
            return;
        }

        match File::open(&self.logs_file) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match serde_json::from_reader::<_, Vec<LogEntry>>(reader) {
                    Ok(entries) => {
                        let entry_count = entries.len();
                        self.write_lock(&self.logs, "logs", |logs| {
                            *logs = entries;
                        });
                        info!("Loaded {} log entries from disk", entry_count);
                    }
                    Err(e) => {
                        error!("Failed to parse logs file: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to open logs file: {}", e);
            }
        }
    }

    /// Save logs to disk.
    fn save_logs(&self) {
        let logs = self.logs();

        match serde_json::to_vec_pretty(&logs) {
            Ok(json) => {
                if let Err(e) = write_private_atomic(&self.logs_file, &json) {
                    error!("Failed to write logs file: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to serialize logs: {}", e);
            }
        }
    }

    /// Get all log entries.
    pub fn logs(&self) -> Vec<LogEntry> {
        self.read_lock(&self.logs, "logs", |l| l.clone())
    }

    /// Append a log entry and save to disk.
    pub fn append_log(&self, level: &str, message: &str) {
        let now = chrono::Local::now();
        let entry = LogEntry {
            timestamp: now.format("%Y-%m-%d %H:%M:%S").to_string(),
            level: level.to_uppercase(),
            message: message.to_string(),
        };

        let max_entries = self.settings().max_log_entries;

        self.write_lock(&self.logs, "logs", |logs| {
            logs.push(entry);

            // Keep only the last N entries based on config
            if logs.len() > max_entries {
                let drain_count = logs.len() - max_entries;
                logs.drain(0..drain_count);
            }
        });

        self.save_logs();
    }

    /// Clear all logs and save to disk.
    pub fn clear_logs(&self) {
        self.write_lock(&self.logs, "logs", |logs| {
            logs.clear();
        });
        self.save_logs();
    }

    /// Get the config directory path.
    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }
}

impl Default for DataStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the system temp dir.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "network-manager-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn test_write_private_atomic_replaces_contents() {
        let dir = temp_dir("atomic");
        let path = dir.join("settings.toml");

        write_private_atomic(&path, b"first").expect("first write");
        write_private_atomic(&path, b"second").expect("second write");
        assert_eq!(fs::read(&path).expect("read back"), b"second");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // No temporary files are left behind.
        let entries: Vec<_> = fs::read_dir(&dir).expect("read dir").collect();
        assert_eq!(entries.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unreadable_profiles_cache_is_backed_up() {
        let dir = temp_dir("corrupt-cache");
        let cache = dir.join("profiles_cache.json");
        fs::write(&cache, b"[{\"truncated\"").expect("write corrupt cache");

        let store = DataStore::with_config_dir(dir.clone());
        store.load_profiles_cache();
        assert!(store.profiles().is_empty());

        let backups: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("profiles_cache.json.corrupt-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups[0].path()).expect("read backup"),
            b"[{\"truncated\""
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_encrypted_profiles_round_trip() {
        let dir = temp_dir("encrypted");
        let store = DataStore::with_config_dir(dir.clone());

        let passphrase = "correct horse battery staple";
        let check = crate::services::make_check_token(passphrase).expect("check token");
        let config = AppConfig {
            encrypt_profiles: true,
            encryption_key: Some(passphrase.to_string()),
            encryption_check: Some(check.clone()),
            ..Default::default()
        };
        store.update_settings(config.clone());

        store.update_profiles_cache(vec![Profile::new("Encrypted")]);

        assert!(dir.join("profiles_cache.json.enc").exists());
        assert!(!dir.join("profiles_cache.json").exists());
        assert!(store.verify_encryption_passphrase(passphrase, Some(&check)));
        assert!(!store.verify_encryption_passphrase("wrong passphrase", Some(&check)));

        // A new process starts without the runtime key; setting it allows
        // the encrypted cache to be read again.
        let reopened = DataStore::with_config_dir(dir.clone());
        reopened.update_settings(config);
        reopened.load_profiles_cache();
        let profiles = reopened.profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name(), "Encrypted");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_disabling_encryption_removes_encrypted_cache() {
        let dir = temp_dir("decrypt");
        let store = DataStore::with_config_dir(dir.clone());

        let passphrase = "a long enough passphrase";
        let mut config = AppConfig {
            encrypt_profiles: true,
            encryption_key: Some(passphrase.to_string()),
            encryption_check: Some(
                crate::services::make_check_token(passphrase).expect("check token"),
            ),
            ..Default::default()
        };
        store.update_settings(config.clone());
        store.update_profiles_cache(vec![Profile::new("Secret")]);
        assert!(dir.join("profiles_cache.json.enc").exists());

        config.encrypt_profiles = false;
        config.encryption_key = None;
        config.encryption_check = None;
        store.update_settings(config);
        store.save_profiles_cache_now();

        assert!(!dir.join("profiles_cache.json.enc").exists());
        assert!(dir.join("profiles_cache.json").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_encrypted_cache_survives_lost_encryption_setting() {
        let dir = temp_dir("lost-setting");
        let passphrase = "a long enough passphrase";
        {
            let store = DataStore::with_config_dir(dir.clone());
            store.update_settings(AppConfig {
                encrypt_profiles: true,
                encryption_key: Some(passphrase.to_string()),
                ..Default::default()
            });
            store.update_profiles_cache(vec![Profile::new("Secret")]);
        }
        let encrypted = fs::read(dir.join("profiles_cache.json.enc")).expect("encrypted cache");

        // Settings reset to defaults: encryption off, nothing unlocked.
        let reopened = DataStore::with_config_dir(dir.clone());
        reopened.update_settings(AppConfig::default());
        reopened.load_profiles_cache();
        assert!(reopened.encryption_locked());

        reopened.update_profiles_cache(vec![Profile::new("New")]);
        assert_eq!(fs::read(dir.join("profiles_cache.json.enc")).expect("still there"), encrypted);
        assert!(!dir.join("profiles_cache.json").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_profiles_are_sorted_by_creation_time() {
        let dir = temp_dir("sorted");
        let store = DataStore::with_config_dir(dir.clone());

        let mut older = Profile::new("Zeta");
        let mut newer = Profile::new("Alpha");
        older.metadata.created_at = chrono::Utc::now() - chrono::TimeDelta::hours(1);
        newer.metadata.created_at = chrono::Utc::now();
        store.update_profiles_cache(vec![newer, older]);

        let names: Vec<String> = store
            .profiles()
            .iter()
            .map(|p| p.name().to_string())
            .collect();
        assert_eq!(names, vec!["Zeta".to_string(), "Alpha".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }
}
