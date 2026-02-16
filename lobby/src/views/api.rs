use std::collections::HashMap;

use anyhow::anyhow;
use chrono::NaiveDateTime;
use http::header::CONTENT_DISPOSITION;
use rocket::{
    get,
    http::{Header, Status},
    post, routes,
    serde::json::Json,
    State,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::{self, BundleId, RoomId, Yaml, YamlId},
    error::{ApiError, ApiResult, WithContext, WithStatus},
    index_manager::IndexManager,
    jobs::{OptionsGenQueue, YamlValidationQueue},
    session::LoggedInSession,
    views::options_gen::OptionsCache,
    yaml::queue_yaml_validation,
};
use crate::{generation::get_slots, views::YamlContent};
use crate::{jobs::GenerationOutDir, session::AdminSession, Context};

#[derive(Serialize)]
pub struct YamlInfo {
    id: YamlId,
    player_name: String,
    discord_handle: String,
    game: String,
    slot_number: usize,
    has_patch: bool,
    created_at: NaiveDateTime,
}

#[derive(Serialize)]
pub struct SlotPasswordInfo {
    slot_number: usize,
    player_name: String,
    password: Option<String>,
}

#[derive(Deserialize)]
pub struct SetPasswordRequest {
    password: Option<String>,
}

#[derive(Serialize)]
pub struct RoomServerInfo {
    host: String,
    port: i32,
}

#[derive(Serialize)]
pub struct RoomInfo {
    id: RoomId,
    name: String,
    close_date: NaiveDateTime,
    description: String,
    yamls: Vec<YamlInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_info: Option<RoomServerInfo>,
}

