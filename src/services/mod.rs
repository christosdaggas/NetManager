// Network Manager - Background Services
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Background services for automated operations.
//!
//! This module contains services that run in the background:
//! - Scheduler: Activates profiles at scheduled times
//! - Watchdog: Monitors connectivity and takes action on failure
//! - Sandbox: Provides script execution isolation
//! - Encryption: Profile data encryption/decryption
//! - Scanner: Discovers hosts on the local network

pub mod autoswitch;
pub mod encryption;
pub mod sandbox;
pub mod scanner;
pub mod watchdog;

pub use watchdog::WatchdogService;

pub use autoswitch::AutoSwitchService;
#[allow(unused_imports)]
pub use encryption::{make_check_token, verify_check_token, ProfileEncryption};
#[allow(unused_imports)]
pub use sandbox::SandboxRunner;
