//! Tracks "which package's code is currently executing" so `PermissionState`
//! can apply a grant declared for one dependency (`package.json["3va"].permissions.axios`)
//! without applying it to every other dependency too.
//!
//! Set by the JS engine's `require()` wrapper right before it hands a
//! capability-gated builtin (fs, net, ...) to the requesting module, and
//! reset immediately after. One JsEngine per thread (see `FS_PERMISSIONS` in
//! `crates/js/src/builtins/fs.rs` for why), so a thread-local is the correct
//! scope for this too.

use std::cell::RefCell;

thread_local! {
    static CURRENT_SCOPE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The root/app scope — used when no package-specific scope is active.
pub const ROOT_SCOPE: &str = ".";

/// Returns the currently active scope, or [`ROOT_SCOPE`] if none is set.
pub fn current_scope() -> String {
    CURRENT_SCOPE.with(|s| {
        let s = s.borrow();
        if s.is_empty() {
            ROOT_SCOPE.to_string()
        } else {
            s.clone()
        }
    })
}

/// Sets the active scope for this thread. Pass [`ROOT_SCOPE`] (or `"."`) to
/// clear back to the app-level scope.
pub fn set_current_scope(scope: &str) {
    CURRENT_SCOPE.with(|s| {
        *s.borrow_mut() = if scope == ROOT_SCOPE {
            String::new()
        } else {
            scope.to_string()
        };
    });
}

/// RAII guard: sets the scope on construction, restores the previous value on drop.
/// Safe against re-entrant/nested scopes (e.g. a package's code calling into
/// another required module that itself touches a gated builtin).
pub struct ScopeGuard {
    previous: String,
}

impl ScopeGuard {
    pub fn enter(scope: &str) -> Self {
        let previous = current_scope();
        set_current_scope(scope);
        ScopeGuard { previous }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        set_current_scope(&self.previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_root_scope() {
        assert_eq!(current_scope(), ROOT_SCOPE);
    }

    #[test]
    fn guard_restores_previous_scope_on_drop() {
        assert_eq!(current_scope(), ROOT_SCOPE);
        {
            let _g1 = ScopeGuard::enter("axios");
            assert_eq!(current_scope(), "axios");
            {
                let _g2 = ScopeGuard::enter("express");
                assert_eq!(current_scope(), "express");
            }
            assert_eq!(current_scope(), "axios");
        }
        assert_eq!(current_scope(), ROOT_SCOPE);
    }
}