#[get("/room/<room_id>")]
#[tracing::instrument(skip(_session, ctx))]
pub(crate) async fn room_info(
    room_id: RoomId,
    _session: LoggedInSession,
    ctx: &State<Context>,
) -> ApiResult<Json<RoomInfo>> {
    let mut conn = ctx.db_pool.get().await?;

    let (room, room_server_info) = db::get_room_with_info(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let yamls = db::get_yamls_for_room_with_author_names(room_id, &mut conn).await?;
    let yamls_vec = yamls.iter().map(|(y, _)| y.clone()).collect::<Vec<_>>();
    let slots = get_slots(&yamls_vec);
    let slot_map: HashMap<YamlId, usize> = slots
        .iter()
        .enumerate()
        .map(|(index, (_, id))| (*id, index))
        .collect();

    Ok(Json(RoomInfo {
        id: room.id,
        name: room.settings.name,
        close_date: room.settings.close_date,
        description: room.settings.description,
        yamls: yamls
            .into_iter()
            .map(|(yaml, discord_handle)| YamlInfo {
                id: yaml.id,
                player_name: yaml.player_name,
                discord_handle,
                game: yaml.game,
                slot_number: *slot_map.get(&yaml.id).unwrap() + 1,
                has_patch: yaml.patch.is_some(),
                created_at: yaml.created_at,
            })
            .collect(),
        server_info: room_server_info.map(|info| RoomServerInfo {
            host: info.host,
            port: info.port,
        }),
    }))
}

#[get("/room/<room_id>/download/<yaml_id>")]
#[tracing::instrument(skip(ctx))]
pub(crate) async fn download_yaml<'a>(
    room_id: RoomId,
    yaml_id: YamlId,
    ctx: &State<Context>,
) -> ApiResult<YamlContent<'a>> {
    let mut conn = ctx.db_pool.get().await?;

    let _room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file")
        .status(Status::NotFound)?;

    let value = format!("attachment; filename=\"{}.yaml\"", yaml.sanitized_name());

    Ok(YamlContent {
        content: yaml.content,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[get("/room/<room_id>/download_bundle/<bundle_id>")]
#[tracing::instrument(skip(ctx))]
pub(crate) async fn download_bundle<'a>(
    room_id: RoomId,
    bundle_id: BundleId,
    ctx: &State<Context>,
) -> ApiResult<YamlContent<'a>> {
    let mut conn = ctx.db_pool.get().await?;

    let _room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let bundle = db::get_bundle_by_id(bundle_id, &mut conn)
        .await
        .context("Couldn't find the YAML bundle")
        .status(Status::NotFound)?;

    let value = format!("attachment; filename=\"{}.yaml\"", bundle.file_name());

    Ok(YamlContent {
        content: bundle.as_yaml(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[get("/room/<room_id>/info/<yaml_id>")]
#[tracing::instrument(skip(ctx))]
pub(crate) async fn yaml_info(
    room_id: RoomId,
    yaml_id: YamlId,
    ctx: &State<Context>,
) -> ApiResult<Json<Yaml>> {
    let mut conn = ctx.db_pool.get().await?;

    let _room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file")
        .status(Status::NotFound)?;

    Ok(Json(yaml))
}

#[get("/room/<room_id>/retry/<yaml_id>")]
#[tracing::instrument(skip(session, index_manager, yaml_validation_queue, ctx))]
pub(crate) async fn retry_yaml(
    room_id: RoomId,
    yaml_id: YamlId,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    ctx: &State<Context>,
) -> ApiResult<()> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file")
        .status(Status::NotFound)?;

    let is_allowed = session.0.is_admin
        || session.user_id() == room.settings.author_id
        || session.user_id() == yaml.owner_id;

    if !is_allowed {
        Err(anyhow!("You're not allowed to retry this validation job"))?
    }

    let mut conn = ctx.db_pool.get().await?;
    queue_yaml_validation(
        &yaml,
        &room,
        index_manager,
        yaml_validation_queue,
        &mut conn,
    )
    .await?;

    Ok(())
}

#[get("/room/<room_id>/refresh_patches")]
#[tracing::instrument(skip(_session, gen_output_dir, ctx))]
pub(crate) async fn refresh_patches(
    _session: AdminSession,
    room_id: RoomId,
    gen_output_dir: &State<GenerationOutDir>,
    ctx: &State<Context>,
) -> ApiResult<()> {
    let mut conn = ctx.db_pool.get().await?;
    crate::jobs::refresh_gen_patches(room_id, &gen_output_dir.0, &mut conn).await?;

    Ok(())
}

#[get("/room/<room_id>/slots_passwords")]
#[tracing::instrument(skip(_session, ctx))]
pub(crate) async fn slots_passwords(
    _session: AdminSession,
    room_id: RoomId,
    ctx: &State<Context>,
) -> ApiResult<Json<Vec<SlotPasswordInfo>>> {
    let mut conn = ctx.db_pool.get().await?;

    let yamls = db::get_yamls_for_room_with_author_names(room_id, &mut conn).await?;
    let yamls_vec = yamls.iter().map(|(y, _)| y.clone()).collect::<Vec<_>>();
    let slots = get_slots(&yamls_vec);

    let slot_map: HashMap<YamlId, usize> = slots
        .iter()
        .enumerate()
        .map(|(index, (_, id))| (*id, index + 1))
        .collect();

    let mut result: Vec<SlotPasswordInfo> = yamls
        .into_iter()
        .map(|(yaml, _)| SlotPasswordInfo {
            slot_number: *slot_map.get(&yaml.id).unwrap(),
            player_name: yaml.player_name,
            password: yaml.password,
        })
        .collect();

    result.sort_by_key(|info| info.slot_number);

    Ok(Json(result))
}

#[post("/room/<room_id>/set_password/<yaml_id>", data = "<request>")]
#[tracing::instrument(skip(_session, request, ctx))]
pub(crate) async fn set_password(
    _session: AdminSession,
    room_id: RoomId,
    yaml_id: YamlId,
    request: Json<SetPasswordRequest>,
    ctx: &State<Context>,
) -> ApiResult<()> {
    let mut conn = ctx.db_pool.get().await?;

    let _room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let _yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file")
        .status(Status::NotFound)?;

    db::update_yaml_password(yaml_id, request.password.clone(), &mut conn).await?;

    Ok(())
}

#[derive(Serialize)]
pub struct GameInfo {
    apworld_name: String,
    game_name: String,
}

#[get("/games")]
#[tracing::instrument(skip(_session, index_manager))]
pub(crate) async fn list_games(
    _session: AdminSession,
    index_manager: &State<IndexManager>,
) -> Json<Vec<GameInfo>> {
    let index = index_manager.index.read().await;
    let mut games: Vec<GameInfo> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| GameInfo {
            apworld_name: apworld_name.clone(),
            game_name: world.name.clone(),
        })
        .collect();
    games.sort_by(|a, b| a.game_name.to_lowercase().cmp(&b.game_name.to_lowercase()));
    Json(games)
}

#[derive(Serialize)]
pub struct OptionInfo {
    name: String,
    ty: String,
    choices: Option<Vec<String>>,
    suggestions: Option<Vec<String>>,
    valid_keys: Option<Vec<String>>,
}

#[get("/games/<apworld>/options")]
#[tracing::instrument(skip(_session, index_manager, options_gen_queue, options_cache))]
pub(crate) async fn game_options(
    _session: AdminSession,
    apworld: &str,
    index_manager: &State<IndexManager>,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
) -> ApiResult<Json<Vec<OptionInfo>>> {
    let version = {
        let index = index_manager.index.read().await;
        let world = index.worlds.get(apworld).ok_or_else(|| {
            ApiError {
                error: anyhow!("Unknown apworld"),
                status: Status::NotFound,
            }
        })?;
        world.versions.keys().max().cloned().ok_or_else(|| {
            ApiError {
                error: anyhow!("No versions available"),
                status: Status::NotFound,
            }
        })?
    };

    let options = super::options_gen::get_options_def(
        apworld,
        &version,
        options_gen_queue,
        options_cache,
    )
    .await
    .status(Status::InternalServerError)?;

    let result: Vec<OptionInfo> = options
        .iter()
        .flat_map(|(_, group)| group.iter())
        .map(|(name, def)| OptionInfo {
            name: name.clone(),
            ty: def.ty.clone(),
            choices: def.choices.clone(),
            suggestions: def.suggestions.clone(),
            valid_keys: def.valid_keys.clone(),
        })
        .collect();

    Ok(Json(result))
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        download_bundle,
        download_yaml,
        retry_yaml,
        yaml_info,
        room_info,
        refresh_patches,
        slots_passwords,
        set_password,
        list_games,
        game_options
    ]
}
