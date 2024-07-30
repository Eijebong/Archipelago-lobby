use http::header::CONTENT_DISPOSITION;
use rocket::{
    get,
    http::{Header, Status},
};
use rocket::{routes, State};
use uuid::Uuid;

use super::YamlContent;
use crate::{
    db,
    error::{ApiResult, WithContext, WithStatus},
    Context,
};

#[get("/room/<room_id>/download/<yaml_id>")]
pub(crate) async fn download_yaml<'a>(
    room_id: Uuid,
    yaml_id: Uuid,
    ctx: &State<Context>,
) -> ApiResult<YamlContent<'a>> {
    let _room = db::get_room(room_id, ctx)
        .await
        .context("Couldn't find the room")
        .status(Status::NotFound)?;

    let yaml = db::get_yaml_by_id(yaml_id, ctx)
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
