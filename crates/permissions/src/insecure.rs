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

fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_allowed_remote_denied_until_opt_in() {
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
}
