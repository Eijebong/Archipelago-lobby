use crate::error::Result;
use crate::schema::{rooms, yamls};

use diesel::dsl::{count, exists, now, AsSelect, IntervalDsl, SqlTypeOf};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

mod gen;
mod json;
mod pagination;
mod room;
mod room_template;
pub mod types;
mod user;
mod yaml;

pub use gen::*;
pub use json::Json;
pub use pagination::{Paginate, Paginated};
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
}

#[derive(Clone, Copy, Debug)]
pub enum WithYaml {
    Any,
    AndFor(i64),
}

#[derive(Clone, Copy, Debug)]
pub enum OpenState {
    Any,
    Open,
    Closed,
}

#[tracing::instrument(skip(conn))]
pub async fn list_rooms(
    room_filter: RoomFilter,
    page: Option<u64>,
    conn: &mut AsyncPgConnection,
) -> Result<(Vec<Room>, u64)> {
    if let Some(page) = page {
        let query = room_filter.as_query().paginate(page);

        Ok(query.load_and_count_pages::<Room>(conn).await?)
    } else {
        let query = room_filter.as_query();
        Ok((query.load::<Room>(conn).await?, 1))
    }
}

#[derive(Debug)]
pub struct RoomFilter {
    with_yaml_from: WithYaml,
    author: Author,
    open_state: OpenState,
}

impl Default for RoomFilter {
    fn default() -> Self {
        Self {
            with_yaml_from: WithYaml::Any,
            author: Author::Any,
            open_state: OpenState::Any,
        }
    }
}

impl RoomFilter {
    pub fn as_query<'f>(&self) -> rooms::BoxedQuery<'f, Pg, SqlTypeOf<AsSelect<Room, Pg>>> {
        let query = rooms::table.select(Room::as_select()).into_boxed();

        let query = match self.author {
            Author::User(user_id) => query.filter(rooms::author_id.eq(user_id)),
            Author::Any => query,
        };

        let query = match self.with_yaml_from {
            WithYaml::AndFor(user_id) => query.or_filter(exists(
                yamls::table.filter(
                    yamls::room_id
                        .eq(rooms::id)
                        .and(yamls::owner_id.eq(user_id)),
                ),
            )),
            WithYaml::Any => query,
        };

        let query = match self.open_state {
            OpenState::Any => query,
            OpenState::Open => query.filter(rooms::close_date.gt(now)),
            OpenState::Closed => query.filter(rooms::close_date.lt(now)),
        };

        query.order_by(rooms::close_date.desc())
    }

    pub fn with_yamls_from(mut self, with_yaml_from: WithYaml) -> Self {
        self.with_yaml_from = with_yaml_from;
        self
    }

    pub fn with_author(mut self, author: Author) -> Self {
        self.author = author;
        self
    }

    pub fn with_open_state(mut self, open_state: OpenState) -> Self {
        self.open_state = open_state;
        self
    }
}

pub async fn get_room_stats(conn: &mut AsyncPgConnection) -> Result<Vec<(i64, RoomId)>> {
    Ok(yamls::table
        .inner_join(rooms::table.on(yamls::room_id.eq(rooms::id)))
        .filter(
            rooms::close_date
                .gt(now)
                .or(rooms::updated_at.lt(now - 1.minute())),
        )
        .group_by(rooms::id)
        .select((count(yamls::id), rooms::id))
        .get_results::<(i64, RoomId)>(conn)
        .await?)
}
