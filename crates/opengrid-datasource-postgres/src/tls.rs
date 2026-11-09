//! TLS to the database (issue #220).
//!
//! [`PostgresDataSource::connect_with_tls`](crate::PostgresDataSource::connect_with_tls)
//! takes any `tokio-postgres` TLS connector, so an application may bring
//! `native-tls` or its own. With the `tls` feature the crate also builds the
//! ready-made one: [`rustls_connector`], `rustls` with the `ring` provider (it
//! builds without system packages) and the Mozilla root certificates, plus the
//! roots of a private certificate authority when the caller gives them.
//!
//! The connection is only half of it: a request that is dropped cancels its
//! statement over a **second** connection (issue #217), and that one needs the
//! same TLS connector, or the cancel request would travel in clear text — or
//! not at all. [`Canceller`] carries the connector without making the data
//! source generic over it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::{CancelToken, NoTls, Socket};

type Request = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Sends the cancel request for a connection, over the same kind of channel the
/// connection itself uses.
#[derive(Clone)]
pub(crate) struct Canceller(Arc<dyn Fn(CancelToken) -> Request + Send + Sync>);

impl Canceller {
    /// For a connection without TLS.
    pub(crate) fn plain() -> Self {
        Self(Arc::new(|token| {
            Box::pin(async move {
                let _ = token.cancel_query(NoTls).await;
            })
        }))
    }

    /// For a connection made with `tls`.
    pub(crate) fn tls<T>(tls: T) -> Self
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        Self(Arc::new(move |token| {
            let tls = tls.clone();
            Box::pin(async move {
                let _ = token.cancel_query(tls).await;
            })
        }))
    }

    /// Best effort by nature: a statement that has just finished has nothing
    /// left to cancel, and a failed request changes nothing.
    pub(crate) async fn cancel(&self, token: CancelToken) {
        (self.0)(token).await;
    }
}

/// A TLS connector for [`PostgresDataSource::connect_with_tls`](crate::PostgresDataSource::connect_with_tls):
/// `rustls` with the `ring` provider, trusting the Mozilla root certificates.
///
/// `extra_roots_pem` adds the certificates of a private certificate authority
/// (PEM, one or more `CERTIFICATE` blocks) for a database whose certificate is
/// not signed by a public one. The server certificate is always verified,
/// against the host name of the connection string.
#[cfg(feature = "tls")]
pub fn rustls_connector(
    extra_roots_pem: Option<&[u8]>,
) -> Result<tokio_postgres_rustls::MakeRustlsConnect, opengrid_datasource::DataSourceError> {
    use opengrid_datasource::DataSourceError;
    use rustls_pki_types::{CertificateDer, pem::PemObject};

    let fail = |what: &str, error: &dyn std::fmt::Display| DataSourceError::Backend {
        message: format!("TLS: {what}: {error}"),
    };

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = extra_roots_pem {
        let mut added = 0;
        for certificate in CertificateDer::pem_slice_iter(pem) {
            let certificate =
                certificate.map_err(|error| fail("reading a root certificate", &error))?;
            roots
                .add(certificate)
                .map_err(|error| fail("a root certificate is not usable", &error))?;
            added += 1;
        }
        if added == 0 {
            return Err(fail(
                "the root certificates",
                &"no certificate found in the PEM text",
            ));
        }
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| fail("protocol versions", &error))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}
