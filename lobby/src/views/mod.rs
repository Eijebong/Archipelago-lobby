use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::PathBuf;

use crate::db::{self, Room, RoomFilter};
use crate::error::Result;
use crate::session::Session;
use crate::{Context, TplContext};
use askama::Template;
use askama_web::WebTemplate;
use rocket::http::ContentType;
use rocket::routes;
use rocket::{get, State};

pub mod api;
pub mod apworlds;
pub mod auth;
pub mod filters;
pub mod manifest_editor;
pub mod queues;
pub mod room;
pub mod room_settings;
pub mod room_templates;
mod utils;

pub use room::YamlContent;

#[derive(Template, WebTemplate)]
#[template(path = "index/main.html")]
struct IndexTpl<'a> {
    base: TplContext<'a>,
    rooms: Vec<Room>,
    current_page: u64,
    max_pages: u64,
}

#[get("/?<page>")]
#[tracing::instrument(skip_all)]
async fn root<'a>(
    page: Option<u64>,
    session: Session,
    ctx: &'a State<Context>,
) -> Result<IndexTpl<'a>> {
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

    Ok(IndexTpl {
        base: TplContext::from_session("index", session, ctx).await,
        rooms,
        current_page,
        max_pages,
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
    let mut all_routes = routes![root, dist, favicon,];
    all_routes.extend(room::routes());
    all_routes
}
