use std::collections::HashMap;

use crate::diesel_uuid::Uuid as DieselUuid;
use crate::error::Result;
use crate::schema::{rooms, yamls};
use crate::Context;

use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use rocket::State;
use uuid::Uuid;

#[derive(diesel::Insertable)]
#[diesel(table_name=rooms)]
pub struct NewRoom<'a> {
    pub id: DieselUuid,
    pub name: &'a str,
    pub close_date: NaiveDateTime,
}

#[derive(diesel::Insertable)]
#[diesel(table_name=yamls)]
pub struct NewYaml<'a> {
    id: DieselUuid,
    room_id: DieselUuid,
    owner_id: DieselUuid,
    content: &'a str,
    player_name: &'a str,
    game: &'a str,
}

#[derive(Debug, diesel::Queryable)]
pub struct Room {
    pub id: DieselUuid,
    pub name: String,
    pub close_date: NaiveDateTime,
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
    pub owner_id: DieselUuid,
    pub content: String,
    pub player_name: String,
    pub game: String,
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

pub fn list_rooms(ctx: &State<Context>) -> Result<Vec<Room>> {
    let mut conn = ctx.db_pool.get()?;
    Ok(rooms::table
        .order(rooms::close_date.asc())
        .load::<Room>(&mut conn)?)
}

pub fn create_room(
    name: &str,
    close_date: &chrono::DateTime<Utc>,
    ctx: &State<Context>,
) -> Result<Room> {
    let mut conn = ctx.db_pool.get()?;

    let new_room = NewRoom {
        id: DieselUuid::random(),
        close_date: close_date.naive_utc(),
        name,
    };
    diesel::insert_into(rooms::table)
        .values(&new_room)
        .execute(&mut conn)?;

    Ok(Room {
        id: new_room.id,
        name: new_room.name.to_string(),
        close_date: close_date.naive_utc(),
    })
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

pub fn add_yaml_to_room(
    uuid: uuid::Uuid,
    owner_id: uuid::Uuid,
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
        owner_id: DieselUuid(owner_id),
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
