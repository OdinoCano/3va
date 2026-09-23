//! Single source of TLS for the runtime's client builtins.
//!
//! Default build: tcp/ftp/imap/irc/mqtt/pop3 use OS-native TLS (native-tls),
//! rustls configs use aws-lc-rs. `fips` build: every TLS connection (those
//! builtins, fetch, EventSource, WebSocket, gRPC, PQ-TLS) goes through rustls
//! on the AWS-LC FIPS 140-3 module, and nothing reaches OpenSSL or ring.

use std::sync::Arc;

#[cfg(not(feature = "fips"))]
pub use native_tls::{TlsConnector, TlsStream};

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

/// ureq agent for fetch/EventSource. ureq's own default is ring, which is not
/// FIPS-validated, so the `fips` build hands it our rustls config.
pub fn agent_builder() -> ureq::AgentBuilder {
    let builder = ureq::AgentBuilder::new();
    #[cfg(feature = "fips")]
    let builder = match super::tcp::pq_tls_client_config_native() {
        Ok(cfg) => builder.tls_config(cfg),
        // No roots → every HTTPS request fails verification anyway; surface it there.
        Err(e) => {
            eprintln!("[3va-tls] {e}");
            builder
        }
    };
    builder
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
