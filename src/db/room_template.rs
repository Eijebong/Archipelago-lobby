use apwm::Manifest;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::error::Result;
use crate::schema::rooms;
use crate::{
    db::{Json, Room, RoomTemplate, RoomTemplateId},
    schema::room_templates,
};

#[derive(Insertable, AsChangeset, Debug)]
#[diesel(table_name=room_templates)]
pub struct NewRoomTemplate<'a> {
    pub id: RoomTemplateId,
    pub tpl_name: &'a str,
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
    pub global: bool,
}

#[tracing::instrument(skip(conn))]
pub async fn get_room_templates_for_author(
    author_id: i64,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<RoomTemplate>> {
    Ok(room_templates::table
        .filter(
            room_templates::author_id
                .eq(author_id)
                .or(room_templates::global.eq(true)),
        )
        .select(RoomTemplate::as_select())
        .order_by((room_templates::global.desc(), room_templates::created_at))
        .get_results(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn get_room_template_by_id(
    room_tpl_id: RoomTemplateId,
    conn: &mut AsyncPgConnection,
) -> Result<RoomTemplate> {
    Ok(room_templates::table
        .find(room_tpl_id)
        .select(RoomTemplate::as_select())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn create_room_template<'a>(
    new_tpl: &'a NewRoomTemplate<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<RoomTemplate> {
    Ok(diesel::insert_into(room_templates::table)
        .values(new_tpl)
        .returning(RoomTemplate::as_returning())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn update_room_template<'a>(
    new_tpl: &'a NewRoomTemplate<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(room_templates::table)
        .filter(room_templates::id.eq(&new_tpl.id))
        .set(new_tpl)
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn delete_room_template(
    tpl_id: RoomTemplateId,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::delete(room_templates::table)
        .filter(room_templates::id.eq(tpl_id))
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(conn))]
pub async fn list_rooms_from_template(
    tpl_id: RoomTemplateId,
    author_id: i64,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<Room>> {
    Ok(rooms::table
        .filter(
            rooms::from_template_id
                .eq(Some(tpl_id))
                .and(rooms::author_id.eq(author_id)),
        )
        .select(Room::as_select())
        .get_results(conn)
        .await?)
}
