use std::collections::HashMap;

use anyhow::anyhow;
use chrono::NaiveDateTime;
use http::header::CONTENT_DISPOSITION;
use rocket::{
    get,
    http::{Header, Status},
    routes,
    serde::json::Json,
    State,
};
use serde::Serialize;

use crate::Context;
use crate::{
    db::{self, RoomId, Yaml, YamlId},
    error::{ApiResult, WithContext, WithStatus},
    index_manager::IndexManager,
    jobs::YamlValidationQueue,
    session::LoggedInSession,
    yaml::queue_yaml_validation,
};
use crate::{generation::get_slots, views::YamlContent};

#[derive(Serialize)]
pub struct YamlInfo {
    id: YamlId,
    player_name: String,
    discord_handle: String,
    game: String,
    slot_number: usize,
    has_patch: bool,
}

#[derive(Serialize)]
pub struct RoomInfo {
    id: RoomId,
    name: String,
    close_date: NaiveDateTime,
    description: String,
    yamls: Vec<YamlInfo>,
}

#[get("/room/<room_id>")]
pub(crate) async fn room_info(
    room_id: RoomId,
    _session: LoggedInSession,
    ctx: &State<Context>,
) -> ApiResult<Json<RoomInfo>> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn).await?;

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
            })
            .collect(),
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

#[get("/room/<room_id>/info/<yaml_id>")]
#[tracing::instrument(skip(ctx))]
pub(crate) async fn yaml_info<'a>(
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
pub(crate) async fn retry_yaml<'a>(
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

pub fn routes() -> Vec<rocket::Route> {
    routes![download_yaml, retry_yaml, yaml_info, room_info]
}
