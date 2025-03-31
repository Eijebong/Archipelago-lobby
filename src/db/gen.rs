use std::str::FromStr;

use crate::error::Error;
use crate::schema::yamls;
use crate::{error::Result, schema::generations};
use anyhow::anyhow;
use diesel::result::OptionalExtension;
use diesel::{ExpressionMethods, Insertable, QueryDsl};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;
use wq::JobId;

use super::RoomId;

#[derive(PartialEq)]
pub enum GenerationStatus {
    Pending,
    Running,
    Failed,
    Done,
}

impl FromStr for GenerationStatus {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(Self::Pending),
            "Running" => Ok(Self::Running),
            "Failed" => Ok(Self::Failed),
            "Done" => Ok(Self::Done),
            v => Err(crate::error::Error(anyhow!(
                "Unknown variant for GenerationStatus: {}",
                v
            ))),
        }
    }
}

impl GenerationStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Failed => "Failed",
            Self::Done => "Done",
        }
    }
}

pub struct Generation {
    pub room_id: RoomId,
    pub job_id: JobId,
    pub status: GenerationStatus,
}

#[derive(Insertable)]
#[diesel(table_name = generations)]
pub struct NewGeneration {
    room_id: RoomId,
    job_id: Uuid,
    status: String,
}

pub async fn get_generation_for_room(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<Option<Generation>> {
    Ok(generations::table
        .select((
            generations::room_id,
            generations::job_id,
            generations::status,
        ))
        .find(room_id)
        .first::<(RoomId, Uuid, String)>(conn)
        .await
        .optional()?
        .map(|(room_id, job_id, status)| Generation {
            room_id,
            job_id: job_id.into(),
            status: GenerationStatus::from_str(&status)
                .expect("Invalid value for GenerationStatus in database"),
        }))
}

pub async fn insert_generation_for_room(
    room_id: RoomId,
    job_id: JobId,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::insert_into(generations::table)
        .values(NewGeneration {
            room_id,
            job_id: job_id.into(),
            status: GenerationStatus::Pending.as_str().to_string(),
        })
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn delete_generation_for_room(
    room_id: RoomId,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    conn.transaction::<(), Error, _>(|conn| {
        async move {
            diesel::delete(generations::table.find(room_id))
                .execute(conn)
                .await?;

            diesel::update(yamls::table.filter(yamls::room_id.eq(room_id)))
                .set(yamls::patch.eq(Option::<String>::None))
                .execute(conn)
                .await?;

            Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(())
}

pub async fn update_generation_status_for_room(
    room_id: RoomId,
    new_status: GenerationStatus,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    diesel::update(generations::table.find(room_id))
        .set(generations::status.eq(new_status.as_str()))
        .execute(conn)
        .await?;

    Ok(())
}
