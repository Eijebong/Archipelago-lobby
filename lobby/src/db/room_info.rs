use crate::db::RoomId;
use crate::error::Result;
use crate::schema::room_info;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Serialize;

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = room_info)]
pub struct RoomInfo {
    pub room_id: RoomId,
    pub host: String,
    pub port: i32,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = room_info)]
pub struct NewRoomInfo<'a> {
    pub room_id: RoomId,
    pub host: &'a str,
    pub port: i32,
}

#[tracing::instrument(skip(conn))]
pub async fn get_room_info(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<Option<RoomInfo>> {
    Ok(room_info::table
        .find(room_id)
        .select(RoomInfo::as_select())
        .first::<RoomInfo>(conn)
        .await
        .optional()?)
}

#[tracing::instrument(skip(conn))]
pub async fn create_room_info<'a>(
    new_room_info: &'a NewRoomInfo<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<RoomInfo> {
    Ok(diesel::insert_into(room_info::table)
        .values(new_room_info)
        .returning(RoomInfo::as_returning())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn update_room_info<'a>(
    room_id: RoomId,
    new_room_info: &'a NewRoomInfo<'a>,
    conn: &mut AsyncPgConnection,
) -> Result<RoomInfo> {
    Ok(diesel::update(room_info::table.find(room_id))
        .set((
            room_info::host.eq(new_room_info.host),
            room_info::port.eq(new_room_info.port),
        ))
        .returning(RoomInfo::as_returning())
        .get_result(conn)
        .await?)
}

#[tracing::instrument(skip(conn))]
pub async fn delete_room_info(room_id: RoomId, conn: &mut AsyncPgConnection) -> Result<()> {
    diesel::delete(room_info::table.find(room_id))
        .execute(conn)
        .await?;
    Ok(())
}
