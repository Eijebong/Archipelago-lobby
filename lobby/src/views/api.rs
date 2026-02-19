use std::collections::HashMap;

use anyhow::anyhow;
use chrono::{NaiveDateTime, Utc};
use http::header::CONTENT_DISPOSITION;
use rocket::{
    delete, get,
    http::{Header, Status},
    post, put, routes,
    serde::json::Json,
    State,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::{self, BundleId, Json as DbJson, RoomId, Yaml, YamlId},
    error::{ApiError, ApiResult, WithContext, WithStatus},
    index_manager::IndexManager,
    jobs::{OptionsGenQueue, YamlValidationQueue},
    session::LoggedInSession,
    views::options_gen::OptionsCache,
    yaml::{queue_yaml_validation, YamlValidationJobResult},
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
        content: yaml.current_content().to_string(),
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
        let world = index.worlds.get(apworld).ok_or_else(|| ApiError {
            error: anyhow!("Unknown apworld"),
            status: Status::NotFound,
        })?;
        world
            .versions
            .keys()
            .max()
            .cloned()
            .ok_or_else(|| ApiError {
                error: anyhow!("No versions available"),
                status: Status::NotFound,
            })?
    };

    let options =
        super::options_gen::get_options_def(apworld, &version, options_gen_queue, options_cache)
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

#[derive(Serialize)]
pub(crate) struct BulkYamlInfo {
    id: YamlId,
    player_name: String,
    discord_handle: String,
    game: String,
    content: String,
    created_at: NaiveDateTime,
    last_edited_by_name: Option<String>,
    last_edited_at: Option<NaiveDateTime>,
}

