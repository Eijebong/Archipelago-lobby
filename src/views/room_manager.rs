#![allow(clippy::blocks_in_conditions)]

use ap_lobby::db::{self, Author, NewRoom, Room, RoomFilter, RoomId, RoomTemplateId};
use ap_lobby::error::{RedirectTo, Result, WithContext};
use ap_lobby::index_manager::IndexManager;
use ap_lobby::session::LoggedInSession;
use askama::Template;
use chrono::{DateTime, TimeZone, Utc};
use rocket::form::Form;
use rocket::http::uri::Absolute;
use rocket::http::{self, CookieJar};
use rocket::response::Redirect;
use rocket::{get, post, FromForm};
use std::str::FromStr;

use crate::{Context, TplContext};
use rocket::State;

use super::manifest_editor::{manifest_from_form, ManifestForm};
use super::room_settings::RoomSettingsBuilder;
use super::room_settings::RoomSettingsType;

#[derive(Template)]
#[template(path = "room_manager/edit_room.html")]
struct EditRoom<'a> {
    base: TplContext<'a>,
    room: Option<Room>,
    room_settings_form: RoomSettingsBuilder<'a>,
}

#[derive(FromForm, Debug)]
pub struct CreateRoomForm<'a> {
    room: RoomSettingsForm<'a>,
}

#[derive(FromForm, Debug)]
pub struct RoomSettingsForm<'a> {
    pub room_name: &'a str,
    pub room_description: &'a str,
    pub close_date: &'a str,
    pub tz_offset: i32,
    pub room_url: &'a str,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
    pub yaml_limit_per_user: bool,
    pub yaml_limit_per_user_nb: i32,
    pub yaml_limit_bypass_list: &'a str,
    pub show_apworlds: bool,
    pub me: ManifestForm<'a>,
}

#[derive(Template)]
#[template(path = "room_manager/rooms.html")]
struct ListRoomsTpl<'a> {
    base: TplContext<'a>,
    rooms: Vec<Room>,
    current_page: u64,
    max_pages: u64,
}

#[get("/rooms?<page>")]
#[tracing::instrument(skip_all)]
async fn my_rooms<'a>(
    ctx: &State<Context>,
    session: LoggedInSession,
    page: Option<u64>,
    cookies: &CookieJar<'a>,
) -> Result<ListRoomsTpl<'a>> {
    let author_filter = if session.0.is_admin {
        Author::Any
    } else {
        Author::User(session.user_id())
    };

    let mut conn = ctx.db_pool.get().await?;
    let current_page = page.unwrap_or(1);

    let (rooms, max_pages) = db::list_rooms(
        RoomFilter::default().with_author(author_filter),
        current_page,
        &mut conn,
    )
    .await?;

    Ok(ListRoomsTpl {
        base: TplContext::from_session("rooms", session.0, cookies),
        rooms,
        current_page,
        max_pages,
    })
}

#[get("/create-room?<from_template>")]
#[tracing::instrument(skip_all)]
async fn create_room<'a>(
    from_template: Option<RoomTemplateId>,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    ctx: &State<Context>,
    cookies: &CookieJar<'_>,
) -> Result<EditRoom<'a>> {
    let current_user_id = session.user_id();
    let base = TplContext::from_session("create-room", session.0, cookies);
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

pub fn parse_date(date: &str, tz_offset: i32) -> Result<DateTime<Utc>> {
    let offset = chrono::FixedOffset::west_opt(tz_offset * 60)
        .ok_or_else(|| ap_lobby::error::Error(anyhow::anyhow!("Wrong timezone offset")))?;
    let datetime = chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M")?;
    let date = offset
        .from_local_datetime(&datetime)
        .single()
        .ok_or_else(|| ap_lobby::error::Error(anyhow::anyhow!("Cannot parse passed datetime")))?;

    Ok(date.into())
}

