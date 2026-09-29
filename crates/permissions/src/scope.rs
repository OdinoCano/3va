// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

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

/// Reads which packages have code on the JS stack (installed by the engine).
type StackResolver = fn() -> Vec<String>;

thread_local! {
    static CURRENT_SCOPE: RefCell<String> = const { RefCell::new(String::new()) };
    static STACK_RESOLVER: std::cell::Cell<Option<StackResolver>> =
        const { std::cell::Cell::new(None) };
    static INHERITED_SCOPES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Installs the JS engine's reader of "which packages have code on the
/// stack right now". Unlike [`current_scope`], which the `require()` wrapper
/// sets from JS, the stack can't be sidestepped by calling a global (`fetch`),
/// a constructor (`new net.Socket()`), a raw native binding, or by tampering
/// with the wrapper's globals.
pub fn set_stack_scopes_resolver(f: Option<StackResolver>) {
    STACK_RESOLVER.with(|r| r.set(f));
}

/// Scopes carried over from where deferred work was scheduled (a timer
/// callback runs with the scopes that were on the stack at `setTimeout`).
/// Returns the previous value so the caller can restore it.
pub fn set_inherited_scopes(scopes: Vec<String>) -> Vec<String> {
    INHERITED_SCOPES.with(|s| std::mem::replace(&mut *s.borrow_mut(), scopes))
}

static SCOPED_RULES_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set once any package-scoped grant or deny rule exists. Until then nobody needs to
/// know which package is running, so stack walks and promise tagging are
/// skipped entirely.
pub fn mark_scoped_rules_active() {
    SCOPED_RULES_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn scoped_rules_active() -> bool {
    SCOPED_RULES_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Package scopes whose code is on the JS stack right now (empty when no
/// resolver is installed, only app code is running, or no package has deny
/// rules).
pub fn stack_scopes() -> Vec<String> {
    if !scoped_rules_active() {
        return Vec::new();
    }
    STACK_RESOLVER
        .with(|r| r.get())
        .map(|f| f())
        .unwrap_or_default()
}

/// Packages involved in the current operation: every package with code on
/// the stack plus those inherited from where deferred work was scheduled.
/// Without a stack resolver (no JS engine on this thread) this falls back to
/// the scope set via [`set_current_scope`]. Never contains [`ROOT_SCOPE`].
pub fn package_scopes() -> Vec<String> {
    let mut out = if STACK_RESOLVER.with(|r| r.get()).is_some() {
        stack_scopes()
    } else {
        let cur = current_scope();
        if cur == ROOT_SCOPE {
            Vec::new()
        } else {
            vec![cur]
        }
    };
    out.extend(INHERITED_SCOPES.with(|s| s.borrow().clone()));
    out.sort();
    out.dedup();
    out
}

/// Scopes whose `deny-*` rules apply: [`package_scopes`] plus the scope the
/// require() wrapper set. JS can set the latter to anything, which is
/// harmless here (it can only add denials) but is why grants ignore it.
pub fn deny_scopes() -> Vec<String> {
    let mut out = package_scopes();
    let cur = current_scope();
    if cur != ROOT_SCOPE && !out.contains(&cur) {
        out.push(cur);
    }
    out
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
