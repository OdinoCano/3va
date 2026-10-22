// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Single source of TLS for the runtime's client builtins.
//!
//! Default build: tcp/ftp/imap/irc/mqtt/pop3 use OS-native TLS (native-tls),
//! rustls configs use aws-lc-rs. `fips` build: every TLS connection (those
//! builtins, fetch, EventSource, WebSocket, gRPC, PQ-TLS) goes through rustls
//! on the AWS-LC FIPS 140-3 module, and nothing reaches OpenSSL or ring.

use std::sync::Arc;

#[cfg(not(feature = "fips"))]
pub use native_tls::TlsStream;

/// native-tls lets the OS negotiate down to TLS 1.0 by default; every client
/// builtin goes through here so the floor is TLS 1.2 everywhere.
#[cfg(not(feature = "fips"))]
pub struct TlsConnector;

/// The TLS options a request named: the trust anchors it asked for (`ca`),
/// the client certificate (`cert`/`key`) and whether it wants the peer
/// verified (`rejectUnauthorized`). Shared by the native-tls (default) and
/// rustls/FIPS connectors, so it is always compiled.
#[derive(Debug, Clone, Default)]
pub struct TlsOptions {
    /// The PEM bundles the request trusts. In Node, `ca` *replaces* the
    /// default roots: a private CA means only that CA is trusted.
    pub ca: Vec<String>,
    /// The client certificate (PEM) for mTLS.
    pub cert: Option<String>,
    /// The client key (PEM) for mTLS.
    pub key: Option<String>,
    /// The passphrase of the client key, if it has one.
    pub passphrase: Option<String>,
    /// `false` when the request asked not to verify the peer.
    pub reject_unauthorized: bool,
}

impl TlsOptions {
    /// True when the request named a client certificate.
    pub fn has_client_certificate(&self) -> bool {
        self.cert.is_some() || self.key.is_some()
    }
}

impl TlsOptions {
    /// Read the options from the JSON the JS side sends.
    pub fn from_json(json: &str) -> TlsOptions {
        let mut out = TlsOptions::default();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return out;
        };
        match value.get("ca") {
            Some(serde_json::Value::String(pem)) => out.ca.push(pem.clone()),
            Some(serde_json::Value::Array(items)) => {
                for item in items {
                    if let serde_json::Value::String(pem) = item {
                        out.ca.push(pem.clone());
                    }
                }
            }
            _ => {}
        }
        if let Some(serde_json::Value::Bool(verify)) = value.get("rejectUnauthorized") {
            out.reject_unauthorized = *verify;
        }
        if let Some(serde_json::Value::String(pem)) = value.get("cert") {
            out.cert = Some(pem.clone());
        }
        if let Some(serde_json::Value::String(pem)) = value.get("key") {
            out.key = Some(pem.clone());
        }
        if let Some(serde_json::Value::String(pass)) = value.get("passphrase") {
            out.passphrase = Some(pass.clone());
        }
        out
    }
}

/// Node's `NODE_EXTRA_CA_CERTS`: add the PEM bundle as trust anchors, the same
/// variable the rustls/ureq path already honours (add_extra_ca_certs in
/// tcp.rs). Without this, `https`/`tls` cannot verify a private CA that
/// `fetch` and Node trust.
#[cfg(not(feature = "fips"))]
fn add_extra_ca_certs(builder: &mut native_tls::TlsConnectorBuilder) {
    let Ok(path) = std::env::var("NODE_EXTRA_CA_CERTS") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    match std::fs::read(&path) {
        Ok(pem) => {
            let mut reader = std::io::BufReader::new(pem.as_slice());
            for cert in rustls_pemfile::certs(&mut reader).flatten() {
                if let Ok(cert) = native_tls::Certificate::from_der(&cert) {
                    let _ = builder.add_root_certificate(cert);
                }
            }
        }
        Err(e) => eprintln!("[3va-tls] NODE_EXTRA_CA_CERTS: cannot read {path}: {e}"),
    }
}

#[cfg(not(feature = "fips"))]
impl TlsConnector {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> native_tls::Result<native_tls::TlsConnector> {
        let mut builder = native_tls::TlsConnector::builder();
        builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
        add_extra_ca_certs(&mut builder);
        builder.build()
    }

    /// The connector for a request that named its own trust anchors (`ca`) or
    /// asked not to verify the peer (`rejectUnauthorized: false`).
    pub fn with_options(opts: Option<&TlsOptions>) -> Result<native_tls::TlsConnector, String> {
        let mut builder = native_tls::TlsConnector::builder();
        builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
        if let Some(opts) = opts {
            if !opts.ca.is_empty() {
                // Node's `ca` *replaces* the default roots: a request that
                // names a private CA trusts only that CA (and not the system
                // roots, nor the NODE_EXTRA_CA_CERTS bundle).
                builder.disable_built_in_roots(true);
                for pem in &opts.ca {
                    let mut reader = std::io::BufReader::new(pem.as_bytes());
                    for cert in rustls_pemfile::certs(&mut reader).flatten() {
                        if let Ok(cert) = native_tls::Certificate::from_der(&cert) {
                            let _ = builder.add_root_certificate(cert);
                        }
                    }
                }
            }
            // Only reached for a request that asked `rejectUnauthorized: false`
            // *and* passed the caller's permission check (tcp.rs refuses the
            // request with ERR_TLS_VERIFICATION_SKIPPED unless `--allow-insecure`
            // or a loopback host was granted). The opt-in is enforced before
            // this connector is built, so the check is never disabled silently.
            if !opts.reject_unauthorized {
                builder.danger_accept_invalid_certs(true);
                builder.danger_accept_invalid_hostnames(true);
            }
            if opts.has_client_certificate() {
                let cert = opts.cert.clone().unwrap_or_default();
                let key = opts.key.clone().unwrap_or_default();
                if let Some(pass) = opts.passphrase.as_ref()
                    && !pass.is_empty()
                {
                    return Err(
                        "the client certificate could not be loaded: an encrypted private key is not supported"
                            .to_string(),
                    );
                }
                let identity = native_tls::Identity::from_pkcs8(cert.as_bytes(), key.as_bytes())
                    .map_err(|e| {
                        format!(
                            "the client certificate could not be loaded: {e} (the key must be a PEM \
                             PKCS#8 private key and the certificate a PEM chain)"
                        )
                    })?;
                builder.identity(identity);
            }
        }
        if opts.map(|o| o.ca.is_empty()).unwrap_or(true) {
            add_extra_ca_certs(&mut builder);
        }
        builder.build().map_err(|e| e.to_string())
    }
}