#[post("/create-room?<from_template>", data = "<room_form>")]
#[tracing::instrument(skip_all)]
async fn create_room_submit<'a>(
    from_template: Option<RoomTemplateId>,
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    mut room_form: Form<CreateRoomForm<'a>>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set("/create-room");

    validate_room_form(&mut room_form.room)?;
    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.room.me, &index)
    }?;

    let author_id = session.user_id();
    let close_date = parse_date(room_form.room.close_date, room_form.room.tz_offset)?;
    let new_room = NewRoom {
        id: RoomId::new_v4(),
        name: room_form.room.room_name.trim(),
        close_date: close_date.naive_utc(),
        description: room_form.room.room_description.trim(),
        room_url: "",
        author_id: Some(author_id),
        yaml_validation: room_form.room.yaml_validation,
        allow_unsupported: room_form.room.allow_unsupported,
        yaml_limit_per_user: room_form
            .room
            .yaml_limit_per_user
            .then_some(room_form.room.yaml_limit_per_user_nb),
        yaml_limit_bypass_list: room_form
            .room
            .yaml_limit_bypass_list
            .split(',')
            .filter_map(|id| i64::from_str(id).ok())
            .collect(),
        manifest: db::Json(room_manifest),
        show_apworlds: room_form.room.show_apworlds,
        from_template_id: Some(from_template),
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
#[tracing::instrument(skip(ctx, session, cookies, index_manager))]
async fn edit_room<'a>(
    ctx: &State<Context>,
    room_id: RoomId,
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    cookies: &CookieJar<'a>,
) -> Result<EditRoom<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);

    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to edit this room").into());
    }

    let index = index_manager.index.read().await;
    let base = TplContext::from_session("room", session.0, cookies);

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
async fn delete_room<'a>(
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
#[tracing::instrument(skip(redirect_to, room_form, index_manager, ctx, session))]
async fn edit_room_submit<'a>(
    redirect_to: &RedirectTo,
    room_id: RoomId,
    mut room_form: Form<CreateRoomForm<'a>>,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set(&format!("/edit-room/{}", room_id));

    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);
    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to edit this room").into());
    }

    validate_room_form(&mut room_form.room)?;

    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.room.me, &index)
    }?;

    let new_room = NewRoom {
        id: room_id,
        name: room_form.room.room_name.trim(),
        description: room_form.room.room_description.trim(),
        close_date: parse_date(room_form.room.close_date, room_form.room.tz_offset)?.naive_utc(),
        room_url: room_form.room.room_url,
        author_id: None, // (Skips updating that field)
        yaml_validation: room_form.room.yaml_validation,
        allow_unsupported: room_form.room.allow_unsupported,
        yaml_limit_per_user: room_form
            .room
            .yaml_limit_per_user
            .then_some(room_form.room.yaml_limit_per_user_nb),
        yaml_limit_bypass_list: room_form
            .room
            .yaml_limit_bypass_list
            .split(',')
            .filter_map(|id| i64::from_str(id).ok())
            .collect(),
        manifest: db::Json(room_manifest),
        show_apworlds: room_form.room.show_apworlds,
        from_template_id: None,
    };

    db::update_room(&new_room, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

pub fn validate_room_form(room_form: &mut RoomSettingsForm<'_>) -> Result<()> {
    if room_form.room_name.trim().is_empty() {
        return Err(anyhow::anyhow!("The room name shouldn't be empty").into());
    }

    if room_form.room_name.len() > 200 {
        return Err(anyhow::anyhow!("The room name shouldn't exceed 200 characters. Seriously it doesn't need to be that long.").into());
    }

    let room_url = room_form.room_url.trim();
    if !room_url.is_empty() {
        if let Err(e) = http::uri::Uri::parse::<Absolute>(room_url) {
            return Err(anyhow::anyhow!("Error while parsing room URL: {}", e).into());
        }
    }
    room_form.room_url = room_url;

    if room_form.yaml_limit_per_user && room_form.yaml_limit_per_user_nb <= 0 {
        return Err(
            anyhow::anyhow!("The per player YAML limit should be greater or equal to 1").into(),
        );
    }

    if !room_form.yaml_limit_bypass_list.is_empty() {
        let possible_ids = room_form.yaml_limit_bypass_list.split(',');
        for possible_id in possible_ids {
            if i64::from_str(possible_id).is_err() {
                return Err(anyhow::anyhow!(
                    "The YAML limit bypass list should be a comma delimited list of discord IDs."
                )
                .into());
            }
        }
    }

    Ok(())
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        my_rooms,
        create_room,
        edit_room,
        delete_room,
        create_room_submit,
        edit_room_submit
    ]
}
