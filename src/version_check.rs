// Network Manager - Version Check
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! GitHub release version checker.
//!
//! Checks for newer releases on startup. Silent on any errors.

use serde::Deserialize;
use tracing::{debug, warn};

/// GitHub repo coordinates
const GITHUB_OWNER: &str = "christosdaggas";
const GITHUB_REPO: &str = "NetManager";

/// Largest release response accepted (512 KiB).
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Result of a successful version check.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Latest version string (e.g. "1.2.0").
    pub latest_version: String,
    /// URL the user can visit to download the release.
    pub download_url: String,
    /// Release name / title (may be empty).
    #[allow(dead_code)]
    pub release_name: String,
}

/// Subset of the GitHub Releases API response we care about.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    name: Option<String>,
}

/// Check GitHub for the latest release.
///
/// Returns `Some(UpdateInfo)` if a newer version exists,
/// `None` if the local version is current or on ANY error.
pub async fn check_for_update(current_version: &str) -> Option<UpdateInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        GITHUB_OWNER, GITHUB_REPO
    );

    debug!("Checking for updates at {}", url);

    // Build a client with a 10-second timeout.
    // GitHub API requires a User-Agent header.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(format!("{}/{}", GITHUB_REPO, current_version))
        .build()
        .ok()?;

    let mut response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            debug!("Update check HTTP request failed (not an error): {}", e);
            return None;
        }
    };

    if !response.status().is_success() {
        debug!(
            "Update check got HTTP {} (repo may not have releases yet)",
            response.status()
        );
        return None;
    }

    // Guard against unexpectedly large responses: reject an oversized
    // Content-Length up front and stop reading once the cap is exceeded.
    if let Some(length) = response.content_length() {
        if length > MAX_RESPONSE_BYTES as u64 {
            warn!("Update check response too large ({} bytes), ignoring", length);
            return None;
        }
    }

    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    warn!("Update check response exceeds {} bytes, ignoring", MAX_RESPONSE_BYTES);
                    return None;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => {
                warn!("Failed to read update check response: {}", e);
                return None;
            }
        }
    }

    let release: GitHubRelease = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to parse GitHub release JSON: {}", e);
            return None;
        }
    };

    // Strip leading 'v' or 'V' from tag (e.g. "v1.2.0" → "1.2.0")
    let latest = release
        .tag_name
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string();

    debug!("Update check: local={}, remote={}", current_version, latest);

    if is_newer(&latest, current_version) {
        Some(UpdateInfo {
            latest_version: latest,
            download_url: release.html_url,
            release_name: release.name.unwrap_or_default(),
        })
    } else {
        debug!("Application is up to date");
        None
    }
}

/// Compare two semver-ish version strings.
/// Returns true if `remote` is strictly newer than `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };

    let r = parse(remote);
    let l = parse(local);

    let max_len = r.len().max(l.len());
    for i in 0..max_len {
        let rv = r.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(is_newer("1.0.0.1", "1.0.0"));
    }
}
