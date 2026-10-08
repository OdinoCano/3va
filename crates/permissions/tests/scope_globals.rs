// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Process-global scoped-rule state can only be asserted from a fresh process.
//!
//! `SCOPED_RULES_ACTIVE` is a monotonic atomic shared by every test in a
//! binary, so once any test flips it there is no way back. This file gets its
//! own test binary, where the flag starts false.

use vvva_permissions::scope::{
    mark_scoped_rules_active, scoped_rules_active, set_stack_scopes_resolver, stack_scopes,
};

#[test]
fn scoped_rules_flag_starts_off_and_stack_scopes_needs_it() {
    assert!(
        !scoped_rules_active(),
        "a fresh test binary must start with scoped rules inactive"
    );
    assert!(stack_scopes().is_empty());

    fn resolver() -> Vec<String> {
        vec!["pkg-a".to_string()]
    }
    set_stack_scopes_resolver(Some(resolver));
    // Still inactive: the installed resolver is only consulted once some
    // package actually has scoped rules.
    assert!(stack_scopes().is_empty());

    mark_scoped_rules_active();
    assert!(scoped_rules_active());
    assert_eq!(stack_scopes(), vec!["pkg-a".to_string()]);
    set_stack_scopes_resolver(None);
}
