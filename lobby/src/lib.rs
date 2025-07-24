#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::DiscordConfig;
use crate::instrumentation::RoomMetrics;
use crate::session::{AdminSession, AdminToken, Session};
use anyhow::Context as _;
use deadpool::Runtime;
use deadpool_redis::{Config, Pool as RedisPool};
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use diesel_async::AsyncPgConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};
use dotenvy::dotenv;
use instrumentation::QueueCounters;
use otlp::TracingFairing;
use rocket::config::ShutdownConfig;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::{Method, Status};
use rocket::response::Redirect;
use rocket::route::{Handler, Outcome};
use rocket::{catch, catchers, Request};
use rocket::{Data, Route};
use rocket_oauth2::OAuth2;
use rocket_prometheus::PrometheusMetrics;

use crate::index_manager::IndexManager;
use crate::jobs::{
    get_generation_callback, get_yaml_validation_callback, GenerationOutDir, GenerationQueue,
    YamlValidationQueue,
};
use views::queues::QueueTokens;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

pub mod config;
pub mod db;
pub mod error;
pub mod extractor;
pub mod generation;
pub mod index_manager;
pub mod instrumentation;
pub mod jobs;
pub mod otlp;
pub mod schema;
pub mod session;
pub mod utils;
pub mod views;
pub mod yaml;

pub struct Discord;

pub struct Context {
    db_pool: DieselPool<AsyncPgConnection>,
    redis_pool: RedisPool,
}

const CSS_VERSION: &str = std::env!("CSS_VERSION");
const JS_VERSION: &str = std::env!("JS_VERSION");

#[derive(Clone)]
pub struct TplContext<'a> {
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
    pub async fn from_session(module: &'a str, session: Session, ctx: &Context) -> Self {
        Self {
            cur_module: module,
            is_admin: session.is_admin,
            is_logged_in: session.is_logged_in,
            user_id: session.user_id,
            err_msg: session.retrieve_errors(ctx).await.unwrap(),
            warning_msg: session.retrieve_warnings(ctx).await.unwrap(),
            css_version: CSS_VERSION,
            js_version: JS_VERSION,
        }
    }
}

#[catch(401)]
async fn unauthorized<'r>(req: &'r Request<'r>) -> crate::error::Result<Redirect> {
    let ctx = req.rocket().state::<Context>().unwrap();

    let session = Session::from_request_sync(req);
    if session.is_logged_in {
        session
            .push_error("You don't have the rights to see this page", ctx)
            .await?;
        return Ok(Redirect::to("/"));
    }

    Ok(Redirect::to(format!(
        "/auth/login?redirect={}",
        req.uri().path()
    )))
}

#[derive(Clone)]
struct MetricsRoute(PrometheusMetrics, QueueCounters, RoomMetrics);

#[rocket::async_trait]
impl Handler for MetricsRoute {
    async fn handle<'r>(&self, req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r> {
        let rocket::outcome::Outcome::Success(_admin_session) = req.guard::<AdminSession>().await
        else {
            return Outcome::Error(Status::Forbidden);
        };

        let yaml_validation_queue = req.rocket().state::<YamlValidationQueue>().unwrap();
        let ctx = req.rocket().state::<Context>().unwrap();
        let stats = yaml_validation_queue.get_stats().await.unwrap();
        self.1.update_queue("yaml_validation", stats);
        let generation_queue = req.rocket().state::<GenerationQueue>().unwrap();
        let stats = generation_queue.get_stats().await.unwrap();
        self.1.update_queue("generation", stats);

        let mut conn = ctx.db_pool.get().await.unwrap();
        self.2.refresh(&mut conn).await.unwrap();

        self.0.handle(req, data).await
    }
}

impl From<MetricsRoute> for Vec<Route> {
    fn from(val: MetricsRoute) -> Self {
        vec![Route::new(Method::Get, "/", val)]
    }
}

