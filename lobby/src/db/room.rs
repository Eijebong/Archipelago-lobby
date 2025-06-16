use crate::db::RoomId;
use anyhow::Context;
use apwm::{Index, Manifest};
use chrono::{NaiveDateTime, Timelike};
use diesel::backend::Backend;
use diesel::deserialize::FromStaticSqlRow;
use diesel::dsl::now;
use diesel::prelude::*;
use diesel::{AsChangeset, Insertable, Queryable, Selectable};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::db::Json;
use crate::error::Result;
use crate::schema::{discord_users, room_templates, rooms, yamls};

use super::{RoomTemplateId, YamlValidationStatus};

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
    pub from_template_id: Option<Option<RoomTemplateId>>,
    pub allow_invalid_yamls: bool,
    pub meta_file: String,
}

#[derive(Debug, Clone)]
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
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub allow_invalid_yamls: bool,
    pub meta_file: String,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: RoomId,
    pub settings: RoomSettings,
    pub from_template_id: Option<RoomTemplateId>,
}

#[derive(Debug, Clone)]
pub struct RoomTemplate {
    pub id: RoomTemplateId,
    pub settings: RoomSettings,
    pub global: bool,
    pub tpl_name: String,
}

impl<DB: Backend> Selectable<DB> for Room {
    type SelectExpression = <rooms::table as Table>::AllColumns;

    fn construct_selection() -> Self::SelectExpression {
        rooms::all_columns
    }
}

impl<DB: Backend> Selectable<DB> for RoomTemplate {
    type SelectExpression = <room_templates::table as Table>::AllColumns;

    fn construct_selection() -> Self::SelectExpression {
        room_templates::all_columns
    }
}

impl<
        DB: Backend,
        ST0,
        ST1,
        ST2,
        ST3,
        ST4,
        ST5,
        ST6,
        ST7,
        ST8,
        ST9,
        ST10,
        ST11,
        ST12,
        ST13,
        ST14,
        ST15,
        ST16,
    >
    Queryable<
        (
            ST0,
            ST1,
            ST2,
            ST3,
            ST4,
            ST5,
            ST6,
            ST7,
            ST8,
            ST9,
            ST10,
            ST11,
            ST12,
            ST13,
            ST14,
            ST15,
            ST16,
        ),
        DB,
    > for Room
where
    (
        RoomId,
        String,
        NaiveDateTime,
        String,
        String,
        i64,
        bool,
        bool,
        Option<i32>,
        Vec<i64>,
        Json<Manifest>,
        bool,
        NaiveDateTime,
        NaiveDateTime,
        Option<RoomTemplateId>,
        bool,
        String,
    ): FromStaticSqlRow<
        (
            ST0,
            ST1,
            ST2,
            ST3,
            ST4,
            ST5,
            ST6,
            ST7,
            ST8,
            ST9,
            ST10,
            ST11,
            ST12,
            ST13,
            ST14,
            ST15,
            ST16,
        ),
        DB,
    >,
{
    type Row = (
        RoomId,
        String,
        NaiveDateTime,
        String,
        String,
        i64,
        bool,
        bool,
        Option<i32>,
        Vec<i64>,
        Json<Manifest>,
        bool,
        NaiveDateTime,
        NaiveDateTime,
        Option<RoomTemplateId>,
        bool,
        String,
    );

    fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
        Ok(Room {
            id: row.0,
            settings: RoomSettings {
                name: row.1,
                close_date: row.2,
                description: row.3,
                room_url: row.4,
                author_id: row.5,
                yaml_validation: row.6,
                allow_unsupported: row.7,
                yaml_limit_per_user: row.8,
                yaml_limit_bypass_list: row.9,
                manifest: row.10,
                show_apworlds: row.11,
                created_at: row.12,
                updated_at: row.13,
                allow_invalid_yamls: row.15,
                meta_file: row.16,
            },
            from_template_id: row.14,
        })
    }
}

