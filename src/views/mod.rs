use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use crate::{Context, TplContext};
use ap_lobby::db::{
    self, Author, Json, NewYaml, Room, RoomFilter, RoomId, YamlId, YamlWithoutContent,
};
use ap_lobby::error::{Error, RedirectTo, Result, WithContext};
use ap_lobby::events::RoomEventsReceiver;
use ap_lobby::index_manager::IndexManager;
use ap_lobby::jobs::YamlValidationQueue;
use ap_lobby::session::{LoggedInSession, Session};
use ap_lobby::utils::ZipFile;
use ap_lobby::yaml::YamlValidationResult;
use apwm::{World, WorldOrigin};
use askama::Template;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use futures_util::stream::StreamExt;
use http::header::CONTENT_DISPOSITION;
use itertools::Itertools;
use rocket::form::Form;
use rocket::http::{ContentType, CookieJar, Header};
use rocket::response::stream::{Event, EventStream};
use rocket::response::Redirect;
use rocket::routes;
use rocket::{get, post, uri, State};
use semver::Version;
use tracing::Instrument;

pub mod api;
pub mod apworlds;
pub mod auth;
pub mod filters;
pub mod manifest_editor;
pub mod queues;
pub mod room_manager;
pub mod room_settings;
pub mod room_templates;
mod utils;

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
#[template(path = "room_manager/room_apworlds.html")]
struct RoomApworldsTpl<'a> {
    base: TplContext<'a>,
    is_my_room: bool,
    supported_apworlds: Vec<(String, (World, Version))>,
    unsupported_apworlds: Vec<(String, (World, Version))>,
    room: Room,
}

#[derive(Template)]
#[template(path = "index.html")]
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
    cookies: &'a CookieJar<'_>,
    ctx: &'a State<Context>,
) -> Result<IndexTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let current_page = page.unwrap_or(1);

    let (rooms, max_pages) = if let Some(user_id) = session.user_id {
        let your_rooms_filter = RoomFilter::default()
            .with_author(Author::User(user_id))
            .with_yamls_from(db::WithYaml::AndFor(user_id));

        db::list_rooms(your_rooms_filter, Some(current_page), &mut conn).await?
    } else {
        (vec![], 1)
    };

    if rooms.is_empty() && current_page != 1 {
        return Box::pin(root(None, session, cookies, ctx)).await;
    }

    Ok(IndexTpl {
        base: TplContext::from_session("index", session, cookies),
        rooms,
        current_page,
        max_pages,
    })
}

#[get("/room/<room_id>")]
#[tracing::instrument(skip(ctx, session, cookies))]
async fn room<'a>(
    room_id: RoomId,
    ctx: &State<Context>,
    session: Session,
    cookies: &CookieJar<'a>,
) -> Result<RoomTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let (room, author_name) = db::get_room_and_author(room_id, &mut conn).await?;
    let mut yamls = db::get_yamls_for_room_with_author_names(room_id, &mut conn).await?;
    yamls.sort_by(|a, b| a.0.game.cmp(&b.0.game));
    let unique_player_count = yamls.iter().unique_by(|yaml| yaml.0.owner_id).count();
    let unique_game_count = yamls
        .iter()
        .filter(|yaml| !&yaml.0.game.starts_with("Random ("))
        .unique_by(|yaml| &yaml.0.game)
        .count();

    let is_my_room = session.is_admin || session.user_id == Some(room.settings.author_id);
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
        has_room_url: !room.settings.room_url.is_empty() && current_user_has_yaml_in_room,
        author_name,
        room,
        yamls,
        is_my_room,
    })
}

#[get("/room/<room_id>/events")]
fn room_events(
    room_id: RoomId,
    room_events_receiver: &State<RoomEventsReceiver>,
) -> EventStream![] {
    let stream = room_events_receiver.stream_for_room(room_id);

    EventStream! {
        futures_util::pin_mut!(stream);
        while let Some(msg) = stream.next().await {
            yield Event::json(&msg)
        }
    }
}

#[derive(rocket::form::FromForm)]
struct Yamls<'a> {
    yamls: Vec<&'a str>,
}

