use diesel::connection::Instrumentation;
use diesel::{ConnectionError, ConnectionResult};
use diesel_async::AsyncPgConnection;
use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use once_cell::sync::Lazy;
use prometheus::{HistogramOpts, HistogramVec};
use rustls::Error as TLSError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::ring;
use rustls::pki_types::{ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

use std::sync::Arc;
use std::time::Instant;

#[derive(Default)]
pub struct DbInstrumentation {
    query_start: Option<Instant>,
}

pub static QUERY_HISTOGRAM: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("diesel_query_seconds", "SQL query duration").buckets(vec![
            0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.1, 1.0,
        ]),
        &["query"],
    )
    .expect("Failed to create query histogram")
});

impl Instrumentation for DbInstrumentation {
    fn on_connection_event(&mut self, event: diesel::connection::InstrumentationEvent<'_>) {
        match event {
            diesel::connection::InstrumentationEvent::StartQuery { .. } => {
                tracing::event!(tracing::Level::INFO, "Query started");
                self.query_start = Some(Instant::now());
            }
            diesel::connection::InstrumentationEvent::FinishQuery { query, .. } => {
                let Some(query_start) = self.query_start else {
                    return;
                };
                let elapsed = query_start.elapsed();
                let query = query.to_string().replace('\n', " ");
                let query = query.split("--").next().unwrap().trim();
                QUERY_HISTOGRAM
                    .with_label_values(&[query])
                    .observe(elapsed.as_secs_f64());
                tracing::event!(tracing::Level::INFO, %query, "Query finished");
            }
            diesel::connection::InstrumentationEvent::StartEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "StartEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::FinishEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "FinishEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::CacheQuery { .. } => {
                tracing::event!(tracing::Level::INFO, "CacheQuery");
            }
            diesel::connection::InstrumentationEvent::BeginTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "BeginTransaction");
            }
            diesel::connection::InstrumentationEvent::CommitTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "CommitTransaction");
            }
            diesel::connection::InstrumentationEvent::RollbackTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "RollbackTransaction");
            }
            _ => {}
        };
    }
}

#[derive(Debug)]
// Copied over from reqwest
pub(crate) struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer,
        _intermediates: &[rustls_pki_types::CertificateDer],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TLSError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TLSError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TLSError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

#[tracing::instrument]
fn establish_connection(config: &str) -> BoxFuture<'_, ConnectionResult<AsyncPgConnection>> {
    let fut = async {
        let rustls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth();

        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(rustls_config);
        let (client, conn) = tokio_postgres::connect(config, tls)
            .await
            .map_err(|e| ConnectionError::BadConnection(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Database connection: {e}");
            }
        });
        AsyncPgConnection::try_from(client).await
    };
    fut.boxed()
}

pub async fn get_database_pool(
    db_url: &str,
    migrations: EmbeddedMigrations,
) -> anyhow::Result<DieselPool<AsyncPgConnection>> {
    ring::default_provider()
        .install_default()
        .expect("Failed to set ring as crypto provider");

    diesel::connection::set_default_instrumentation(|| {
        Some(Box::new(DbInstrumentation::default()))
    })
    .expect("Failed to set diesel instrumentation");

    let mut config = ManagerConfig::default();
    config.custom_setup = Box::new(establish_connection);

    let mgr = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(db_url, config);
    let db_pool = DieselPool::builder(mgr)
        .build()
        .expect("Failed to create database pool, aborting");
    {
        let connection = db_pool
            .get()
            .await
            .expect("Failed to get database connection to run migrations");

        let mut async_wrapper: AsyncConnectionWrapper<
            deadpool::managed::Object<AsyncDieselConnectionManager<AsyncPgConnection>>,
        > = AsyncConnectionWrapper::from(connection);
        tokio::task::spawn_blocking(move || {
            async_wrapper.run_pending_migrations(migrations).unwrap();
        })
        .await?;
    }

    Ok(db_pool)
}
