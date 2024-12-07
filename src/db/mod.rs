use crate::error::Result;
use crate::schema::{rooms, yamls};

use diesel::dsl::{exists, now, AsSelect, SqlTypeOf};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

pub mod instrumentation;
mod json;
mod room;
mod room_template;
pub mod types;
mod user;
mod yaml;

pub use json::Json;
pub use room::*;
pub use room_template::*;
pub use types::*;
pub use user::*;
pub use yaml::*;

#[derive(Clone, Copy, Debug)]
pub enum RoomStatus {
    Open,
    Closed,
    Any,
}

#[derive(Clone, Copy, Debug)]
pub enum Author {
    Any,
    User(i64),
    IncludeUser(i64),
}

#[derive(Clone, Copy, Debug)]
pub enum WithYaml {
    Any,
    OnlyFor(i64),
    AndFor(i64),
}

#[tracing::instrument(skip(conn))]
pub async fn list_rooms(
    room_filter: RoomFilter,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<Room>> {
    let query = room_filter.as_query();

    Ok(query.load::<Room>(conn).await.unwrap())
}

#[derive(Debug)]
pub struct RoomFilter {
    pub with_yaml_from: WithYaml,
    pub author: Author,
    pub room_status: RoomStatus,
    pub max: i64,
}

impl Default for RoomFilter {
    fn default() -> Self {
        Self {
            with_yaml_from: WithYaml::Any,
            author: Author::Any,
            room_status: RoomStatus::Any,
            max: 50,
        }
    }
}
impl RoomFilter {
    pub fn as_query<'f>(&self) -> rooms::BoxedQuery<'f, Pg, SqlTypeOf<AsSelect<Room, Pg>>> {
        let query = rooms::table
            .select(Room::as_select())
            .limit(self.max)
            .into_boxed();

        let query = match self.author {
            Author::User(user_id) => query.filter(rooms::author_id.eq(user_id)),
            Author::IncludeUser(user_id) => query.or_filter(rooms::author_id.eq(user_id)),
            Author::Any => query,
        };

        let query = match self.with_yaml_from {
            WithYaml::OnlyFor(user_id) => query.filter(exists(
                yamls::table.filter(
                    yamls::room_id
                        .eq(rooms::id)
                        .and(yamls::owner_id.eq(user_id)),
                ),
            )),
            WithYaml::AndFor(user_id) => query.or_filter(exists(
                yamls::table.filter(
                    yamls::room_id
                        .eq(rooms::id)
                        .and(yamls::owner_id.eq(user_id)),
                ),
            )),
            WithYaml::Any => query,
        };

        match self.room_status {
            RoomStatus::Open => query
                .filter(rooms::close_date.gt(now))
                .order(rooms::close_date.asc()),
            RoomStatus::Closed => query
                .filter(rooms::close_date.lt(now))
                .order(rooms::close_date.desc()),
            RoomStatus::Any => query.order(rooms::close_date.asc()),
        }
    }

    pub fn with_status(mut self, status: RoomStatus) -> Self {
        self.room_status = status;
        self
    }

    pub fn with_max(mut self, max: i64) -> Self {
        self.max = max;
        self
    }

    pub fn with_yamls_from(mut self, with_yaml_from: WithYaml) -> Self {
        self.with_yaml_from = with_yaml_from;
        self
    }

    pub fn with_author(mut self, author: Author) -> Self {
        self.author = author;
        self
    }
}
