// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Permission model — capability-based sandbox for network, filesystem, env, and process access.
//!
//! # Examples
//!
//! ```
//! use vvva_permissions::{Capability, PermissionState};
//! use std::path::PathBuf;
//!
//! let ps = PermissionState::new();
//!
//! // Denied by default
//! assert!(!ps.check(&Capability::Network("example.com".into())));
//!
//! // Grant and re-check
//! ps.grant(Capability::Network("example.com".into()));
//! assert!(ps.check(&Capability::Network("example.com".into())));
//!
//! // Wildcard grant: "*" allows any host
//! let ps2 = PermissionState::new();
//! ps2.grant(Capability::Network("*".into()));
//! assert!(ps2.check(&Capability::Network("any-host.io".into())));
//! ```

pub mod audit;
pub mod capability;
pub mod enforcement;
pub mod insecure;
pub mod preset;
pub mod sandbox;
pub mod scope;

pub use audit::{AuditEvent, AuditLog, AuditLogger};
pub use capability::{
    Capability, PermissionState, authority, describe_capability, grant_flag, manifest_key,
};
pub use enforcement::{EnvEnforcer, FsEnforcer, NetEnforcer, PermissionError, ProcessEnforcer};
pub use insecure::{plaintext_allowed, plaintext_denied_message, set_allow_insecure};
pub use preset::PermissionPreset;
pub use sandbox::{VirtualFs, VirtualNetwork};
pub use scope::{ROOT_SCOPE, ScopeGuard, current_scope, set_current_scope};