#[get("/room/<room_id>/yamls")]
#[tracing::instrument(skip(_session, ctx))]
pub(crate) async fn bulk_yamls(
    room_id: RoomId,
    _session: AdminSession,
    ctx: &State<Context>,
) -> ApiResult<Json<Vec<BulkYamlInfo>>> {
    use crate::schema::{discord_users, yamls};
    use diesel::dsl::sql;
    use diesel::prelude::*;
    use diesel::sql_types::Text;
    use diesel_async::RunQueryDsl;

    let mut conn = ctx.db_pool.get().await?;

    let _room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let rows: Vec<(
        YamlId,
        String,
        String,
        String,
        String,
        NaiveDateTime,
        Option<String>,
        Option<NaiveDateTime>,
    )> = yamls::table
        .filter(yamls::room_id.eq(&room_id))
        .inner_join(discord_users::table)
        .select((
            yamls::id,
            yamls::player_name,
            discord_users::username,
            yamls::game,
            sql::<Text>("COALESCE(yamls.edited_content, yamls.content)"),
            yamls::created_at,
            yamls::last_edited_by_name,
            yamls::last_edited_at,
        ))
        .get_results(&mut conn)
        .await?;

    let result = rows
        .into_iter()
        .map(
            |(
                id,
                player_name,
                discord_handle,
                game,
                content,
                created_at,
                last_edited_by_name,
                last_edited_at,
            )| {
                BulkYamlInfo {
                    id,
                    player_name,
                    discord_handle,
                    game,
                    content,
                    created_at,
                    last_edited_by_name,
                    last_edited_at,
                }
            },
        )
        .collect();

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct EditYamlRequest {
    content: String,
    edited_by: i64,
    edited_by_name: String,
}

#[derive(Serialize)]
pub struct EditYamlResponse {
    ok: bool,
    game: String,
    player_name: String,
}

#[put("/room/<room_id>/yaml/<yaml_id>/edit", data = "<request>")]
#[tracing::instrument(skip(_session, request, index_manager, yaml_validation_queue, ctx))]
pub(crate) async fn edit_yaml(
    _session: AdminSession,
    room_id: RoomId,
    yaml_id: YamlId,
    request: Json<EditYamlRequest>,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    ctx: &State<Context>,
) -> ApiResult<Json<EditYamlResponse>> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    if !room.is_closed() {
        return Err(ApiError {
            error: anyhow!("Edits are only allowed on closed rooms"),
            status: Status::BadRequest,
        });
    }

    let _yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file")
        .status(Status::NotFound)?;

    let req = request.into_inner();
    let documents = crate::yaml::parse_raw_yamls(&[&req.content]).map_err(|e| ApiError {
        error: e.0,
        status: Status::BadRequest,
    })?;

    if documents.len() != 1 {
        return Err(ApiError {
            error: anyhow!("Edited content must contain exactly one YAML document"),
            status: Status::BadRequest,
        });
    }

    let (document, parsed) = &documents[0];
    let game_name = crate::yaml::validate_game(&parsed.game).map_err(|e| ApiError {
        error: e.0,
        status: Status::BadRequest,
    })?;

    if room.settings.yaml_validation {
        let validation_result = crate::yaml::validate_yaml(
            document,
            parsed,
            &room.settings.manifest,
            index_manager,
            yaml_validation_queue,
        )
        .await
        .map_err(|e| ApiError {
            error: e.0,
            status: Status::InternalServerError,
        })?;

        let (apworlds, validation_status, last_error) = match validation_result {
            YamlValidationJobResult::Success(apworlds) => {
                (apworlds, db::YamlValidationStatus::Validated, None)
            }
            YamlValidationJobResult::Failure(apworlds, error) => {
                if room.settings.allow_invalid_yamls {
                    (apworlds, db::YamlValidationStatus::Failed, Some(error))
                } else {
                    return Err(ApiError {
                        error: anyhow!("Validation failed: {}", error),
                        status: Status::BadRequest,
                    });
                }
            }
            YamlValidationJobResult::Unsupported(games) => {
                let error = format!("Unsupported apworlds: {}", games.join(", "));
                (vec![], db::YamlValidationStatus::Unsupported, Some(error))
            }
        };

        let index = index_manager.index.read().await;
        let features = crate::extractor::extract_features(&index, parsed, document)?;

        db::update_yaml_edited_content(
            yaml_id,
            &req.content,
            &game_name,
            &parsed.name,
            DbJson(features),
            validation_status,
            apworlds,
            last_error,
            req.edited_by,
            &req.edited_by_name,
            Utc::now().naive_utc(),
            &mut conn,
        )
        .await?;
    } else {
        let index = index_manager.index.read().await;
        let features = crate::extractor::extract_features(&index, parsed, document)?;

        db::update_yaml_edited_content(
            yaml_id,
            &req.content,
            &game_name,
            &parsed.name,
            DbJson(features),
            db::YamlValidationStatus::Unknown,
            vec![],
            None,
            req.edited_by,
            &req.edited_by_name,
            Utc::now().naive_utc(),
            &mut conn,
        )
        .await?;
    }

    Ok(Json(EditYamlResponse {
        ok: true,
        game: game_name,
        player_name: parsed.name.clone(),
    }))
}

#[delete("/room/<room_id>/yaml/<yaml_id>")]
#[tracing::instrument(skip(_session, ctx))]
pub(crate) async fn delete_yaml_api(
    _session: AdminSession,
    room_id: RoomId,
    yaml_id: YamlId,
    ctx: &State<Context>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    if !room.is_closed() {
        return Err(ApiError {
            error: anyhow!("Deleting YAMLs is only allowed on closed rooms"),
            status: Status::BadRequest,
        });
    }

    let _yaml = db::get_yaml_by_id(yaml_id, &mut conn)
        .await
        .context("Couldn't find the YAML file in this room")
        .status(Status::NotFound)?;

    db::remove_yaml(yaml_id, &mut conn).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        download_bundle,
        download_yaml,
        retry_yaml,
        yaml_info,
        room_info,
        bulk_yamls,
        refresh_patches,
        slots_passwords,
        set_password,
        list_games,
        game_options,
        edit_yaml,
        delete_yaml_api
    ]
}
