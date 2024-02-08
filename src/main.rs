use diesel::r2d2::Pool;
use diesel::sqlite::Sqlite;
use diesel::{r2d2::ConnectionManager, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use rocket::data::{Limits, ToByteUnit};
use rocket::http::CookieJar;
use rocket::launch;
use uuid::Uuid;
use views::auth::Session;

mod api;
mod diesel_uuid;
mod error;
mod schema;
mod views;

pub struct Context {
    db_pool: Pool<ConnectionManager<SqliteConnection>>,
}

struct TplContext<'a> {
    is_admin: bool,
    cur_module: &'a str,
    user_id: Uuid,
    err_msg: Option<String>,
}

impl<'a> TplContext<'a> {
    pub fn from_session(module: &'a str, mut session: Session, cookies: &CookieJar) -> Self {
        let tpl = Self {
            cur_module: module,
            is_admin: session.is_admin,
            user_id: session.user_id,
            err_msg: session.err_msg.take(),
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

#[launch]
fn rocket() -> _ {
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

    let ctx = Context { db_pool };

    let limits = Limits::default().limit("bytes", 2.megabytes());

    let figment = rocket::Config::figment().merge(("limits", limits));

    rocket::custom(figment)
        .mount("/", views::routes())
        .mount("/admin/", views::admin::routes())
        .mount("/auth/", views::auth::routes())
        .manage(ctx)
}
