use std::collections::HashMap;
use std::time::Instant;

use crate::error::Result;
use crate::schema::{discord_users, rooms, yamls};
use crate::Context;

use chrono::NaiveDateTime;
use diesel::connection::Instrumentation;
use diesel::dsl::{exists, now, AsSelect, SqlTypeOf};
use diesel::prelude::*;
use diesel::pg::Pg;
use once_cell::sync::Lazy;
use prometheus::{HistogramOpts, HistogramVec};
use rocket::State;
use uuid::Uuid;

#[derive(Insertable, diesel::AsChangeset, Debug)]
#[diesel(table_name=rooms)]
pub struct NewRoom<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub close_date: NaiveDateTime,
    pub description: &'a str,
    pub room_url: &'a str,
    pub author_id: Option<i64>,
    pub private: bool,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
}

#[derive(Insertable)]
#[diesel(table_name=yamls)]
pub struct NewYaml<'a> {
    id: Uuid,
    room_id: Uuid,
    owner_id: i64,
    content: &'a str,
    player_name: &'a str,
    game: &'a str,
}

#[derive(Debug, diesel::Queryable, diesel::Selectable)]
pub struct Room {
    pub id: Uuid,
    pub name: String,
    pub close_date: NaiveDateTime,
    pub description: String,
    pub room_url: String,
    pub author_id: i64,
    pub private: bool,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
}

impl Room {
    pub fn is_closed(&self) -> bool {
        self.close_date < chrono::offset::Utc::now().naive_utc()
    }
}

#[derive(Debug, diesel::Selectable, diesel::Queryable)]
pub struct Yaml {
    pub id: Uuid,
    pub content: String,
    pub player_name: String,
    pub game: String,
    pub owner_id: i64,
}

#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
pub enum YamlGame {
    Name(String),
    Map(HashMap<String, f64>),
}

#[derive(serde::Deserialize, Debug)]
pub struct YamlFile {
    pub game: YamlGame,
    pub name: String,
}

#[derive(Clone, Copy)]
pub enum RoomStatus {
    Open,
    Closed,
    Any,
}

#[derive(Clone, Copy)]
pub enum Author {
    Any,
    User(i64),
    IncludeUser(i64),
}

#[derive(Clone, Copy)]
pub enum WithYaml {
    Any,
    OnlyFor(i64),
    AndFor(i64),
}

pub fn list_rooms(room_filter: RoomFilter, ctx: &State<Context>) -> Result<Vec<Room>> {
    let mut conn = ctx.db_pool.get()?;
    let query = room_filter.as_query();

    Ok(query.load::<Room>(&mut conn)?)
}

pub fn create_room(new_room: &NewRoom, ctx: &State<Context>) -> Result<Room> {
    let mut conn = ctx.db_pool.get()?;

    Ok(diesel::insert_into(rooms::table)
        .values(new_room)
        .returning(Room::as_returning())
        .get_result(&mut conn)?)
}

pub fn update_room(new_room: &NewRoom, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;

    diesel::update(rooms::table)
        .filter(rooms::id.eq(new_room.id))
        .set(new_room)
        .execute(&mut conn)?;

    Ok(())
}

pub fn get_yamls_for_room_with_author_names(
    uuid: uuid::Uuid,
    ctx: &State<Context>,
) -> Result<Vec<(Yaml, String)>> {
    let mut conn = ctx.db_pool.get()?;
    let room = rooms::table.find(uuid).first::<Room>(&mut conn);
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(uuid))
        .inner_join(discord_users::table)
        .select((Yaml::as_select(), discord_users::username))
        .get_results(&mut conn)?)
}

pub fn get_yamls_for_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Vec<Yaml>> {
    let mut conn = ctx.db_pool.get()?;
    let room = rooms::table.find(uuid).first::<Room>(&mut conn);
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(uuid))
        .select(Yaml::as_select())
        .get_results::<Yaml>(&mut conn)?)
}

pub fn get_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Room> {
    let mut conn = ctx.db_pool.get()?;
    Ok(rooms::table
        .find(uuid)
        .first::<Room>(&mut conn)?)
}

pub fn get_room_and_author(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<(Room, String)> {
    let mut conn = ctx.db_pool.get()?;

    Ok(rooms::table
        .find(uuid)
        .inner_join(discord_users::table)
        .select((Room::as_select(), discord_users::username))
        .first(&mut conn)?)
}

pub fn add_yaml_to_room(
    room_id: uuid::Uuid,
    owner_id: i64,
    content: &str,
    parsed: &YamlFile,
    ctx: &State<Context>,
) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;
    let game_name = match &parsed.game {
        YamlGame::Name(name) => name.clone(),
        YamlGame::Map(map) => {
            if map.len() == 1 {
                map.keys().next().unwrap().clone()
            } else {
                "Unknown".to_string()
            }
        }
    };

    let new_yaml = NewYaml {
        id: Uuid::new_v4(),
        owner_id,
        room_id,
        content,
        player_name: &parsed.name,
        game: &game_name,
    };
    diesel::insert_into(yamls::table)
        .values(new_yaml)
        .execute(&mut conn)?;

    Ok(())
}

pub fn remove_yaml(yaml_id: uuid::Uuid, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;
    diesel::delete(yamls::table.find(yaml_id)).execute(&mut conn)?;

    Ok(())
}

pub fn get_yaml_by_id(yaml_id: Uuid, ctx: &State<Context>) -> Result<Yaml> {
    let mut conn = ctx.db_pool.get()?;
    Ok(yamls::table
        .find(yaml_id)
        .select(Yaml::as_select())
        .first::<Yaml>(&mut conn)?)
}

impl Yaml {
    pub fn sanitized_name(&self) -> String {
        self.player_name.replace(['/', '\\'], "_")
    }
}

#[derive(Insertable, Queryable)]
#[diesel(table_name=discord_users)]
pub struct DiscordUser {
    pub id: i64,
    pub username: String,
}

pub fn upsert_discord_user(discord_id: i64, username: &str, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get()?;

    let discord_user = DiscordUser {
        id: discord_id,
        username: username.to_string(),
    };

    diesel::insert_into(discord_users::table)
        .values(&discord_user)
        .on_conflict(discord_users::id)
        .do_update()
        .set(discord_users::username.eq(username))
        .execute(&mut conn)?;

    Ok(())
}

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

#[derive(Default)]
pub struct DbInstrumentation {
    query_start: Option<Instant>,
}

pub(crate) static QUERY_HISTOGRAM: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("diesel_query_seconds", "SQL query duration").buckets(vec![
            0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.1, 1.0,
        ]),
        &["query"],
    )
    .expect("Failed to create query histogram")
});

impl Instrumentation for DbInstrumentation {
    fn on_connection_event(&mut self, event: diesel::connection::InstrumentationEvent<'_>) {
        match event {
            diesel::connection::InstrumentationEvent::StartQuery { .. } => {
                self.query_start = Some(Instant::now());
            }
            diesel::connection::InstrumentationEvent::FinishQuery { query, .. } => {
                let Some(query_start) = self.query_start else {
                    return;
                };
                let elapsed = query_start.elapsed();
                let query = query.to_string().replace('\n', " ");
                let query = query.split("--").next().unwrap().trim();
                QUERY_HISTOGRAM
                    .with_label_values(&[query])
                    .observe(elapsed.as_secs_f64());
            }
            _ => {}
        };
    }
}
