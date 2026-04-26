use crate::db::{self, Room, RoomId, RoomTemplateId};
use crate::error::{RedirectTo, Result, WithContext};
use crate::index_manager::IndexManager;
use crate::jobs::YamlValidationQueue;
use crate::session::LoggedInSession;
use crate::yaml::revalidate_yamls_if_necessary;
use askama::Template;
use askama_web::WebTemplate;
use rocket::form::Form;
use rocket::response::Redirect;
use rocket::State;
use rocket::{get, post};

use crate::{Context, TplContext};

use crate::views::room_settings::{
    validate_room_form, CreateRoomForm, RoomSettingsBuilder, RoomSettingsType,
};

#[derive(Template, WebTemplate)]
#[template(path = "room/edit.html")]
pub struct EditRoom<'a> {
    base: TplContext<'a>,
    room: Option<Room>,
    room_settings_form: RoomSettingsBuilder<'a>,
}

#[get("/create-room?<from_template>")]
#[tracing::instrument(skip_all)]
pub async fn create_room<'a>(
    from_template: Option<RoomTemplateId>,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    ctx: &State<Context>,
) -> Result<EditRoom<'a>> {
    let current_user_id = session.user_id();
    let base = TplContext::from_session("create-room", session.0, ctx).await;
    let index = index_manager.index.read().await;

    let form_builder = if let Some(template_id) = from_template {
        let mut conn = ctx.db_pool.get().await?;
        let template = db::get_room_template_by_id(template_id, &mut conn)
            .await
            .context("Couldn't get the specified template")?;
        if !template.global && template.settings.author_id != current_user_id {
            RoomSettingsBuilder::new(base.clone(), &index, RoomSettingsType::Room)?
        } else {
            RoomSettingsBuilder::room_from_template(base.clone(), index.clone(), template)?
        }
    } else {
        RoomSettingsBuilder::new(base.clone(), &index, RoomSettingsType::Room)?
    };

    Ok(EditRoom {
        room: None,
        room_settings_form: form_builder,
        base,
    })
}

#[post("/create-room?<from_template>", data = "<room_form>")]
#[tracing::instrument(skip_all)]
pub async fn create_room_submit<'a>(
    from_template: Option<RoomTemplateId>,
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    mut room_form: Form<CreateRoomForm<'a>>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set("/create-room");

    validate_room_form(&mut room_form.room)?;
    let new_room = {
        let index = index_manager.index.read().await;
        room_form.room.to_new_room(
            RoomId::new_v4(),
            &index,
            Some(session.user_id()),
            Some(from_template),
        )?
    };

    let mut conn = ctx.db_pool.get().await?;
    if let Some(template_id) = from_template {
        let tpl = db::get_room_template_by_id(template_id, &mut conn)
            .await
            .context("The given template couldn't be found")?;
        if !tpl.global && tpl.settings.author_id != session.user_id() {
            Err(anyhow::anyhow!("The given template couldn't be found"))?
        }
    }

    let new_room = db::create_room(&new_room, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{}", new_room.id)))
}

#[get("/edit-room/<room_id>")]
#[tracing::instrument(skip(ctx, session, index_manager))]
pub async fn edit_room<'a>(
    ctx: &State<Context>,
    room_id: RoomId,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
) -> Result<EditRoom<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);

    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to edit this room").into());
    }

    let index = index_manager.index.read().await;
    let base = TplContext::from_session("room", session.0, ctx).await;

    Ok(EditRoom {
        room_settings_form: RoomSettingsBuilder::new_with_room(
            base.clone(),
            index.clone(),
            room.clone(),
        ),
        room: Some(room),
        base,
    })
}

#[get("/edit-room/<room_id>/delete")]
#[tracing::instrument(skip(ctx, session))]
pub async fn delete_room(
    ctx: &State<Context>,
    room_id: RoomId,
    session: LoggedInSession,
) -> Result<Redirect> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);

    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to delete this room").into());
    }

    db::delete_room(room_id, &mut conn).await?;

    Ok(Redirect::to("/"))
}

#[post("/edit-room/<room_id>", data = "<room_form>")]
#[tracing::instrument(skip(
    redirect_to,
    room_form,
    index_manager,
    ctx,
    session,
    yaml_validation_queue
))]
pub async fn edit_room_submit<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    mut room_form: Form<CreateRoomForm<'a>>,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set(&format!("/edit-room/{room_id}"));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);
    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to edit this room").into());
    }

    validate_room_form(&mut room_form.room)?;

    let (old_resolved, new_room) = {
        let index = index_manager.index.read().await;
        let old_resolved = room.settings.manifest.resolve_with(&index).0;
        // author_id and from_template_id are None to skip updating those fields.
        let new_room = room_form.room.to_new_room(room_id, &index, None, None)?;
        (old_resolved, new_room)
    };

    let room = db::update_room(&new_room, &mut conn).await?;
    revalidate_yamls_if_necessary(
        &room,
        &old_resolved,
        index_manager,
        yaml_validation_queue,
        &mut conn,
    )
    .await?;

    Ok(Redirect::to(format!("/room/{room_id}")))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        //create_room,
        //create_room_submit,
        edit_room,
        delete_room,
        edit_room_submit
    ]
}
