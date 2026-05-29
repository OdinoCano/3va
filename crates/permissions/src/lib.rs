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
pub mod sandbox;

pub use audit::{AuditEvent, AuditLog, AuditLogger};
pub use capability::{Capability, PermissionState};
pub use enforcement::{EnvEnforcer, FsEnforcer, NetEnforcer, PermissionError, ProcessEnforcer};
pub use sandbox::{VirtualFs, VirtualNetwork};
