use http::header::CONTENT_DISPOSITION;
use rocket::{
    get,
    http::{Header, Status},
    routes, State,
};

use crate::views::YamlContent;
use crate::Context;
use ap_lobby::{
    db::{self, RoomId, YamlId},
    error::{ApiResult, WithContext, WithStatus},
};

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

pub fn routes() -> Vec<rocket::Route> {
    routes![download_yaml,]
}
