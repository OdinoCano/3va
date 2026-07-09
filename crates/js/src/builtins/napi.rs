//! Node-API (NAPI) compatibility layer stub for V8.
//!
//! Native .node addons are not supported in the V8 version of 3va.
//! This module provides stub implementations for compatibility.

use std::sync::Arc;
use v8::{ContextScope, HandleScope};
use vvva_permissions::PermissionState;

pub fn inject_napi(
    _scope: &mut ContextScope<HandleScope>,
    _permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    // Native .node addons are not supported with V8 engine.
    // The require() function will fail for .node files with an appropriate error.
    Ok(())
}
