use std::collections::HashMap;
use std::time::Instant;

use crate::error::Result;
use crate::schema::{discord_users, rooms, yamls};
use crate::Context;

use chrono::NaiveDateTime;
use diesel::connection::Instrumentation;
use diesel::dsl::{exists, now, AsSelect, SqlTypeOf};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
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
    #[diesel(treat_none_as_null = true)]
    pub yaml_limit_per_user: Option<i32>,
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
    pub yaml_limit_per_user: Option<i32>,
}

impl Room {
    pub fn is_closed(&self) -> bool {
        self.close_date < chrono::offset::Utc::now().naive_utc()
    }
}

#[derive(Debug, diesel::Selectable, diesel::Queryable)]
pub struct Yaml {
    pub content: String,
    pub player_name: String,
    pub owner_id: i64,
}

#[derive(Debug, diesel::Selectable, diesel::Queryable)]
#[diesel(table_name = yamls)]
pub struct YamlWithoutContent {
    pub id: Uuid,
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

#[tracing::instrument(skip(ctx))]
pub async fn list_rooms(room_filter: RoomFilter, ctx: &State<Context>) -> Result<Vec<Room>> {
    let mut conn = ctx.db_pool.get().await?;
    let query = room_filter.as_query();

    Ok(query.load::<Room>(&mut conn).await?)
}

#[tracing::instrument(skip(ctx))]
pub async fn create_room<'a>(new_room: &'a NewRoom<'a>, ctx: &State<Context>) -> Result<Room> {
    let mut conn = ctx.db_pool.get().await?;

    Ok(diesel::insert_into(rooms::table)
        .values(new_room)
        .returning(Room::as_returning())
        .get_result(&mut conn)
        .await?)
}

#[tracing::instrument(skip(ctx))]
pub async fn update_room<'a>(new_room: &'a NewRoom<'a>, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get().await?;

    diesel::update(rooms::table)
        .filter(rooms::id.eq(new_room.id))
        .set(new_room)
        .execute(&mut conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(ctx))]
pub async fn delete_room(room_id: &uuid::Uuid, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get().await?;

    diesel::delete(rooms::table)
        .filter(rooms::id.eq(room_id))
        .execute(&mut conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(ctx))]
pub async fn get_yamls_for_room_with_author_names(
    uuid: uuid::Uuid,
    ctx: &State<Context>,
) -> Result<Vec<(YamlWithoutContent, String)>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = rooms::table.find(uuid).first::<Room>(&mut conn).await;
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(uuid))
        .inner_join(discord_users::table)
        .select((YamlWithoutContent::as_select(), discord_users::username))
        .get_results(&mut conn)
        .await?)
}

#[tracing::instrument(skip(ctx))]
pub async fn get_yamls_for_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Vec<Yaml>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = rooms::table.find(uuid).first::<Room>(&mut conn).await;
    let Ok(_room) = room else {
        Err(anyhow::anyhow!("Couldn't get room"))?
    };

    Ok(yamls::table
        .filter(yamls::room_id.eq(uuid))
        .select(Yaml::as_select())
        .get_results::<Yaml>(&mut conn)
        .await?)
}

#[tracing::instrument(skip(ctx))]
pub async fn get_room(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<Room> {
    let mut conn = ctx.db_pool.get().await?;
    Ok(rooms::table.find(uuid).first::<Room>(&mut conn).await?)
}

#[tracing::instrument(skip(ctx))]
pub async fn get_room_and_author(uuid: uuid::Uuid, ctx: &State<Context>) -> Result<(Room, String)> {
    let mut conn = ctx.db_pool.get().await?;

    Ok(rooms::table
        .find(uuid)
        .inner_join(discord_users::table)
        .select((Room::as_select(), discord_users::username))
        .first(&mut conn)
        .await?)
}

#[tracing::instrument(skip(conn, content))]
pub async fn add_yaml_to_room(
    room_id: uuid::Uuid,
    owner_id: i64,
    game_name: &str,
    content: &str,
    parsed: &YamlFile,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let new_yaml = NewYaml {
        id: Uuid::new_v4(),
        owner_id,
        room_id,
        content,
        player_name: &parsed.name,
        game: game_name,
    };

    diesel::insert_into(yamls::table)
        .values(new_yaml)
        .execute(conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(ctx))]
pub async fn remove_yaml(yaml_id: uuid::Uuid, ctx: &State<Context>) -> Result<()> {
    let mut conn = ctx.db_pool.get().await?;
    diesel::delete(yamls::table.find(yaml_id))
        .execute(&mut conn)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(ctx))]
pub async fn get_yaml_by_id(yaml_id: Uuid, ctx: &State<Context>) -> Result<Yaml> {
    let mut conn = ctx.db_pool.get().await?;
    Ok(yamls::table
        .find(yaml_id)
        .select(Yaml::as_select())
        .first::<Yaml>(&mut conn)
        .await?)
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

#[tracing::instrument(skip(ctx, discord_id), fields(%discord_id))]
pub async fn upsert_discord_user(
    discord_id: i64,
    username: &str,
    ctx: &State<Context>,
) -> Result<()> {
    let mut conn = ctx.db_pool.get().await?;

    let discord_user = DiscordUser {
        id: discord_id,
        username: username.to_string(),
    };

    diesel::insert_into(discord_users::table)
        .values(&discord_user)
        .on_conflict(discord_users::id)
        .do_update()
        .set(discord_users::username.eq(username))
        .execute(&mut conn)
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
                tracing::event!(tracing::Level::INFO, "Query started");
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
                tracing::event!(tracing::Level::INFO, %query, "Query finished");
            }
            diesel::connection::InstrumentationEvent::StartEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "StartEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::FinishEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "FinishEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::CacheQuery { .. } => {
                tracing::event!(tracing::Level::INFO, "CacheQuery");
            }
            diesel::connection::InstrumentationEvent::BeginTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "BeginTransaction");
            }
            diesel::connection::InstrumentationEvent::CommitTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "CommitTransaction");
            }
            diesel::connection::InstrumentationEvent::RollbackTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "RollbackTransaction");
            }
            _ => {}
        };
    }
}
