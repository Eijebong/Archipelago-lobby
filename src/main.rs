use diesel::r2d2::Pool;
use diesel::sqlite::Sqlite;
use diesel::{r2d2::ConnectionManager, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use dotenvy::dotenv;
use reqwest::Url;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket::{catch, catchers, launch, Request};
use rocket_oauth2::OAuth2;
use views::auth::Session;

mod db;
mod diesel_uuid;
mod error;
mod schema;
mod views;

pub struct Discord;

pub struct Context {
    db_pool: Pool<ConnectionManager<SqliteConnection>>,
    yaml_validator_url: Option<Url>,
}

const CSS_VERSION: &str = std::env!("CSS_VERSION");

struct TplContext<'a> {
    is_admin: bool,
    is_logged_in: bool,
    cur_module: &'a str,
    user_id: Option<i64>,
    err_msg: Vec<String>,
    css_version: &'a str,
}

impl<'a> TplContext<'a> {
    pub fn from_session(module: &'a str, mut session: Session, cookies: &CookieJar) -> Self {
        let tpl = Self {
            cur_module: module,
            is_admin: session.is_admin,
            is_logged_in: session.is_logged_in,
            user_id: session.user_id,
            err_msg: session.err_msg.drain(..).collect(),
            css_version: CSS_VERSION,
        };

        session
            .save(cookies)
            .expect("Failed to save session somehow");

        tpl
    }
}

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

fn run_migrations(
    connection: &mut impl MigrationHarness<Sqlite>,
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

#[launch]
fn rocket() -> _ {
    dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let _admin_token =
        std::env::var("ADMIN_TOKEN").expect("Plox provide a ADMIN_TOKEN env variable");

    let manager = ConnectionManager::<SqliteConnection>::new(db_url);
    let db_pool = Pool::new(manager).expect("Failed to create database pool, aborting");
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

    rocket::custom(figment.clone())
        .mount("/", views::routes())
        .mount("/", views::room_manager::routes())
        .mount("/auth/", views::auth::routes())
        .register("/", catchers![unauthorized])
        .manage(ctx)
        .manage(figment)
        .attach(OAuth2::<Discord>::fairing("discord"))
}
