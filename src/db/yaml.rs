use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::{Insertable, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::db::{Json, Room, RoomId, YamlId};
use crate::error::{Error, Result};
use crate::extractor::YamlFeatures;
use crate::schema::{discord_users, rooms, yamls};

use super::YamlValidationStatus;

#[derive(Insertable, Debug)]
#[diesel(table_name=yamls)]
pub struct NewYaml<'a> {
    pub id: YamlId,
    pub room_id: RoomId,
    pub owner_id: i64,
    pub content: &'a str,
    pub player_name: &'a str,
    pub game: &'a str,
    pub features: Json<YamlFeatures>,
    pub validation_status: YamlValidationStatus,
    pub apworlds: Vec<(String, Version)>,
    pub last_error: Option<String>,
}

#[derive(Debug, Selectable, Queryable, Serialize)]
pub struct Yaml {
    pub id: YamlId,
    pub content: String,
    pub game: String,
    pub player_name: String,
    #[serde(skip)]
    pub owner_id: i64,
    pub validation_status: YamlValidationStatus,
    pub apworlds: Vec<(String, Version)>,
    #[serde(skip)]
    pub last_validation_time: NaiveDateTime,
    pub last_error: Option<String>,
    pub patch: Option<String>,
}

#[derive(Clone, Debug, Selectable, Queryable)]
#[diesel(table_name = yamls)]
pub struct YamlWithoutContent {
    pub id: YamlId,
    pub player_name: String,
    pub game: String,
    pub owner_id: i64,
    pub features: Json<YamlFeatures>,
    pub validation_status: YamlValidationStatus,
    pub patch: Option<String>,
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

#[tracing::instrument(skip_all, fields(new_yaml.room_id))]
pub async fn add_yaml_to_room(new_yaml: NewYaml<'_>, conn: &mut AsyncPgConnection) -> Result<()> {
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

#[tracing::instrument(skip(conn))]
pub async fn reset_yaml_validation_status(
    yaml_id: YamlId,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(yamls::table.find(yaml_id))
        .set(yamls::validation_status.eq(YamlValidationStatus::Unknown))
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn update_yaml_status(
    yaml_id: YamlId,
    validation_status: YamlValidationStatus,
    error: Option<String>,
    apworlds: Vec<(String, Version)>,
    validation_time: DateTime<Utc>,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(yamls::table.find(yaml_id))
        .set((
            yamls::validation_status.eq(validation_status),
            yamls::apworlds.eq(apworlds),
            yamls::last_error.eq(error),
            yamls::last_validation_time.eq(validation_time.naive_utc()),
        ))
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn associate_patch_files(
    associations: HashMap<YamlId, String>,
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    conn.transaction::<(), Error, _>(|conn| {
        async move {
            diesel::update(yamls::table.filter(yamls::room_id.eq(room_id)))
                .set(yamls::patch.eq(Option::<String>::None))
                .execute(conn)
                .await?;

            for (yaml_id, patch_path) in associations.iter() {
                diesel::update(yamls::table.find(yaml_id))
                    .set(yamls::patch.eq(Some(patch_path)))
                    .execute(conn)
                    .await?;
            }

            Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(())
}

impl Yaml {
    pub fn sanitized_name(&self) -> String {
        sanitize_yaml_name(&self.player_name)
    }
}

impl YamlWithoutContent {
    pub fn sanitized_name(&self) -> String {
        sanitize_yaml_name(&self.player_name)
    }
}

fn sanitize_yaml_name(name: &str) -> String {
    name.replace(['/', '\\', '<', '>', ':', '?', '*', '|', '"'], "_")
}
