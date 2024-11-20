use crate::db::RoomId;
use anyhow::Context;
use apwm::{Index, Manifest};
use chrono::{NaiveDateTime, Timelike};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel::{AsChangeset, Insertable, Queryable, Selectable};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::db::Json;
use crate::error::Result;
use crate::schema::{discord_users, rooms};
#[derive(Insertable, AsChangeset, Debug)]
#[diesel(table_name=rooms)]
pub struct NewRoom<'a> {
    pub id: RoomId,
    pub name: &'a str,
    pub close_date: NaiveDateTime,
    pub description: &'a str,
    pub room_url: &'a str,
    pub author_id: Option<i64>,
    #[diesel(treat_none_as_null = true)]
    pub yaml_limit_per_user: Option<i32>,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
    pub yaml_limit_bypass_list: Vec<i64>,
    pub manifest: Json<Manifest>,
    pub show_apworlds: bool,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(check_for_backend(Pg))]
pub struct Room {
    pub id: RoomId,
    #[diesel(embed)]
    pub settings: RoomSettings,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(check_for_backend(Pg))]
#[diesel(table_name=rooms)]
pub struct RoomSettings {
    pub name: String,
    pub close_date: NaiveDateTime,
    pub description: String,
    pub room_url: String,
    pub author_id: i64,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
    pub yaml_limit_per_user: Option<i32>,
    pub yaml_limit_bypass_list: Vec<i64>,
    pub manifest: Json<Manifest>,
    pub show_apworlds: bool,
}

impl RoomSettings {
    pub fn default(index: &Index) -> Result<Self> {
        Ok(Self {
            name: "".to_string(),
            close_date: chrono::Utc::now()
                .naive_utc()
                .with_second(0)
                .context("Failed to create default datetime")?,
            description: "".to_string(),
            room_url: "".to_string(),
            author_id: -1,
            yaml_validation: true,
            allow_unsupported: false,
            yaml_limit_per_user: None,
            yaml_limit_bypass_list: vec![],
            manifest: Json(Manifest::from_index_with_latest_versions(index)?),
            show_apworlds: true,
        })
    }
}

impl Room {
    pub fn is_closed(&self) -> bool {
        self.settings.close_date < chrono::offset::Utc::now().naive_utc()
    }
}

#[tracing::instrument(skip(conn))]
pub async fn create_room<'a>(
    new_room: &'a NewRoom<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<Room> {
    Ok(diesel::insert_into(rooms::table)
        .values(new_room)
        .returning(Room::as_returning())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn update_room<'a>(
    new_room: &'a NewRoom<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(rooms::table)
        .filter(rooms::id.eq(&new_room.id))
        .set(new_room)
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn delete_room(room_id: RoomId, conn: &mut AsyncPgConnection) -> Result<()> {
    diesel::delete(rooms::table)
        .filter(rooms::id.eq(room_id))
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn get_room(room_id: RoomId, conn: &mut AsyncPgConnection) -> Result<Room> {
    Ok(rooms::table
        .find(room_id)
        .select(Room::as_select())
        .first::<Room>(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn get_room_and_author(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<(Room, String)> {
    Ok(rooms::table
        .find(room_id)
        .inner_join(discord_users::table)
        .select((Room::as_select(), discord_users::username))
        .first(conn)
        .await?)
}