#[post("/room/<room_id>/upload", data = "<yaml_form>")]
#[tracing::instrument(skip(
    redirect_to,
    yaml_form,
    session,
    cookies,
    ctx,
    index_manager,
    yaml_validation_queue
))]
async fn upload_yaml(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    yaml_form: Form<Yamls<'_>>,
    mut session: LoggedInSession,
    cookies: &CookieJar<'_>,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn)
        .await
        .context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let documents = ap_lobby::yaml::parse_raw_yamls(&yaml_form.yamls)?;
    let games = ap_lobby::yaml::parse_and_validate_yamls_for_room(
        &room,
        &documents,
        &mut session,
        cookies,
        yaml_validation_queue,
        index_manager,
        &mut conn,
    )
    .await?;

    conn.transaction::<(), Error, _>(|conn| {
        async move {
            for YamlValidationResult {
                ref game_name,
                document,
                parsed,
                features,
                validation_status,
                apworlds,
                error,
            } in games
            {
                let new_yaml = NewYaml {
                    id: YamlId::new_v4(),
                    owner_id: session.user_id(),
                    room_id,
                    content: document,
                    player_name: &parsed.name,
                    game: game_name,
                    features: Json(features),
                    validation_status,
                    apworlds,
                    last_error: error,
                };

                db::add_yaml_to_room(new_yaml, conn).await?;
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
    room_id: RoomId,
    yaml_id: YamlId,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn)
        .await
        .context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn).await?;

    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);
    if yaml.owner_id != session.user_id() && !is_my_room {
        Err(anyhow::anyhow!("Can't delete a yaml file that isn't yours"))?
    }

    db::remove_yaml(yaml_id, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

#[get("/room/<room_id>/yamls")]
#[tracing::instrument(skip(redirect_to, ctx, _session))]
async fn download_yamls<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    ctx: &State<Context>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let yamls = db::get_yamls_for_room(room_id, &mut conn).await?;
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut emitted_names = HashSet::new();

    for yaml in yamls {
        let player_name = yaml.sanitized_name();
        let mut original_file_name = format!("{}.yaml", player_name);

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name.to_lowercase()) {
            loop {
                let new_file_name = format!("{}_{}.yaml", player_name, suffix);
                if !emitted_names.contains(&new_file_name.to_lowercase()) {
                    original_file_name = new_file_name;
                    break;
                }
                suffix += 1;
            }
        }
        writer.start_file(original_file_name.clone(), options)?;
        emitted_names.insert(original_file_name.to_lowercase());
        writer.write_all(yaml.content.as_bytes())?;
    }

    let res = writer.finish()?;
    let value = format!(
        "attachment; filename=\"yamls-{}.zip\"",
        room.settings.close_date.format("%Y-%m-%d_%H_%M_%S")
    );

    Ok(ZipFile {
        content: res.into_inner(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[get("/room/<room_id>/worlds")]
#[tracing::instrument(skip(redirect_to, ctx, index_manager, session, cookies))]
async fn room_worlds<'a>(
    room_id: RoomId,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    redirect_to: &RedirectTo,
    cookies: &CookieJar<'_>,
    ctx: &State<Context>,
) -> Result<RoomApworldsTpl<'a>> {
    redirect_to.set(&format!("/room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);

    let index = index_manager.index.read().await.clone();
    let (worlds, resolve_errors) = room.settings.manifest.resolve_with(&index);
    if !resolve_errors.is_empty() {
        Err(anyhow::anyhow!(
            "Error while resolving apworlds for this room: {}",
            resolve_errors.iter().join("\n")
        ))?
    }

    let (mut supported_apworlds, mut unsupported_apworlds): (Vec<_>, Vec<_>) =
        worlds.into_iter().partition(|(_, (world, version))| {
            world.supported && matches!(world.get_version(version).unwrap(), WorldOrigin::Supported)
        });

    supported_apworlds.sort_by_cached_key(|(_, (world, _))| world.display_name.clone());
    unsupported_apworlds.sort_by_cached_key(|(_, (world, _))| world.display_name.clone());

    Ok(RoomApworldsTpl {
        base: TplContext::from_session("room", session.0, cookies),
        is_my_room,
        supported_apworlds,
        unsupported_apworlds,
        room,
    })
}

#[get("/room/<room_id>/worlds/download_all")]
#[tracing::instrument(skip(ctx, _session, index_manager, redirect_to))]
async fn room_download_all_worlds<'a>(
    room_id: RoomId,
    _session: LoggedInSession,
    index_manager: &'a State<IndexManager>,
    redirect_to: &'a RedirectTo,
    ctx: &'a State<Context>,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;

    Ok(index_manager
        .download_apworlds(&room.settings.manifest)
        .await?)
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
    room_id: RoomId,
    yaml_id: YamlId,
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
    routes![
        root,
        room,
        room_events,
        room_worlds,
        room_download_all_worlds,
        upload_yaml,
        delete_yaml,
        download_yamls,
        download_yaml,
        dist,
        favicon,
    ]
}
