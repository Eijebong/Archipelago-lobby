use std::collections::HashMap;

use crate::diesel_uuid::Uuid as DieselUuid;
use crate::error::Result;
use crate::schema::{discord_users, rooms, yamls};
use crate::Context;

use chrono::{NaiveDateTime, Utc};
use diesel::dsl::{exists, now};
use diesel::prelude::*;
use diesel::query_dsl::JoinOnDsl;
use rocket::State;
use uuid::Uuid;

#[derive(Insertable, diesel::AsChangeset, Debug)]
#[diesel(table_name=rooms)]
pub struct NewRoom<'a> {
    pub id: DieselUuid,
    pub name: &'a str,
    pub close_date: NaiveDateTime,
    pub description: &'a str,
    pub room_url: &'a str,
    pub author_id: Option<i64>,
}

#[derive(Insertable)]
#[diesel(table_name=yamls)]
pub struct NewYaml<'a> {
    id: DieselUuid,
    room_id: DieselUuid,
    owner_id: i64,
    content: &'a str,
    player_name: &'a str,
    game: &'a str,
}

#[derive(Debug, diesel::Queryable, diesel::Selectable)]
pub struct Room {
    pub id: DieselUuid,
    pub name: String,
    pub close_date: NaiveDateTime,
    pub description: String,
    pub room_url: String,
    pub author_id: i64,
}

impl Room {
    pub fn is_closed(&self) -> bool {
        self.close_date < chrono::offset::Utc::now().naive_utc()
    }
}

#[derive(Debug, diesel::Queryable)]
pub struct Yaml {
    pub id: DieselUuid,
    pub room_id: DieselUuid,
    pub content: String,
    pub player_name: String,
    pub game: String,
    pub owner_id: i64,
}

#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
pub enum YamlGame {
    Name(String),
    Map(HashMap<String, f64>),
}

#[derive(serde::Deserialize, Debug)]
pub struct YamlFile {
    pub game: YamlGame,
    pub name: String,
}

#[derive(Clone, Copy)]
pub enum RoomStatus {
    Open,
    Closed,
    //Any,
}

#[derive(Clone, Copy)]
pub enum Author {
    Any,
    User(i64),
}

pub fn list_rooms(
    status: RoomStatus,
    author: Author,
    max: i64,
    ctx: &State<Context>,
) -> Result<Vec<Room>> {
    let mut conn = ctx.db_pool.get()?;
    let query = rooms::table
        .order(rooms::close_date.asc())
        .limit(max)
        .into_boxed();

    let query = match status {
        RoomStatus::Open => query.filter(rooms::close_date.gt(now)),
        RoomStatus::Closed => query.filter(rooms::close_date.lt(now)),
        //RoomStatus::Any => query,
    };

    let query = match author {
        Author::User(user_id) => query.filter(rooms::author_id.eq(user_id)),
        Author::Any => query,
    };

    Ok(query.load::<Room>(&mut conn)?)
}

pub fn list_room_with_yaml_from(
    player_id: i64,
    status: RoomStatus,
    max: i64,
    ctx: &State<Context>,
) -> Result<Vec<Room>> {
    let mut conn = ctx.db_pool.get()?;
    let query = rooms::table
        .filter(exists(
            yamls::table.filter(
                yamls::room_id
                    .eq(rooms::id)
                    .and(yamls::owner_id.eq(player_id)),
            ),
        ))
        .limit(max)
        .into_boxed();
    let query = match status {
        RoomStatus::Open => query
            .filter(rooms::close_date.gt(now))
            .order(rooms::close_date.asc()),
        RoomStatus::Closed => query
            .filter(rooms::close_date.lt(now))
            .order(rooms::close_date.desc()),
        //RoomStatus::Any => query.order(rooms::close_date.asc()),
    };

    Ok(query.load::<Room>(&mut conn)?)
}

pub fn create_room(
    name: &str,
    description: &str,
    close_date: &chrono::DateTime<Utc>,
    author_id: i64,
    ctx: &State<Context>,
) -> Result<Room> {
    let mut conn = ctx.db_pool.get()?;

    let new_room = NewRoom {
        id: DieselUuid::random(),
        close_date: close_date.naive_utc(),
        name,
        description,
        room_url: "",
        author_id: Some(author_id),
    };
    diesel::insert_into(rooms::table)
        .values(&new_room)
        .execute(&mut conn)?;

    Ok(Room {
        id: new_room.id,
        name: new_room.name.to_string(),
        close_date: close_date.naive_utc(),
        description: new_room.description.to_string(),
        room_url: "".into(),
        author_id,
    })
}

pub fn update_room(new_room: &NewRoom, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;

    diesel::update(rooms::table)
        .filter(rooms::id.eq(new_room.id))
        .set(new_room)
        .execute(&mut conn)?;

    Ok(())
}

pub fn get_yamls_for_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Vec<Yaml>> {
    let mut conn = ctx.db_pool.get()?;
    let room = rooms::table.find(DieselUuid(uuid)).first::<Room>(&mut conn);
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(DieselUuid(uuid)))
        .get_results::<Yaml>(&mut conn)?)
}

pub fn get_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Room> {
    let mut conn = ctx.db_pool.get()?;
    Ok(rooms::table
        .find(DieselUuid(uuid))
        .first::<Room>(&mut conn)?)
}

pub fn get_room_and_author(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<(Room, String)> {
    let mut conn = ctx.db_pool.get()?;

    Ok(rooms::table
        .find(DieselUuid(uuid))
        .inner_join(discord_users::table)
        .select((Room::as_select(), discord_users::username))
        .first(&mut conn)?)
}

pub fn add_yaml_to_room(
    uuid: uuid::Uuid,
    owner_id: i64,
    content: &str,
    parsed: &YamlFile,
    ctx: &State<Context>,
) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;
    let game_name = match &parsed.game {
        YamlGame::Name(name) => name.clone(),
        YamlGame::Map(map) => {
            if map.len() == 1 {
                map.keys().next().unwrap().clone()
            } else {
                "Unknown".to_string()
            }
        }
    };

    let new_yaml = NewYaml {
        id: DieselUuid::random(),
        owner_id,
        room_id: DieselUuid(uuid),
        content,
        player_name: &parsed.name,
        game: &game_name,
    };
    diesel::insert_into(yamls::table)
        .values(new_yaml)
        .execute(&mut conn)?;

    Ok(())
}

pub fn remove_yaml(yaml_id: uuid::Uuid, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;
    diesel::delete(yamls::table.find(DieselUuid(yaml_id))).execute(&mut conn)?;

    Ok(())
}

pub fn get_yaml_by_id(yaml_id: Uuid, ctx: &State<Context>) -> Result<Yaml> {
    let mut conn = ctx.db_pool.get()?;
    Ok(yamls::table
        .find(DieselUuid(yaml_id))
        .first::<Yaml>(&mut conn)?)
}

impl Yaml {
    pub fn sanitized_name(&self) -> String {
        self.player_name.replace(['/', '\\'], "_")
    }
}

#[derive(Insertable, Queryable)]
#[diesel(table_name=discord_users)]
pub struct DiscordUser {
    pub id: i64,
    pub username: String,
}

pub fn upsert_discord_user(discord_id: i64, username: &str, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;

    let discord_user = DiscordUser {
        id: discord_id,
        username: username.to_string(),
    };

    diesel::insert_into(discord_users::table)
        .values(&discord_user)
        .on_conflict(discord_users::id)
        .do_update()
        .set(discord_users::username.eq(username))
        .execute(&mut conn)?;

    Ok(())
}
