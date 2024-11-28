use anyhow::anyhow;
use ap_lobby::db::Room;
use ap_lobby::error::Result;
use ap_lobby::{
    db::{self, NewRoomTemplate, RoomTemplate, RoomTemplateId},
    error::RedirectTo,
    index_manager::IndexManager,
    session::LoggedInSession,
};
use askama::Template;
use rocket::{form::Form, get, http::CookieJar, post, response::Redirect, State};
use std::str::FromStr;

use crate::{Context, TplContext};

use super::{
    manifest_editor::manifest_from_form,
    room_manager::{parse_date, validate_room_form, CreateRoomForm},
    room_settings::{RoomSettingsBuilder, RoomSettingsType},
};

#[derive(Template)]
#[template(path = "room_manager/room_templates.html")]
pub struct ListRoomTemplatesTpl<'a> {
    base: TplContext<'a>,
    room_templates: Vec<RoomTemplate>,
}

#[derive(Template)]
#[template(path = "room_manager/edit_room_template.html")]
pub struct EditRoomTemplateTpl<'a> {
    base: TplContext<'a>,
    tpl: Option<RoomTemplate>,
    tpl_settings_form: RoomSettingsBuilder<'a>,
}

#[derive(Template)]
#[template(path = "room_manager/room_templates_associated.html")]
pub struct AssociatedRoomsTpl<'a> {
    base: TplContext<'a>,
    tpl: RoomTemplate,
    rooms: Vec<Room>,
}

#[get("/room-templates")]
#[tracing::instrument(skip_all)]
async fn list_templates<'a>(
    ctx: &State<Context>,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<ListRoomTemplatesTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let room_templates = db::get_room_templates_for_author(session.user_id(), &mut conn).await?;

    Ok(ListRoomTemplatesTpl {
        base: TplContext::from_session("room-templates", session.0, cookies),
        room_templates,
    })
}

#[get("/room-templates/create")]
#[tracing::instrument(skip_all)]
async fn create_template<'a>(
    index_manager: &State<IndexManager>,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<EditRoomTemplateTpl<'a>> {
    let index = index_manager.index.read().await;

    let base = TplContext::from_session("room-templates", session.0, cookies);
    Ok(EditRoomTemplateTpl {
        tpl: None,
        tpl_settings_form: RoomSettingsBuilder::new(
            base.clone(),
            &index,
            RoomSettingsType::Template,
        )?,
        base,
    })
}

#[post("/room-templates/create", data = "<room_form>")]
#[tracing::instrument(skip_all)]
async fn create_tpl_submit<'a>(
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    mut room_form: Form<CreateRoomForm<'a>>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set("/room-templates/create");

    validate_room_form(&mut room_form)?;
    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.me, &index)
    }?;

    let author_id = session.user_id();
    let close_date = parse_date(room_form.close_date, room_form.tz_offset)?;
    let new_tpl = NewRoomTemplate {
        id: RoomTemplateId::new_v4(),
        name: room_form.room_name.trim(),
        close_date: close_date.naive_utc(),
        description: room_form.room_description.trim(),
        room_url: "",
        author_id: Some(author_id),
        yaml_validation: room_form.yaml_validation,
        allow_unsupported: room_form.allow_unsupported,
        yaml_limit_per_user: room_form
            .yaml_limit_per_user
            .then_some(room_form.yaml_limit_per_user_nb),
        yaml_limit_bypass_list: room_form
            .yaml_limit_bypass_list
            .split(',')
            .filter_map(|id| i64::from_str(id).ok())
            .collect(),
        manifest: db::Json(room_manifest),
        show_apworlds: room_form.show_apworlds,
    };

    let mut conn = ctx.db_pool.get().await?;
    db::create_room_template(&new_tpl, &mut conn).await?;

    Ok(Redirect::to("/room-templates"))
}

