use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Write};

use crate::db::{self, BundleId, Json, NewYaml, Room, RoomId, YamlId, YamlWithoutContent};
use crate::error::{Error, RedirectTo, Result, WithContext};
use crate::generation::get_generation_info;
use crate::index_manager::IndexManager;
use crate::jobs::{GenerationOutDir, YamlValidationQueue};
use crate::session::{LoggedInSession, Session};
use crate::utils::{NamedBuf, ZipFile};
use crate::views::api;
use crate::views::filters;
use crate::yaml::YamlValidationResult;
use crate::{Context, TplContext};
use askama::Template;
use askama_web::WebTemplate;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use http::header::CONTENT_DISPOSITION;
use itertools::Itertools;
use rocket::form::Form;
use rocket::http::Header;
use rocket::response::Redirect;
use rocket::{get, post, uri, State};
use tracing::Instrument;
use zip::ZipArchive;

#[derive(Template, WebTemplate)]
#[template(path = "room/main.html")]
pub struct RoomTpl<'a> {
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
    room_info: Option<db::RoomInfo>,
    current_user_has_yaml_in_room: bool,
    game_display_names: HashMap<String, String>,
}

impl RoomTpl<'_> {
    fn get_game_display_name<'a>(&'a self, game: &'a str) -> &'a str {
        self.game_display_names
            .get(game)
            .map(|s| s.as_str())
            .unwrap_or(game)
    }
}

