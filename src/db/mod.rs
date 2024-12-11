use crate::error::Result;
use crate::schema::{rooms, yamls};

use diesel::dsl::{exists, AsSelect, SqlTypeOf};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::AsyncPgConnection;

pub mod instrumentation;
mod json;
mod pagination;
mod room;
mod room_template;
pub mod types;
mod user;
mod yaml;

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

#[tracing::instrument(skip(conn))]
pub async fn list_rooms(
    room_filter: RoomFilter,
    page: u64,
    conn: &mut AsyncPgConnection,
) -> Result<(Vec<Room>, u64)> {
    let query = room_filter.as_query().paginate(page);

    Ok(query.load_and_count_pages::<Room>(conn).await?)
}

#[derive(Debug)]
pub struct RoomFilter {
    pub with_yaml_from: WithYaml,
    pub author: Author,
}

impl Default for RoomFilter {
    fn default() -> Self {
        Self {
            with_yaml_from: WithYaml::Any,
            author: Author::Any,
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
}
