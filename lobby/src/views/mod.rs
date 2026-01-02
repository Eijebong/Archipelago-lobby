use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::PathBuf;

use crate::db::{self, Room, RoomFilter};
use crate::error::Result;
use crate::session::Session;
use crate::{Context, TplContext};
use askama::Template;
use askama_web::WebTemplate;
use diesel::IntoSql;
use diesel_async::RunQueryDsl;
use redis::AsyncCommands;
use rocket::http::{ContentType, Status};
use rocket::routes;
use rocket::serde::json::Json;
use rocket::{get, State};
use serde::Serialize;

pub mod api;
pub mod apworlds;
pub mod auth;
pub mod filters;
pub mod manifest_editor;
pub mod options_gen;
pub mod queues;
pub mod room;
pub mod room_settings;
pub mod room_templates;
mod utils;

pub use room::YamlContent;

#[derive(rocket::Responder)]
enum Index<'a> {
    RoomList(IndexTpl<'a>),
    Help(HelpTpl<'a>),
}

#[derive(Template, WebTemplate)]
#[template(path = "index/main.html")]
struct IndexTpl<'a> {
    base: TplContext<'a>,
    rooms: Vec<Room>,
    current_page: u64,
    max_pages: u64,
}

#[derive(Template, WebTemplate)]
#[template(path = "index/help.html")]
struct HelpTpl<'a> {
    base: TplContext<'a>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    db: &'static str,
    redis: &'static str,
}

#[get("/health")]
async fn health(ctx: &State<Context>) -> (Status, Json<HealthResponse>) {
    let db_ok = match ctx.db_pool.get().await {
        Ok(mut conn) => diesel::select(1.into_sql::<diesel::sql_types::Integer>())
            .execute(&mut conn)
            .await
            .is_ok(),
        Err(_) => false,
    };

    let redis_ok = match ctx.redis_pool.get().await {
        Ok(mut conn) => {
            let result: std::result::Result<String, _> = conn.ping().await;
            result.is_ok()
        }
        Err(_) => false,
    };

    let all_ok = db_ok && redis_ok;
    let response = HealthResponse {
        status: if all_ok { "healthy" } else { "unhealthy" },
        db: if db_ok { "ok" } else { "error" },
        redis: if redis_ok { "ok" } else { "error" },
    };

    let status = if all_ok {
        Status::Ok
    } else {
        Status::ServiceUnavailable
    };

    (status, Json(response))
}

#[get("/?<page>")]
#[tracing::instrument(skip_all)]
async fn root<'a>(
    page: Option<u64>,
    session: Session,
    ctx: &'a State<Context>,
) -> Result<Index<'a>> {
    if !session.is_logged_in {
        return Ok(Index::Help(help(session, ctx).await?));
    }

    let mut conn = ctx.db_pool.get().await?;
    let current_page = page.unwrap_or(1);

    let (rooms, max_pages) = if let Some(user_id) = session.user_id {
        let your_rooms_filter = RoomFilter::default()
            .with_author(db::Author::User(user_id))
            .with_yamls_from(db::WithYaml::AndFor(user_id));

        db::list_rooms(your_rooms_filter, Some(current_page), &mut conn).await?
    } else {
        (vec![], 1)
    };

    if rooms.is_empty() && current_page != 1 {
        return Box::pin(root(None, session, ctx)).await;
    }

    Ok(Index::RoomList(IndexTpl {
        base: TplContext::from_session("index", session, ctx).await,
        rooms,
        current_page,
        max_pages,
    }))
}

#[get("/help")]
#[tracing::instrument(skip_all)]
async fn help<'a>(session: Session, ctx: &'a State<Context>) -> Result<HelpTpl<'a>> {
    Ok(HelpTpl {
        base: TplContext::from_session("index", session, ctx).await,
    })
}

#[get("/static/<file..>")]
#[tracing::instrument]
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let filename = file.display().to_string();
    let asset = Asset::get(&filename)?;
    let content_type = file
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

#[get("/favicon.ico")]
#[tracing::instrument]
fn favicon() -> Option<(ContentType, Cow<'static, [u8]>)> {
    let asset = Asset::get("images/favicon.ico")?;
    let content_type = ContentType::Icon;

    Some((content_type, asset.data))
}

#[derive(rust_embed::RustEmbed)]
#[folder = "./static/"]
struct Asset;

pub fn routes() -> Vec<rocket::Route> {
    let mut all_routes = routes![root, dist, favicon, help, health];
    all_routes.extend(room::routes());
    all_routes
}
