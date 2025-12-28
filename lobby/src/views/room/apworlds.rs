use apwm::World;
use askama::Template;
use askama_web::WebTemplate;
use itertools::Itertools;
use rocket::State;
use semver::Version;

use crate::db::{self, Room, RoomId};
use crate::error::{RedirectTo, Result};
use crate::index_manager::IndexManager;
use crate::session::LoggedInSession;
use crate::session::Session;
use crate::utils::ZipFile;
use crate::views::filters;
use crate::{Context, TplContext};

#[derive(Template, WebTemplate)]
#[template(path = "room/apworlds.html")]
pub struct RoomApworldsTpl<'a> {
    base: TplContext<'a>,
    is_my_room: bool,
    apworlds: Vec<(String, (World, Version))>,
    room: Room,
}

#[rocket::get("/room/<room_id>/worlds")]
#[tracing::instrument(skip(redirect_to, ctx, index_manager, session))]
pub async fn room_worlds<'a>(
    room_id: RoomId,
    session: Session,
    index_manager: &State<IndexManager>,
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
) -> Result<RoomApworldsTpl<'a>> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.is_admin || session.user_id == Some(room.settings.author_id);

    let index = index_manager.index.read().await.clone();
    let (apworlds, resolve_errors) = room.settings.manifest.resolve_with(&index);
    if !resolve_errors.is_empty() {
        Err(anyhow::anyhow!(
            "Error while resolving apworlds for this room: {}",
            resolve_errors.iter().join("\n")
        ))?
    }

    let mut apworlds = Vec::from_iter(apworlds);
    apworlds.sort_by_key(|(_, (world, _))| world.display_name.to_lowercase());

    Ok(RoomApworldsTpl {
        base: TplContext::from_session("room", session, ctx).await,
        is_my_room,
        apworlds,
        room,
    })
}

#[rocket::get("/room/<room_id>/worlds/download_all")]
#[tracing::instrument(skip(ctx, _session, index_manager, redirect_to))]
pub async fn room_download_all_worlds<'a>(
    room_id: RoomId,
    _session: LoggedInSession,
    index_manager: &'a State<IndexManager>,
    redirect_to: &'a RedirectTo,
    ctx: &'a State<Context>,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;

    Ok(index_manager
        .download_apworlds(&room.settings.manifest)
        .await?)
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![room_worlds, room_download_all_worlds]
}