#[rocket::main]
pub async fn main() -> crate::error::Result<()> {
    dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    let _sentry_guard = if let Ok(sentry_dsn) = std::env::var("SENTRY_DSN") {
        Some(sentry::init((
            sentry_dsn,
            sentry::ClientOptions {
                release: Some(format!("{}@{}", env!("CARGO_PKG_NAME"), env!("GIT_VERSION")).into()),
                environment: Some(
                    std::env::var("ROCKET_PROFILE")
                        .unwrap_or_else(|_| "dev".to_string())
                        .into(),
                ),
                traces_sample_rate: 1.0,
                ..Default::default()
            },
        )))
    } else {
        None
    };

    let otlp_endpoint = std::env::var("OTLP_ENDPOINT").ok();
    let _guard = otlp::init_tracing_subscriber(otlp_endpoint);

    let db_url = std::env::var("DATABASE_URL").expect("Provide a DATABASE_URL env variable");
    let valkey_url = std::env::var("VALKEY_URL").expect("Provide a VALKEY_URL env variable");
    let admin_token =
        AdminToken(std::env::var("ADMIN_TOKEN").expect("Provide a ADMIN_TOKEN env variable"));
    let generation_out_dir = GenerationOutDir(PathBuf::from(
        std::env::var("GENERATION_OUTPUT_DIR")
            .expect("Provide a GENERATION_OUTPUT_DIR env variable"),
    ));

    let db_pool = common::db::get_database_pool(&db_url, MIGRATIONS).await?;

    let redis_cfg = Config::from_url(&valkey_url);
    let redis_pool = redis_cfg.create_pool(Some(Runtime::Tokio1))?;

    let limits = Limits::default().limit("string", 2.megabytes());
    let shutdown_config = ShutdownConfig {
        grace: 0,
        mercy: 0,
        ..Default::default()
    };

    let figment = rocket::Config::figment()
        .merge(("limits", limits))
        .merge(("shutdown", shutdown_config));

    let discord_config = DiscordConfig::from_figment(&figment)?;
    let prometheus = PrometheusMetrics::new().with_request_filter(|request| {
        request.uri().path() != "/metrics"
            && request.uri().path().segments().last() != Some("claim_job")
    });
    prometheus
        .registry()
        .register(Box::new(common::db::QUERY_HISTOGRAM.clone()))
        .expect("Failed to register query histogram");

    let index_manager = IndexManager::new()?;
    if std::env::var("SKIP_APWORLDS_UPDATE").is_err() {
        index_manager.update().await?;
    }

    let yaml_validation_queue = YamlValidationQueue::builder("yaml_validation")
        .with_callback(get_yaml_validation_callback(db_pool.clone()))
        .with_reclaim_timeout(Duration::from_secs(10))
        .build(&valkey_url)
        .await
        .expect("Failed to create job queue for yaml validation");
    yaml_validation_queue.start_reclaim_checker();

    let generation_queue = GenerationQueue::builder("generation_queue")
        .with_callback(get_generation_callback(
            db_pool.clone(),
            generation_out_dir.0.clone(),
        ))
        .with_reclaim_timeout(Duration::from_secs(10))
        .build(&valkey_url)
        .await
        .expect("Failed to create job queue for generation");
    generation_queue.start_reclaim_checker();

    let queue_tokens = QueueTokens(HashMap::from([
        (
            "yaml_validation",
            std::env::var("YAML_VALIDATION_QUEUE_TOKEN").context("YAML_VALIDATION_QUEUE_TOKEN")?,
        ),
        (
            "generation",
            std::env::var("GENERATION_QUEUE_TOKEN").context("GENERATION_QUEUE_TOKEN")?,
        ),
    ]));
    let queue_counters = QueueCounters::new(prometheus.registry())?;
    let room_counters = RoomMetrics::new(prometheus.registry())?;

    let ctx = Context {
        db_pool,
        redis_pool,
    };

    rocket::custom(figment.clone())
        .attach(TracingFairing)
        .attach(prometheus.clone())
        .mount("/", views::routes())
        .mount("/", views::room_manager::routes())
        .mount("/", views::room_templates::routes())
        .mount("/", views::apworlds::routes())
        .mount("/", views::gen::routes())
        .mount("/auth/", views::auth::routes())
        .mount("/api/", views::api::routes())
        .mount(
            "/metrics",
            MetricsRoute(prometheus, queue_counters, room_counters),
        )
        .mount("/queues", views::queues::routes())
        .register("/", catchers![unauthorized])
        .manage(ctx)
        .manage(discord_config)
        .manage(figment)
        .manage(admin_token)
        .manage(generation_out_dir)
        .manage(index_manager)
        .manage(yaml_validation_queue)
        .manage(generation_queue)
        .manage(queue_tokens)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await
        .unwrap();

    Ok(())
}
