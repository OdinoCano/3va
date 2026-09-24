// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Hashing and HTTPS for the package manager, in one place so the `fips` build
//! can route both through the AWS-LC FIPS 140-3 module (see
//! docs/10-security/10-fips.md). Default build: RustCrypto `sha2` plus reqwest's
//! native TLS. Call sites use the same API in both builds.

#[cfg(not(feature = "fips"))]
pub use sha2::{Digest, Sha256, Sha512};

#[cfg(feature = "fips")]
pub use aws::{Digest, Sha256, Sha512};

/// reqwest client builder for every registry/OSV/Sigstore request.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(feature = "fips")]
    let builder = builder.use_preconfigured_tls(aws::tls_config());
    builder
}

#[cfg(feature = "fips")]
mod aws {
    use aws_lc_rs::digest;
    use std::sync::Arc;

    /// The subset of `sha2::Digest` the package manager uses.
    pub trait Digest: Sized {
        fn new() -> Self;
        fn update(&mut self, data: impl AsRef<[u8]>);
        fn finalize(self) -> Vec<u8>;
    }

    macro_rules! aws_hash {
        ($name:ident, $alg:expr) => {
            pub struct $name(digest::Context);
            impl Digest for $name {
                fn new() -> Self {
                    Self(digest::Context::new(&$alg))
                }
                fn update(&mut self, data: impl AsRef<[u8]>) {
                    self.0.update(data.as_ref())
                }
                fn finalize(self) -> Vec<u8> {
                    self.0.finish().as_ref().to_vec()
                }
            }
        };
    }
    aws_hash!(Sha256, digest::SHA256);
    aws_hash!(Sha512, digest::SHA512);

    pub(super) fn tls_config() -> rustls::ClientConfig {
        let mut roots = rustls::RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for err in &native.errors {
            eprintln!("[3va-pm] native cert load warning: {err}");
        }
        for cert in native.certs {
            let _ = roots.add(cert);
        }
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::default_fips_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("FIPS provider supports TLS 1.2/1.3")
        .with_root_certificates(roots)
        .with_no_client_auth()
    }
}
