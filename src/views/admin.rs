#![allow(clippy::blocks_in_conditions)]

use crate::error::{RedirectTo, Result};
use crate::views::auth::AdminSession;
use askama::Template;
use chrono::{DateTime, TimeZone, Utc};
use rocket::form::Form;
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket::{get, post, FromForm};
use uuid::Uuid;

use crate::db::{self, NewRoom, Room};
use crate::{Context, TplContext};
use rocket::State;

#[derive(Template)]
#[template(path = "admin/create_room.html")]
struct EditRoom<'a> {
    base: TplContext<'a>,
    room: Option<Room>,
}

#[derive(FromForm, Debug)]
struct CreateRoomForm<'a> {
    room_name: &'a str,
    close_date: &'a str,
    tz_offset: i32,
}

#[derive(Template)]
#[template(path = "admin/rooms.html")]
struct ListRoomsTpl<'a> {
    base: TplContext<'a>,
    rooms: Vec<Room>,
}

#[get("/rooms")]
fn rooms<'a>(
    ctx: &State<Context>,
    session: AdminSession,
    cookies: &CookieJar,
) -> Result<ListRoomsTpl<'a>> {
    Ok(ListRoomsTpl {
        base: TplContext::from_session("rooms", session.0, cookies),
        rooms: db::list_rooms(db::RoomStatus::Any, 50, ctx)?,
    })
}
#[get("/create-room")]
fn create_room<'a>(session: AdminSession, cookies: &CookieJar) -> Result<EditRoom<'a>> {
    Ok(EditRoom {
        base: TplContext::from_session("create-room", session.0, cookies),
        room: None,
    })
}

fn parse_date(date: &str, tz_offset: i32) -> Result<DateTime<Utc>> {
    let offset = chrono::FixedOffset::west_opt(tz_offset * 60)
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Wrong timezone offset")))?;
    let datetime = chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M")?;
    let date = offset
        .from_local_datetime(&datetime)
        .single()
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Cannot parse passed datetime")))?;

    Ok(date.into())
}

#[post("/create-room", data = "<room_form>")]
fn create_room_submit(
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    room_form: Form<CreateRoomForm>,
    _session: AdminSession,
) -> Result<Redirect> {
    redirect_to.set("/admin/create_room");

    let close_date = parse_date(room_form.close_date, room_form.tz_offset)?;
    let new_room = db::create_room(room_form.room_name, &close_date, ctx)?;

    Ok(Redirect::to(format!("/room/{}", new_room.id)))
}

#[get("/edit-room/<room_id>")]
fn edit_room<'a>(
    ctx: &State<Context>,
    room_id: Uuid,
    session: AdminSession,
    cookies: &CookieJar,
) -> Result<EditRoom<'a>> {
    let room = crate::db::get_room(room_id, ctx)?;

    Ok(EditRoom {
        base: TplContext::from_session("create-room", session.0, cookies),
        room: Some(room),
    })
}

#[post("/edit-room/<room_id>", data = "<room_form>")]
fn edit_room_submit(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    room_form: Form<CreateRoomForm>,
    ctx: &State<Context>,
    _session: AdminSession,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let new_room = NewRoom {
        id: crate::diesel_uuid::Uuid(room_id),
        name: room_form.room_name,
        close_date: parse_date(room_form.close_date, room_form.tz_offset)?.naive_utc(),
    };
    crate::db::update_room(&new_room, ctx)?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        create_room,
        rooms,
        create_room_submit,
        edit_room,
        edit_room_submit
    ]
}
