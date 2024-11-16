#![allow(clippy::blocks_in_conditions)]

use ap_lobby::db::{self, Author, NewRoom, Room, RoomFilter};
use ap_lobby::error::{RedirectTo, Result};
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
use uuid::Uuid;

use crate::{Context, TplContext};
use rocket::State;

use super::manifest_editor::{manifest_from_form, ManifestForm};
use super::room_settings::RoomSettingsBuilder;

#[derive(Template)]
#[template(path = "room_manager/edit_room.html")]
struct EditRoom<'a> {
    base: TplContext<'a>,
    room: Option<Room>,
    room_settings_form: RoomSettingsBuilder<'a>,
}

#[derive(FromForm, Debug)]
struct CreateRoomForm<'a> {
    room_name: &'a str,
    room_description: &'a str,
    close_date: &'a str,
    tz_offset: i32,
    room_url: &'a str,
    private: bool,
    yaml_validation: bool,
    allow_unsupported: bool,
    yaml_limit_per_user: bool,
    yaml_limit_per_user_nb: i32,
    yaml_limit_bypass_list: &'a str,
    show_apworlds: bool,
    me: ManifestForm<'a>,
}

#[derive(Template)]
#[template(path = "room_manager/rooms.html")]
struct ListRoomsTpl<'a> {
    base: TplContext<'a>,
    open_rooms: Vec<Room>,
    closed_rooms: Vec<Room>,
}

#[get("/rooms")]
#[tracing::instrument(skip_all)]
async fn my_rooms<'a>(
    ctx: &State<Context>,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<ListRoomsTpl<'a>> {
    let author_filter = if session.0.is_admin {
        Author::Any
    } else {
        Author::User(session.user_id())
    };

    let mut conn = ctx.db_pool.get().await?;

    Ok(ListRoomsTpl {
        base: TplContext::from_session("rooms", session.0, cookies),
        open_rooms: db::list_rooms(
            RoomFilter::default()
                .with_status(db::RoomStatus::Open)
                .with_author(author_filter)
                .with_private(true),
            &mut conn,
        )
        .await?,
        closed_rooms: db::list_rooms(
            RoomFilter::default()
                .with_status(db::RoomStatus::Closed)
                .with_author(author_filter)
                .with_private(true),
            &mut conn,
        )
        .await?,
    })
}
#[get("/create-room")]
#[tracing::instrument(skip_all)]
async fn create_room<'a>(
    session: LoggedInSession,
    index_manager: &State<IndexManager>,
    cookies: &CookieJar<'_>,
) -> Result<EditRoom<'a>> {
    let index = index_manager.index.read().await;

    let base = TplContext::from_session("create-room", session.0, cookies);
    Ok(EditRoom {
        room: None,
        room_settings_form: RoomSettingsBuilder::new(base.clone(), &index)?,
        base,
    })
}

fn parse_date(date: &str, tz_offset: i32) -> Result<DateTime<Utc>> {
    let offset = chrono::FixedOffset::west_opt(tz_offset * 60)
        .ok_or_else(|| ap_lobby::error::Error(anyhow::anyhow!("Wrong timezone offset")))?;
    let datetime = chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M")?;
    let date = offset
        .from_local_datetime(&datetime)
        .single()
        .ok_or_else(|| ap_lobby::error::Error(anyhow::anyhow!("Cannot parse passed datetime")))?;

    Ok(date.into())
}

#[post("/create-room", data = "<room_form>")]
#[tracing::instrument(skip_all)]
async fn create_room_submit<'a>(
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    index_manager: &State<IndexManager>,
    mut room_form: Form<CreateRoomForm<'a>>,
    session: LoggedInSession,
) -> Result<Redirect> {
    redirect_to.set("/create-room");

    validate_room_form(&mut room_form)?;
    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.me, &index)
    }?;

    let author_id = session.user_id();
    let close_date = parse_date(room_form.close_date, room_form.tz_offset)?;
    let new_room = NewRoom {
        id: Uuid::new_v4(),
        name: room_form.room_name.trim(),
        close_date: close_date.naive_utc(),
        description: room_form.room_description.trim(),
        room_url: "",
        author_id: Some(author_id),
        private: room_form.private,
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
    let new_room = db::create_room(&new_room, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{}", new_room.id)))
}

#[get("/edit-room/<room_id>")]
#[tracing::instrument(skip(ctx, session, cookies, index_manager))]
async fn edit_room<'a>(
    ctx: &State<Context>,
    room_id: Uuid,
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

#[get("/delete-room/<room_id>")]
#[tracing::instrument(skip(ctx, session))]
async fn delete_room<'a>(
    ctx: &State<Context>,
    room_id: Uuid,
    session: LoggedInSession,
) -> Result<Redirect> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.settings.author_id);

    if !is_my_room {
        return Err(anyhow::anyhow!("You're not allowed to delete this room").into());
    }

    db::delete_room(&room_id, &mut conn).await?;

    Ok(Redirect::to("/"))
}

#[post("/edit-room/<room_id>", data = "<room_form>")]
#[tracing::instrument(skip(redirect_to, room_form, index_manager, ctx, session))]
async fn edit_room_submit<'a>(
    redirect_to: &RedirectTo,
    room_id: Uuid,
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

    validate_room_form(&mut room_form)?;

    let room_manifest = {
        let index = index_manager.index.read().await;
        manifest_from_form(&room_form.me, &index)
    }?;

    let new_room = NewRoom {
        id: room_id,
        name: room_form.room_name.trim(),
        description: room_form.room_description.trim(),
        close_date: parse_date(room_form.close_date, room_form.tz_offset)?.naive_utc(),
        room_url: room_form.room_url,
        author_id: None, // (Skips updating that field)
        private: room_form.private,
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

    db::update_room(&new_room, &mut conn).await?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

fn validate_room_form(room_form: &mut Form<CreateRoomForm<'_>>) -> Result<()> {
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
        create_room,
        my_rooms,
        create_room_submit,
        edit_room,
        delete_room,
        edit_room_submit
    ]
}
