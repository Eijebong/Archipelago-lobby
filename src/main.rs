use std::sync::Arc;

use ap_lobby::db::{DbInstrumentation, QUERY_HISTOGRAM};
use ap_lobby::session::{AdminSession, AdminToken, Session};
use diesel::{ConnectionError, ConnectionResult};
use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::AsyncPgConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use dotenvy::dotenv;
use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use reqwest::Url;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::{CookieJar, Method, Status};
use rocket::response::Redirect;
use rocket::route::{Handler, Outcome};
use rocket::{catch, catchers, Request};
use rocket::{Data, Route};
use rocket_oauth2::OAuth2;
use rocket_prometheus::PrometheusMetrics;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{ServerName, UnixTime};
use rustls::Error as TLSError;
use rustls::{DigitallySignedStruct, SignatureScheme};

use ap_lobby::index_manager::IndexManager;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

mod otlp;
mod views;

pub struct Discord;

pub struct Context {
    db_pool: Pool<AsyncPgConnection>,
    yaml_validator_url: Option<Url>,
}

const CSS_VERSION: &str = std::env!("CSS_VERSION");
const JS_VERSION: &str = std::env!("JS_VERSION");

struct TplContext<'a> {
    is_admin: bool,
    is_logged_in: bool,
    cur_module: &'a str,
    user_id: Option<i64>,
    err_msg: Vec<String>,
    warning_msg: Vec<String>,
    css_version: &'a str,
    js_version: &'a str,
}

impl<'a> TplContext<'a> {
    pub fn from_session(module: &'a str, mut session: Session, cookies: &CookieJar) -> Self {
        let tpl = Self {
            cur_module: module,
            is_admin: session.is_admin,
            is_logged_in: session.is_logged_in,
            user_id: session.user_id,
            err_msg: session.err_msg.drain(..).collect(),
            warning_msg: session.warning_msg.drain(..).collect(),
            css_version: CSS_VERSION,
            js_version: JS_VERSION,
        };

        session
            .save(cookies)
            .expect("Failed to save session somehow");

        tpl
    }
}

#[catch(401)]
fn unauthorized(req: &Request) -> ap_lobby::error::Result<Redirect> {
    let mut session = Session::from_request_sync(req);
    if session.is_logged_in {
        let cookies = req.cookies();
        session
            .err_msg
            .push("You don't have the rights to see this page".into());
        session.save(cookies)?;
        return Ok(Redirect::to("/"));
    }

    Ok(Redirect::to(format!(
        "/auth/login?redirect={}",
        req.uri().path()
    )))
}

#[derive(Clone)]
struct AdminOnlyRoute<R: Handler + Clone>(R);

#[rocket::async_trait]
impl<R: Handler + Clone> Handler for AdminOnlyRoute<R> {
    async fn handle<'r>(&self, req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r> {
        let guard = req.guard::<AdminSession>().await;
        match guard {
            rocket::request::Outcome::Success(..) => self.0.handle(req, data).await,
            _ => Outcome::Error(Status::Forbidden),
        }
    }
}

impl<R: Handler + Clone> From<AdminOnlyRoute<R>> for Vec<Route> {
    fn from(val: AdminOnlyRoute<R>) -> Self {
        vec![Route::new(Method::Get, "/", val)]
    }
}

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }
    let otlp_endpoint = std::env::var("OTLP_ENDPOINT").ok();
    let _guard = otlp::init_tracing_subscriber(otlp_endpoint);

    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let admin_token =
        AdminToken(std::env::var("ADMIN_TOKEN").expect("Plox provide a ADMIN_TOKEN env variable"));

    diesel::connection::set_default_instrumentation(|| {
        Some(Box::new(DbInstrumentation::default()))
    })
    .expect("Failed to set diesel instrumentation");

    let mut config = ManagerConfig::default();
    config.custom_setup = Box::new(establish_connection);

    let mgr = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(db_url, config);
    let db_pool = Pool::builder(mgr)
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
            async_wrapper.run_pending_migrations(MIGRATIONS).unwrap();
        })
        .await?;
    }

    let yaml_validator_url = if let Ok(yaml_validator_url) = std::env::var("YAML_VALIDATOR_URL") {
        Some(
            yaml_validator_url
                .parse()
                .expect("Failed to parse YAML_VALIDATOR_URL"),
        )
    } else {
        None
    };

    let ctx = Context {
        db_pool,
        yaml_validator_url,
    };

    let limits = Limits::default().limit("string", 2.megabytes());

    let figment = rocket::Config::figment().merge(("limits", limits));
    let prometheus =
        PrometheusMetrics::new().with_request_filter(|request| request.uri().path() != "/metrics");
    prometheus
        .registry()
        .register(Box::new(QUERY_HISTOGRAM.clone()))
        .expect("Failed to register query histogram");

    let index_manager = IndexManager::new()?;
    //index_manager.update().await?;

    rocket::custom(figment.clone())
        .attach(prometheus.clone())
        .mount("/", views::routes())
        .mount("/", views::room_manager::routes())
        .mount("/", views::apworlds::routes())
        .mount("/auth/", views::auth::routes())
        .mount("/api/", views::api::routes())
        .mount("/metrics", AdminOnlyRoute(prometheus))
        .register("/", catchers![unauthorized])
        .manage(ctx)
        .manage(figment)
        .manage(admin_token)
        .manage(index_manager)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await
        .unwrap();

    Ok(())
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
fn establish_connection(config: &str) -> BoxFuture<ConnectionResult<AsyncPgConnection>> {
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
