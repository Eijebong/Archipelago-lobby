use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use crate::db::{RoomFilter, RoomStatus, YamlWithoutContent};
use crate::utils::ZipFile;
use crate::{Context, TplContext};
use askama::Template;
use auth::{LoggedInSession, Session};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use http::header::CONTENT_DISPOSITION;
use itertools::Itertools;
use rocket::form::Form;
use rocket::http::{ContentType, CookieJar, Header};
use rocket::response::Redirect;
use rocket::routes;
use rocket::{get, post, uri, State};
use tracing::Instrument;
use uuid::Uuid;

use crate::db::{self, Room};
use crate::error::{Error, RedirectTo, Result, WithContext};

pub mod api;
pub mod apworlds;
pub mod auth;
pub mod room_manager;

#[derive(Template)]
#[template(path = "room.html")]
struct RoomTpl<'a> {
    base: TplContext<'a>,
    room: Room,
    author_name: String,
    yamls: Vec<(YamlWithoutContent, String)>,
    player_count: usize,
    unique_player_count: usize,
    unique_game_count: usize,
    is_closed: bool,
    has_room_url: bool,
    is_my_room: bool,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTpl<'a> {
    base: TplContext<'a>,
    open_rooms: Vec<Room>,
    your_rooms: Vec<Room>,
}

#[get("/")]
#[tracing::instrument(skip_all)]
async fn root<'a>(
    cookies: &CookieJar<'a>,
    session: Session,
    ctx: &State<Context>,
) -> Result<IndexTpl<'a>> {
    let open_rooms_filter = RoomFilter::new().with_status(RoomStatus::Open).with_max(10);
    let open_rooms_filter = if let Some(player_id) = session.user_id {
        open_rooms_filter
            .with_yamls_from(db::WithYaml::AndFor(player_id))
            .with_author(db::Author::IncludeUser(player_id))
    } else {
        open_rooms_filter
    };
    let open_rooms = db::list_rooms(open_rooms_filter, ctx).await?;

    let your_rooms = if let Some(player_id) = session.user_id {
        let your_rooms_filter = RoomFilter::new()
            .with_status(RoomStatus::Closed)
            .with_max(10)
            .with_yamls_from(db::WithYaml::OnlyFor(player_id))
            .with_private(true);
        db::list_rooms(your_rooms_filter, ctx).await?
    } else {
        vec![]
    };

    Ok(IndexTpl {
        base: TplContext::from_session("index", session, cookies),
        open_rooms,
        your_rooms,
    })
}

#[get("/room/<uuid>")]
#[tracing::instrument(skip(ctx, session, cookies))]
async fn room<'a>(
    uuid: Uuid,
    ctx: &State<Context>,
    session: Session,
    cookies: &CookieJar<'a>,
) -> Result<RoomTpl<'a>> {
    let (room, author_name) = db::get_room_and_author(uuid, ctx).await?;
    let mut yamls = db::get_yamls_for_room_with_author_names(uuid, ctx).await?;
    yamls.sort_by(|a, b| a.0.game.cmp(&b.0.game));
    let unique_player_count = yamls.iter().unique_by(|yaml| yaml.0.owner_id).count();
    let unique_game_count = yamls
        .iter()
        .filter(|yaml| !&yaml.0.game.starts_with("Random ("))
        .unique_by(|yaml| &yaml.0.game)
        .count();

    let is_my_room = session.is_admin || session.user_id == Some(room.author_id);
    let current_user_has_yaml_in_room = yamls
        .iter()
        .any(|yaml| Some(yaml.0.owner_id) == session.user_id)
        || is_my_room;

    Ok(RoomTpl {
        base: TplContext::from_session("room", session, cookies),
        player_count: yamls.len(),
        unique_player_count,
        unique_game_count,
        is_closed: room.is_closed(),
        has_room_url: !room.room_url.is_empty() && current_user_has_yaml_in_room,
        author_name,
        room,
        yamls,
        is_my_room,
    })
}