impl<
        DB: Backend,
        ST0,
        ST1,
        ST2,
        ST3,
        ST4,
        ST5,
        ST6,
        ST7,
        ST8,
        ST9,
        ST10,
        ST11,
        ST12,
        ST13,
        ST14,
        ST15,
        ST16,
        ST17,
    >
    Queryable<
        (
            ST0,
            ST1,
            ST2,
            ST3,
            ST4,
            ST5,
            ST6,
            ST7,
            ST8,
            ST9,
            ST10,
            ST11,
            ST12,
            ST13,
            ST14,
            ST15,
            ST16,
            ST17,
        ),
        DB,
    > for RoomTemplate
where
    (
        RoomTemplateId,
        String,
        NaiveDateTime,
        String,
        String,
        i64,
        bool,
        bool,
        Option<i32>,
        Vec<i64>,
        Json<Manifest>,
        bool,
        NaiveDateTime,
        NaiveDateTime,
        bool,
        String,
        bool,
        String,
    ): FromStaticSqlRow<
        (
            ST0,
            ST1,
            ST2,
            ST3,
            ST4,
            ST5,
            ST6,
            ST7,
            ST8,
            ST9,
            ST10,
            ST11,
            ST12,
            ST13,
            ST14,
            ST15,
            ST16,
            ST17,
        ),
        DB,
    >,
{
    type Row = (
        RoomTemplateId,
        String,
        NaiveDateTime,
        String,
        String,
        i64,
        bool,
        bool,
        Option<i32>,
        Vec<i64>,
        Json<Manifest>,
        bool,
        NaiveDateTime,
        NaiveDateTime,
        bool,
        String,
        bool,
        String,
    );

    fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
        Ok(RoomTemplate {
            id: row.0,
            settings: RoomSettings {
                name: row.1,
                close_date: row.2,
                description: row.3,
                room_url: row.4,
                author_id: row.5,
                yaml_validation: row.6,
                allow_unsupported: row.7,
                yaml_limit_per_user: row.8,
                yaml_limit_bypass_list: row.9,
                manifest: row.10,
                show_apworlds: row.11,
                created_at: row.12,
                updated_at: row.13,
                allow_invalid_yamls: row.16,
                meta_file: row.17,
            },
            global: row.14,
            tpl_name: row.15,
        })
    }
}

impl RoomSettings {
    pub fn default(index: &Index) -> Result<Self> {
        Ok(Self {
            name: "".to_string(),
            close_date: Self::default_close_date()?,
            description: "".to_string(),
            room_url: "".to_string(),
            author_id: -1,
            yaml_validation: true,
            allow_unsupported: false,
            yaml_limit_per_user: None,
            yaml_limit_bypass_list: vec![],
            manifest: Json(Manifest::from_index_with_default_versions(index)?),
            show_apworlds: true,
            created_at: Self::default_close_date()?,
            updated_at: Self::default_close_date()?,
            allow_invalid_yamls: false,
            meta_file: "".to_string(),
        })
    }

    pub fn default_close_date() -> Result<NaiveDateTime> {
        Ok(chrono::Utc::now()
            .naive_utc()
            .with_second(0)
            .context("Failed to create default datetime")?)
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
) -> Result<Room> {
    if !new_room.yaml_validation {
        diesel::update(yamls::table)
            .filter(yamls::room_id.eq(new_room.id))
            .set((
                yamls::validation_status.eq(YamlValidationStatus::Unknown),
                yamls::apworlds.eq(Vec::<(String, semver::Version)>::new()),
                yamls::last_error.eq(Option::<String>::None),
                yamls::last_validation_time.eq(now),
            ))
            .execute(conn)
            .await?;
    }

    Ok(diesel::update(rooms::table)
        .filter(rooms::id.eq(&new_room.id))
        .set(new_room)
        .returning(Room::as_returning())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn update_room_manifest<'a>(
    room_id: RoomId,
    new_manifest: &Manifest,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(rooms::table.find(room_id))
        .set(rooms::manifest.eq(Json(new_manifest)))
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