#[cfg(feature = "fips")]
pub use fips::{TlsConnector, TlsStream};

/// The crypto provider behind every rustls config the runtime builds.
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    #[cfg(feature = "fips")]
    return Arc::new(rustls::crypto::default_fips_provider());
    #[cfg(not(feature = "fips"))]
    return Arc::new(rustls::crypto::aws_lc_rs::default_provider());
}

/// Installs `provider()` as the process default (tonic's `ClientConfig::builder()`
/// relies on it). In a `fips` build it also runs the module's power-on self
/// tests and aborts startup if the module is not in FIPS mode.
pub fn init() -> anyhow::Result<()> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        #[cfg(feature = "fips")]
        aws_lc_rs::try_fips_mode().map_err(|e| format!("FIPS module self-test failed: {e}"))?;
        // Err only means another provider was already installed; keep it.
        let _ = provider().as_ref().clone().install_default();
        Ok(())
    })
    .clone()
    .map_err(anyhow::Error::msg)
}

/// `true` when the binary was built with the AWS-LC FIPS module.
pub const FIPS: bool = cfg!(feature = "fips");

/// ureq agent for fetch/EventSource. ureq's own default trusts only its
/// bundled webpki roots and ignores `NODE_EXTRA_CA_CERTS`; we install a rustls
/// config whose roots are the OS trust store plus that variable's PEM bundle,
/// so a private CA Node trusts is also trusted here. In a `fips` build the same
/// config carries the FIPS provider (ureq's default ring is not validated).
pub fn agent_builder() -> ureq::AgentBuilder {
    let builder = ureq::AgentBuilder::new();
    match super::tcp::pq_tls_client_config_native() {
        Ok(cfg) => builder.tls_config(cfg),
        // No roots → every HTTPS request fails verification anyway; surface it there.
        Err(e) => {
            eprintln!("[3va-tls] {e}");
            builder
        }
    }
}

#[cfg(feature = "fips")]
mod fips {
    use std::io::{self, Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;

    /// Drop-in for `native_tls::TlsConnector` (the subset the builtins use).
    pub struct TlsConnector(Arc<rustls::ClientConfig>);

    impl TlsConnector {
        pub fn new() -> Result<Self, String> {
            super::super::tcp::pq_tls_client_config_native().map(Self)
        }

        /// The connector for a request that named its own trust anchors.
        /// The FIPS build always verifies the peer: `rejectUnauthorized:
        /// false` is refused.
        pub fn with_options(opts: Option<&super::TlsOptions>) -> Result<Self, String> {
            let Some(opts) = opts else {
                return Self::new();
            };
            if !opts.reject_unauthorized {
                return Err(
                    "rejectUnauthorized: false is not supported in the FIPS build: the peer is always verified"
                        .to_string(),
                );
            }
            if opts.ca.is_empty() {
                return Self::new();
            }
            // Node's `ca` replaces the default roots.
            let mut roots = rustls::RootCertStore::empty();
            for pem in &opts.ca {
                let mut reader = std::io::BufReader::new(pem.as_bytes());
                for cert in rustls_pemfile::certs(&mut reader).flatten() {
                    let _ = roots.add(cert);
                }
            }
            super::super::tcp::pq_tls_client_config_from_roots(roots).map(Self)
        }

        pub fn connect(&self, host: &str, tcp: TcpStream) -> Result<TlsStream<TcpStream>, String> {
            let name = rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|e| format!("invalid server name {host:?}: {e}"))?;
            let conn = rustls::ClientConnection::new(self.0.clone(), name)
                .map_err(|e| format!("TLS init: {e}"))?;
            let mut s = rustls::StreamOwned::new(conn, tcp);
            while s.conn.is_handshaking() {
                s.conn
                    .complete_io(&mut s.sock)
                    .map_err(|e| format!("TLS handshake failed: {e}"))?;
            }
            Ok(TlsStream(Box::new(s)))
        }
    }

    /// Drop-in for `native_tls::TlsStream`. Boxed: `ClientConnection` is ~1 KB
    /// and would bloat every `Plain | Tls` connection enum in the builtins.
    pub struct TlsStream<S: Read + Write>(Box<rustls::StreamOwned<rustls::ClientConnection, S>>);

    impl<S: Read + Write> TlsStream<S> {
        pub fn get_ref(&self) -> &S {
            &self.0.sock
        }

        pub fn shutdown(&mut self) -> io::Result<()> {
            self.0.conn.send_close_notify();
            self.0.flush()
        }
    }

    impl<S: Read + Write> Read for TlsStream<S> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl<S: Read + Write> Write for TlsStream<S> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
}
