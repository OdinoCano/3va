// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Plaintext network protocols (`http://`, `ws://`, FTP, and IMAP/POP3/IRC/MQTT/gRPC
//! without TLS) are disabled by default for every host except loopback. The
//! user opts in with `--allow-insecure`; granting `--allow-net` alone is not
//! enough, so a script can't silently downgrade to an unencrypted channel.

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};

// ponytail: process-wide switch, set once by the CLI before any script runs.
static ALLOW_INSECURE: AtomicBool = AtomicBool::new(false);

/// Enables plaintext protocols to non-loopback hosts (`--allow-insecure`).
pub fn set_allow_insecure(allow: bool) {
    ALLOW_INSECURE.store(allow, Ordering::Relaxed);
}

/// True when a plaintext connection to `host` may proceed: either the user
/// passed `--allow-insecure`, or `host` is loopback (traffic never leaves the machine).
pub fn plaintext_allowed(host: &str) -> bool {
    ALLOW_INSECURE.load(Ordering::Relaxed) || is_loopback(host)
}

/// Error message thrown to JS when a plaintext connection is refused.
pub fn plaintext_denied_message(protocol: &str, host: &str) -> String {
    format!(
        "Insecure protocol {protocol} to '{host}' is disabled by default. \
         Use the TLS variant, or run with --allow-insecure"
    )
}

/// True when SSH host-key verification may be skipped for `host`: either the
/// user passed `--allow-insecure` (accept any server key, like the plaintext
/// policy) or `host` is loopback (the peer is on this machine, so the
/// man-in-the-middle a key check defends against cannot intercept the
/// connection).
pub fn insecure_ssh_allowed(host: &str) -> bool {
    ALLOW_INSECURE.load(Ordering::Relaxed) || is_loopback(host)
}

/// True when the TLS certificate verification may be skipped for `host`.
///
/// The same rule as the SSH host-key check: either the user passed
/// `--allow-insecure` or `host` is loopback. Without it, a script (or a
/// dependency) could turn `rejectUnauthorized: false` into "accept any
/// certificate" for every host, and the TLS verification would be nothing
/// but a setting in a package.json.
pub fn insecure_tls_allowed(host: &str) -> bool {
    ALLOW_INSECURE.load(Ordering::Relaxed) || is_loopback(host)
}

/// Error message thrown to JS when a TLS verification skip is refused.
pub fn insecure_tls_denied_message(host: &str) -> String {
    format!(
        "Skipping the TLS certificate verification for '{host}' is disabled by default. \
         Pass rejectUnauthorized: true (the default), or run with --allow-insecure"
    )
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag is process-wide, so the tests that change it are serialized:
    /// without this, one test can see the flag another one set.
    static FLAG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn loopback_allowed_remote_denied_until_opt_in() {
        let _guard = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        assert!(plaintext_allowed("localhost"));
        assert!(plaintext_allowed("127.0.0.1"));
        assert!(plaintext_allowed("127.8.9.10"));
        assert!(plaintext_allowed("[::1]"));
        assert!(!plaintext_allowed("example.com"));
        assert!(!plaintext_allowed("10.0.0.1"));
        set_allow_insecure(true);
        assert!(plaintext_allowed("example.com"));
        set_allow_insecure(false);
    }

    #[test]
    fn insecure_ssh_allowed_follows_loopback_and_opt_in() {
        let _guard = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        assert!(insecure_ssh_allowed("localhost"));
        assert!(insecure_ssh_allowed("127.0.0.1"));
        assert!(!insecure_ssh_allowed("example.com"));
        set_allow_insecure(true);
        assert!(insecure_ssh_allowed("example.com"));
        set_allow_insecure(false);
    }

    #[test]
    fn insecure_tls_allowed_follows_loopback_and_opt_in() {
        let _guard = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        assert!(insecure_tls_allowed("localhost"));
        assert!(insecure_tls_allowed("127.0.0.1"));
        assert!(insecure_tls_allowed("[::1]"));
        assert!(!insecure_tls_allowed("example.com"));
        assert!(!insecure_tls_allowed("10.0.0.1"));
        set_allow_insecure(true);
        assert!(insecure_tls_allowed("example.com"));
        set_allow_insecure(false);
    }

    #[test]
    fn insecure_tls_denied_message_names_the_host_and_the_opt_in() {
        let msg = insecure_tls_denied_message("example.com");
        assert!(msg.contains("example.com"), "{msg}");
        assert!(msg.contains("--allow-insecure"), "{msg}");
    }

    #[test]
    fn plaintext_denied_message_names_protocol_host_and_opt_in() {
        let msg = plaintext_denied_message("http", "example.com");
        assert!(msg.contains("http"), "{msg}");
        assert!(msg.contains("example.com"), "{msg}");
        assert!(msg.contains("--allow-insecure"), "{msg}");
    }
}
