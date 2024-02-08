#![allow(clippy::blocks_in_conditions)]

use crate::error::{RedirectTo, Result};
use crate::views::auth::AdminSession;
use askama::Template;
use chrono::TimeZone;
use rocket::form::Form;
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket::{get, post, FromForm};

use crate::api::{self, Room};
use crate::{Context, TplContext};
use rocket::State;

#[derive(Template)]
#[template(path = "admin/create_room.html")]
struct CreateRoom<'a> {
    base: TplContext<'a>,
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
        rooms: api::list_rooms(ctx)?,
    })
}
#[get("/create-room")]
fn create_room<'a>(
    _ctx: &State<Context>,
    session: AdminSession,
    cookies: &CookieJar,
) -> Result<CreateRoom<'a>> {
    Ok(CreateRoom {
        base: TplContext::from_session("create-room", session.0, cookies),
    })
}

#[post("/create-room", data = "<room_form>")]
fn create_room_submit(
    redirect_to: &RedirectTo,
    ctx: &State<Context>,
    room_form: Form<CreateRoomForm>,
) -> Result<Redirect> {
    redirect_to.set("/admin/create_room");

    let offset = chrono::FixedOffset::west_opt(room_form.tz_offset * 60)
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Wrong timezone offset")))?;
    let datetime = chrono::NaiveDateTime::parse_from_str(room_form.close_date, "%Y-%m-%dT%H:%M")?;
    let close_date = offset
        .from_local_datetime(&datetime)
        .single()
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Cannot parse passed datetime")))?;
    let new_room = api::create_room(room_form.room_name, &close_date.into(), ctx)?;

    Ok(Redirect::to(format!("/room/{}", new_room.id)))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![create_room, rooms, create_room_submit]
}
