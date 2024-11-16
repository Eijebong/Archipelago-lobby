use crate::error::Result;
use crate::schema::{discord_users, rooms, yamls};

use diesel::dsl::{exists, now, AsSelect, SqlTypeOf};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

pub mod instrumentation;
mod json;
mod room;
mod yaml;

pub use json::Json;
pub use room::*;
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

#[derive(Insertable, Queryable)]
#[diesel(table_name=discord_users)]
pub struct DiscordUser {
    pub id: i64,
    pub username: String,
}

#[tracing::instrument(skip(conn, discord_id), fields(%discord_id))]
pub async fn upsert_discord_user(
    discord_id: i64,
    username: &str,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let discord_user = DiscordUser {
        id: discord_id,
        username: username.to_string(),
    };

    diesel::insert_into(discord_users::table)
        .values(&discord_user)
        .on_conflict(discord_users::id)
        .do_update()
        .set(discord_users::username.eq(username))
        .execute(conn)
        .await?;

    Ok(())
}

#[derive(Debug)]
pub struct RoomFilter {
    pub show_private: bool,
    pub with_yaml_from: WithYaml,
    pub author: Author,
    pub room_status: RoomStatus,
    pub max: i64,
}

impl RoomFilter {
    pub fn new() -> Self {
        RoomFilter {
            show_private: false,
            with_yaml_from: WithYaml::Any,
            author: Author::Any,
            room_status: RoomStatus::Any,
            max: 50,
        }
    }

    pub fn as_query<'f>(&self) -> rooms::BoxedQuery<'f, Pg, SqlTypeOf<AsSelect<Room, Pg>>> {
        let query = rooms::table
            .select(Room::as_select())
            .limit(self.max)
            .into_boxed();

        let query = if !self.show_private {
            query.filter(rooms::private.eq(false))
        } else {
            query
        };

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

    pub fn with_private(mut self, private: bool) -> Self {
        self.show_private = private;
        self
    }
}
