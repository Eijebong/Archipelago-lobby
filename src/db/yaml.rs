use std::collections::HashMap;

use diesel::prelude::*;
use diesel::{Insertable, Queryable, Selectable};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Deserialize;

use crate::db::{Json, Room, RoomId, YamlId};
use crate::error::Result;
use crate::extractor::YamlFeatures;
use crate::schema::{discord_users, rooms, yamls};

#[derive(Insertable)]
#[diesel(table_name=yamls)]
pub struct NewYaml<'a> {
    id: YamlId,
    room_id: RoomId,
    owner_id: i64,
    content: &'a str,
    player_name: &'a str,
    game: &'a str,
    features: Json<YamlFeatures>,
}

#[derive(Debug, Selectable, Queryable)]
pub struct Yaml {
    pub content: String,
    pub player_name: String,
    pub owner_id: i64,
}

#[derive(Debug, Selectable, Queryable)]
#[diesel(table_name = yamls)]
pub struct YamlWithoutContent {
    pub id: YamlId,
    pub player_name: String,
    pub game: String,
    pub owner_id: i64,
    pub features: Json<YamlFeatures>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum YamlGame {
    Name(String),
    Map(HashMap<String, f64>),
}

#[derive(Deserialize, Debug)]
pub struct YamlFile {
    pub game: YamlGame,
    pub name: String,
}

#[tracing::instrument(skip(conn))]
pub async fn get_yamls_for_room_with_author_names(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<(YamlWithoutContent, String)>> {
    let room = rooms::table
        .find(&room_id)
        .select(Room::as_select())
        .first::<Room>(conn)
        .await;
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(&room_id))
        .inner_join(discord_users::table)
        .select((YamlWithoutContent::as_select(), discord_users::username))
        .get_results(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn get_yamls_for_room(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<Yaml>> {
    let room = rooms::table
        .find(&room_id)
        .select(Room::as_select())
        .first::<Room>(conn)
        .await;
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(&room_id))
        .select(Yaml::as_select())
        .get_results::<Yaml>(conn)
        .await?)
}

#[tracing::instrument(skip(conn, content))]
pub async fn add_yaml_to_room(
    room_id: RoomId,
    owner_id: i64,
    game_name: &str,
    content: &str,
    parsed: &YamlFile,
    features: YamlFeatures,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let new_yaml = NewYaml {
        id: YamlId::new_v4(),
        owner_id,
        room_id,
        content,
        player_name: &parsed.name,
        game: game_name,
        features: Json(features),
    };

    diesel::insert_into(yamls::table)
        .values(new_yaml)
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn remove_yaml(yaml_id: YamlId, conn: &mut AsyncPgConnection) -> Result<()> {
    diesel::delete(yamls::table.find(yaml_id))
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn get_yaml_by_id(yaml_id: YamlId, conn: &mut AsyncPgConnection) -> Result<Yaml> {
    Ok(yamls::table
        .find(yaml_id)
        .select(Yaml::as_select())
        .first::<Yaml>(conn)
        .await?)
}

impl Yaml {
    pub fn sanitized_name(&self) -> String {
        self.player_name.replace(['/', '\\'], "_")
    }
}
