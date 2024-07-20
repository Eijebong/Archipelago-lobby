use std::path::PathBuf;

use db::{DbInstrumentation, QUERY_HISTOGRAM};
use diesel::r2d2::Pool;
use diesel::pg::Pg;
use diesel::{r2d2::ConnectionManager, PgConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use dotenvy::dotenv;
use reqwest::Url;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::{CookieJar, Method, Status};
use rocket::response::Redirect;
use rocket::route::{Handler, Outcome};
use rocket::{catch, catchers, Request};
use rocket::{Data, Route};
use rocket_oauth2::OAuth2;
use rocket_prometheus::PrometheusMetrics;
use views::auth::{AdminSession, Session};

mod db;
mod error;
mod schema;
mod utils;
mod views;

pub struct Discord;

pub struct Context {
    db_pool: Pool<ConnectionManager<PgConnection>>,
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

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

fn run_migrations(
    connection: &mut impl MigrationHarness<Pg>,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    connection.run_pending_migrations(MIGRATIONS)?;

    Ok(())
}

#[catch(401)]
fn unauthorized(req: &Request) -> crate::error::Result<Redirect> {
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

struct AdminToken(String);
struct APWorldPath(PathBuf);

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let admin_token =
        AdminToken(std::env::var("ADMIN_TOKEN").expect("Plox provide a ADMIN_TOKEN env variable"));

    diesel::connection::set_default_instrumentation(|| {
        Some(Box::new(DbInstrumentation::default()))
    })
    .expect("Failed to set diesel instrumentation");


    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let db_pool = Pool::builder()
        .build(manager)
        .expect("Failed to create database pool, aborting");
    dbg!("hello");
    {
        let mut connection = db_pool
            .get()
            .expect("Failed to get database connection to run migrations");
        run_migrations(&mut connection).expect("Failed to run migrations");
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

    let limits = Limits::default().limit("bytes", 2.megabytes());

    let figment = rocket::Config::figment().merge(("limits", limits));
    let prometheus =
        PrometheusMetrics::new().with_request_filter(|request| request.uri().path() != "/metrics");
    prometheus
        .registry()
        .register(Box::new(QUERY_HISTOGRAM.clone()))
        .expect("Failed to register query histogram");

    let apworlds_index_path = std::path::PathBuf::from(
        std::env::var("APWORLDS_INDEX_PATH").expect("Provide a `APWORLDS_INDEX_PATH` env variable"),
    );
    let index_file = apworlds_index_path.join("index.toml");
    let index = apwm::Index::new(&index_file)?;

    let apworlds_path = APWorldPath(std::path::PathBuf::from(
        std::env::var("APWORLDS_PATH").expect("Provide a `APWORLDS_PATH` env variable"),
    ));

    rocket::custom(figment.clone())
        .attach(prometheus.clone())
        .mount("/", views::routes())
        .mount("/", views::room_manager::routes())
        .mount("/", views::apworlds::routes())
        .mount("/auth/", views::auth::routes())
        .mount("/metrics", AdminOnlyRoute(prometheus))
        .register("/", catchers![unauthorized])
        .manage(ctx)
        .manage(figment)
        .manage(admin_token)
        .manage(apworlds_path)
        .manage(index)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await?;

    Ok(())
}