#[derive(rocket::form::FromForm)]
struct Yamls<'a> {
    yamls: Vec<&'a str>,
}

#[post("/room/<room_id>/upload", data = "<yaml_form>")]
#[tracing::instrument(skip(redirect_to, yaml_form, session, cookies, ctx))]
async fn upload_yaml(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    yaml_form: Form<Yamls<'_>>,
    mut session: LoggedInSession,
    cookies: &CookieJar<'_>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let room = db::get_room(room_id, ctx).await.context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let documents = crate::yaml::parse_raw_yamls(&yaml_form.yamls)?;
    let games = crate::yaml::parse_and_validate_yamls_for_room(
        &room,
        &documents,
        &mut session,
        cookies,
        ctx,
    )
    .await?;

    let mut conn = ctx.db_pool.get().await?;
    conn.transaction::<(), Error, _>(|conn| {
        async move {
            for (game_name, document, parsed) in games {
                db::add_yaml_to_room(
                    room_id,
                    session.0.user_id.unwrap(),
                    &game_name,
                    document,
                    parsed,
                    conn,
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .instrument(tracing::info_span!("add_yamls_to_room_transaction"))
    .await?;

    Ok(Redirect::to(uri!(room(room_id))))
}

#[get("/room/<room_id>/delete/<yaml_id>")]
#[tracing::instrument(skip(redirect_to, session, ctx))]
async fn delete_yaml(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    yaml_id: Uuid,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let room = db::get_room(room_id, ctx).await.context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let yaml = db::get_yaml_by_id(yaml_id, ctx).await?;

    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.author_id);
    if yaml.owner_id != session.user_id() && !is_my_room {
        Err(anyhow::anyhow!("Can't delete a yaml file that isn't yours"))?
    }

    db::remove_yaml(yaml_id, ctx).await?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

#[get("/room/<room_id>/yamls")]
#[tracing::instrument(skip(redirect_to, ctx, _session))]
async fn download_yamls<'a>(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    ctx: &State<Context>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{}", room_id));

    let room = db::get_room(room_id, ctx).await?;
    let yamls = db::get_yamls_for_room(room_id, ctx).await?;
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));

    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut emitted_names = HashSet::new();

    for yaml in yamls {
        let player_name = yaml.sanitized_name();
        let mut original_file_name = format!("{}.yaml", player_name);

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name) {
            loop {
                let new_file_name = format!("{}_{}.yaml", player_name, suffix);
                if !emitted_names.contains(&new_file_name) {
                    original_file_name = new_file_name;
                    break;
                }
                suffix += 1;
            }
        }
        writer.start_file(original_file_name.clone(), options)?;
        emitted_names.insert(original_file_name);
        writer.write_all(yaml.content.as_bytes())?;
    }

    let res = writer.finish()?;
    let value = format!(
        "attachment; filename=\"yamls-{}.zip\"",
        room.close_date.format("%Y-%m-%d_%H_%M_%S")
    );

    Ok(ZipFile {
        content: res.into_inner(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/yaml")]
pub(crate) struct YamlContent<'a> {
    content: String,
    headers: Header<'a>,
}

#[get("/room/<room_id>/download/<yaml_id>")]
#[tracing::instrument(skip(redirect_to, ctx))]
async fn download_yaml<'a>(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    yaml_id: Uuid,
    ctx: &State<Context>,
) -> Result<YamlContent<'a>> {
    redirect_to.set("/");

    Ok(api::download_yaml(room_id, yaml_id, ctx)
        .await
        .map_err(|api_err| api_err.error)?)
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

#[derive(rust_embed::RustEmbed)]
#[folder = "./static/"]
struct Asset;

pub fn routes() -> Vec<rocket::Route> {
    routes![
        root,
        room,
        upload_yaml,
        delete_yaml,
        download_yamls,
        download_yaml,
        dist
    ]
}