#[get("/room-templates/<tpl_id>")]
#[tracing::instrument(skip_all)]
async fn edit_template<'a>(
    tpl_id: RoomTemplateId,
    index_manager: &State<IndexManager>,
    ctx: &State<Context>,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<EditRoomTemplateTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let template = db::get_room_template_by_id(tpl_id, &mut conn).await?;
    let is_my_template = template.settings.author_id == session.user_id();
    if !is_my_template && !template.global {
        Err(anyhow!("You are not allowed to edit this template"))?;
    }

    let index = index_manager.index.read().await;

    let base = TplContext::from_session("template", session.0, cookies);
    Ok(EditRoomTemplateTpl {
        tpl: Some(template.clone()),
        tpl_settings_form: RoomSettingsBuilder::new_with_template(
            base.clone(),
            index.clone(),
            template,
        )
        .read_only(!is_my_template),
        base,
    })
}

#[post("/room-templates/<tpl_id>", data = "<room_form>")]
#[tracing::instrument(skip(redirect_to, room_form, index_manager, ctx, session))]
async fn edit_tpl_submit<'a>(
    redirect_to: &RedirectTo,
    tpl_id: RoomTemplateId,
    mut room_form: Form<CreateRoomForm<'a>>,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room-templates/{}", tpl_id));

    let mut conn = ctx.db_pool.get().await?;
    let tpl = db::get_room_template_by_id(tpl_id, &mut conn).await?;
    let is_my_tpl = session.0.is_admin || session.0.user_id == Some(tpl.settings.author_id);
    if !is_my_tpl {
        return Err(anyhow::anyhow!("You're not allowed to edit this room template").into());
    }

    validate_room_form(&mut room_form)?;

    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.me, &index)
    }?;

    let new_tpl = NewRoomTemplate {
        id: tpl_id,
        name: room_form.room_name.trim(),
        description: room_form.room_description.trim(),
        close_date: parse_date(room_form.close_date, room_form.tz_offset)?.naive_utc(),
        room_url: room_form.room_url,
        author_id: None, // (Skips updating that field)
        yaml_validation: room_form.yaml_validation,
        allow_unsupported: room_form.allow_unsupported,
        yaml_limit_per_user: room_form
            .yaml_limit_per_user
            .then_some(room_form.yaml_limit_per_user_nb),
        yaml_limit_bypass_list: room_form
            .yaml_limit_bypass_list
            .split(',')
            .filter_map(|id| i64::from_str(id).ok())
            .collect(),
        manifest: db::Json(room_manifest),
        show_apworlds: room_form.show_apworlds,
    };

    db::update_room_template(&new_tpl, &mut conn).await?;

    Ok(Redirect::to("/room-templates"))
}

#[get("/room-templates/<tpl_id>/delete")]
#[tracing::instrument(skip(ctx, session))]
async fn delete_template<'a>(
    ctx: &State<Context>,
    tpl_id: RoomTemplateId,
    session: LoggedInSession,
) -> Result<Redirect> {
    let mut conn = ctx.db_pool.get().await?;
    let tpl = db::get_room_template_by_id(tpl_id, &mut conn).await?;
    let is_my_tpl = session.0.is_admin || session.0.user_id == Some(tpl.settings.author_id);

    if !is_my_tpl {
        return Err(anyhow::anyhow!("You're not allowed to delete this room template").into());
    }

    db::delete_room_template(tpl_id, &mut conn).await?;

    Ok(Redirect::to("/room-templates"))
}

#[get("/room-templates/<tpl_id>/rooms")]
#[tracing::instrument(skip(ctx, session))]
pub async fn list_associated_rooms<'a>(
    ctx: &State<Context>,
    tpl_id: RoomTemplateId,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<AssociatedRoomsTpl<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let tpl = db::get_room_template_by_id(tpl_id, &mut conn).await?;
    let is_my_tpl =
        tpl.global || session.0.is_admin || session.0.user_id == Some(tpl.settings.author_id);

    if !is_my_tpl {
        return Err(anyhow::anyhow!("Couldn't find the given template").into());
    }

    let rooms = db::list_rooms_from_template(tpl_id, session.user_id(), &mut conn).await?;
    Ok(AssociatedRoomsTpl {
        base: TplContext::from_session("template", session.0, cookies),
        tpl,
        rooms,
    })
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        list_templates,
        create_template,
        edit_template,
        delete_template,
        create_tpl_submit,
        edit_tpl_submit,
        list_associated_rooms,
    ]
}