#[get("/room/<room_id>")]
#[tracing::instrument(skip(ctx, session, index_manager))]
pub async fn room<'a>(
    room_id: RoomId,
    ctx: &State<Context>,
    session: Session,
    index_manager: &State<IndexManager>,
) -> Result<RoomTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let (room, author_name, room_info) = db::get_room_and_author(room_id, &mut conn).await?;
    let mut yamls = db::get_yamls_for_room_with_author_names(room_id, &mut conn).await?;

    yamls.sort_by(|a, b| a.0.game.to_lowercase().cmp(&b.0.game.to_lowercase()));
    let unique_player_count = yamls.iter().unique_by(|yaml| yaml.0.owner_id).count();
    let unique_game_count = yamls
        .iter()
        .filter(|yaml| !&yaml.0.game.starts_with("Random ("))
        .unique_by(|yaml| &yaml.0.game)
        .count();

    let is_my_room = session.is_admin || session.user_id == Some(room.settings.author_id);
    let user_has_yaml = yamls
        .iter()
        .any(|yaml| Some(yaml.0.owner_id) == session.user_id);
    let current_user_has_yaml_in_room = user_has_yaml || is_my_room;

    // Build game name to display name map from the index
    let game_display_names = {
        let index = index_manager.index.read().await;
        yamls
            .iter()
            .map(|(yaml, _)| &yaml.game)
            .unique()
            .filter_map(|game_name| {
                index
                    .get_world_by_name(game_name)
                    .map(|world| (game_name.clone(), world.display_name.clone()))
            })
            .collect::<HashMap<_, _>>()
    };

    Ok(RoomTpl {
        base: TplContext::from_session("room", session, ctx).await,
        player_count: yamls.len(),
        unique_player_count,
        unique_game_count,
        is_closed: room.is_closed(),
        has_room_url: !room.settings.room_url.is_empty() && current_user_has_yaml_in_room,
        author_name,
        room,
        yamls,
        is_my_room,
        room_info,
        current_user_has_yaml_in_room: user_has_yaml,
        game_display_names,
    })
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
    ctx,
    index_manager,
    yaml_validation_queue
))]
async fn upload_yaml(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    yaml_form: Form<Yamls<'_>>,
    mut session: LoggedInSession,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room_record = db::get_room(room_id, &mut conn)
        .await
        .context("Unknown room")?;
    if room_record.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let documents = crate::yaml::parse_raw_yamls(&yaml_form.yamls)?;
    let games = crate::yaml::parse_and_validate_yamls_for_room(
        &room_record,
        &documents,
        &mut session,
        yaml_validation_queue,
        index_manager,
        &mut conn,
    )
    .await?;

    let mut all_disabled_games = HashSet::new();
    let mut all_unsupported_games = HashSet::new();
    for YamlValidationResult {
        disabled_games,
        unsupported_games,
        ..
    } in &games
    {
        all_disabled_games.extend(disabled_games.clone());
        all_unsupported_games.extend(unsupported_games.clone());
    }

    if let Some(msg) = build_unsupported_games_messages(
        all_unsupported_games,
        all_disabled_games,
        !room_record.settings.allow_unsupported,
    ) {
        if !room_record.settings.allow_unsupported {
            session.0.push_error(&msg, ctx).await?;
            return Ok(Redirect::to(uri!(room(room_id))));
        } else {
            session.0.push_warning(&msg, ctx).await?;
        }
    }

    conn.transaction::<(), Error, _>(|conn| {
        async move {
            let bundle_id = BundleId::new_v4();

            for YamlValidationResult {
                ref game_name,
                document,
                parsed,
                features,
                apworlds,
                error,
                validation_status,
                ..
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
                    bundle_id,
                    password: None,
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

fn build_unsupported_games_messages(
    unsupported_games: HashSet<String>,
    disabled_games: HashSet<String>,
    error: bool,
) -> Option<String> {
    if disabled_games.is_empty() && unsupported_games.is_empty() {
        return None;
    }

    let format_game_list = |games: HashSet<String>| -> String {
        games.iter().sorted().map(|g| format!("'{}'", g)).join(", ")
    };

    let mut message_parts = Vec::new();

    if !disabled_games.is_empty() {
        message_parts.push(format!(
            "The following games are disabled for this room: {}",
            format_game_list(disabled_games)
        ));
    }

    if !unsupported_games.is_empty() {
        message_parts.push(format!(
            "The following games are not supported on this lobby: {}",
            format_game_list(unsupported_games)
        ));
    }

    let base_message = message_parts.join("\n");

    if error {
        Some(base_message)
    } else {
        Some(format!(
            "{}.\nUploading anyway since the room owner allowed it.",
            base_message
        ))
    }
}

#[get("/room/<room_id>/delete_bundle/<bundle_id>")]
#[tracing::instrument(skip(redirect_to, session, ctx))]
pub async fn delete_bundle(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    bundle_id: BundleId,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room_record = db::get_room(room_id, &mut conn)
        .await
        .context("Unknown room")?;
    if room_record.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let bundle = db::get_bundle_by_id(bundle_id, &mut conn).await?;

    let is_my_room =
        session.0.is_admin || session.0.user_id == Some(room_record.settings.author_id);
    if bundle.owner_id() != session.user_id() && !is_my_room {
        Err(anyhow::anyhow!(
            "Can't delete a YAML bundle that isn't yours"
        ))?
    }

    db::remove_bundle(bundle_id, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{room_id}")))
}

#[get("/room/<room_id>/delete/<yaml_id>")]
#[tracing::instrument(skip(redirect_to, session, ctx))]
pub async fn delete_yaml(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    yaml_id: YamlId,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room_record = db::get_room(room_id, &mut conn)
        .await
        .context("Unknown room")?;
    if room_record.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn).await?;

    let is_my_room =
        session.0.is_admin || session.0.user_id == Some(room_record.settings.author_id);
    if yaml.owner_id != session.user_id() && !is_my_room {
        Err(anyhow::anyhow!("Can't delete a yaml file that isn't yours"))?
    }

    if room_record.settings.is_bundle_room && !is_my_room {
        Err(anyhow::anyhow!(
            "Can't delete an individual yaml in a bundled room, delete the whole YAML bundle"
        ))?
    }

    db::remove_yaml(yaml_id, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{room_id}")))
}

#[get("/room/<room_id>/yamls")]
#[tracing::instrument(skip(redirect_to, ctx, _session))]
pub async fn download_yamls<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    ctx: &State<Context>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room_record = db::get_room(room_id, &mut conn).await?;
    let yamls = db::get_yamls_for_room(room_id, &mut conn).await?;
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut emitted_names = HashSet::new();

    for yaml in yamls {
        let player_name = yaml.sanitized_name();
        let mut original_file_name = format!("{player_name}.yaml");

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name.to_lowercase()) {
            loop {
                let new_file_name = format!("{player_name}_{suffix}.yaml");
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
        room_record.settings.close_date.format("%Y-%m-%d_%H_%M_%S")
    );

    Ok(ZipFile {
        content: res.into_inner(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[get("/room/<room_id>/bundles")]
#[tracing::instrument(skip(redirect_to, ctx, _session))]
pub async fn download_bundles<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    ctx: &State<Context>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room_record = db::get_room(room_id, &mut conn).await?;
    let bundles = db::get_bundles_for_room(room_id, &mut conn).await?;
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut emitted_names = HashSet::new();

    for bundle in bundles {
        let file_name = bundle.file_name();
        let mut original_file_name = format!("{file_name}.yaml");

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name.to_lowercase()) {
            loop {
                let new_file_name = format!("{file_name}_{suffix}.yaml");
                if !emitted_names.contains(&new_file_name.to_lowercase()) {
                    original_file_name = new_file_name;
                    break;
                }
                suffix += 1;
            }
        }
        writer.start_file(original_file_name.clone(), options)?;
        emitted_names.insert(original_file_name.to_lowercase());
        writer.write_all(bundle.as_yaml().as_bytes())?;
    }

    let res = writer.finish()?;
    let value = format!(
        "attachment; filename=\"yamls-{}.zip\"",
        room_record.settings.close_date.format("%Y-%m-%d_%H_%M_%S")
    );

    Ok(ZipFile {
        content: res.into_inner(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/yaml")]
pub struct YamlContent<'a> {
    pub content: String,
    pub headers: Header<'a>,
}

#[get("/room/<room_id>/download/<yaml_id>")]
#[tracing::instrument(skip(redirect_to, ctx))]
pub async fn download_yaml<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    yaml_id: YamlId,
    ctx: &State<Context>,
) -> Result<YamlContent<'a>> {
    redirect_to.set("/");

    let content: YamlContent<'a> = api::download_yaml(room_id, yaml_id, ctx)
        .await
        .map_err(|api_err| api_err.error)?;
    Ok(content)
}

#[get("/room/<room_id>/download_bundle/<bundle_id>")]
#[tracing::instrument(skip(redirect_to, ctx))]
pub async fn download_bundle<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    bundle_id: BundleId,
    ctx: &State<Context>,
) -> Result<YamlContent<'a>> {
    redirect_to.set("/");

    let content: YamlContent<'a> = api::download_bundle(room_id, bundle_id, ctx)
        .await
        .map_err(|api_err| api_err.error)?;
    Ok(content)
}

#[get("/room/<room_id>/patch/<yaml_id>")]
#[tracing::instrument(skip(redirect_to, gen_output_dir, ctx))]
pub async fn download_patch<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    yaml_id: YamlId,
    gen_output_dir: &State<GenerationOutDir>,
    ctx: &State<Context>,
) -> Result<NamedBuf<'a>> {
    redirect_to.set("/");

    let mut conn = ctx.db_pool.get().await?;

    let Some(generation) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!("No generation found for this room"))?
    };

    let generation_info = get_generation_info(generation.job_id, &gen_output_dir.0)?;
    let Some(generation_file) = generation_info.output_file else {
        Err(anyhow::anyhow!(
            "Generation doesn't have a valid output file"
        ))?
    };

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn).await?;
    let Some(patch_path) = yaml.patch else {
        Err(anyhow::anyhow!(
            "This YAML doesn't have a patch file associated with it"
        ))?
    };

    let archive_path = gen_output_dir
        .0
        .join(generation.job_id.to_string())
        .join(&generation_file);
    let reader = BufReader::new(File::open(archive_path)?);
    let mut archive = ZipArchive::new(reader)?;

    let mut patch_file = archive.by_name(&patch_path)?;

    assert!(patch_file.is_file());
    let mut buf = Vec::new();
    patch_file.read_to_end(&mut buf)?;

    let value = format!("attachment; filename=\"{patch_path}\"");
    Ok(NamedBuf {
        content: buf,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        room,
        upload_yaml,
        delete_bundle,
        delete_yaml,
        download_bundles,
        download_yamls,
        download_bundle,
        download_yaml,
        download_patch,
    ]
}
