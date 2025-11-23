use chrono::{DateTime, Utc};
use diesel::expression::BoxableExpression;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Serialize;

use crate::schema::fuzz_results;

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = fuzz_results)]
pub struct FuzzResult {
    pub id: i64,
    pub world_name: String,
    pub version: String,
    pub checksum: String,
    pub total: i32,
    pub success: i32,
    pub failure: i32,
    pub timeout: i32,
    pub ignored: i32,
    pub task_id: String,
    pub pr_number: Option<i32>,
    pub extra_args: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = fuzz_results)]
pub struct NewFuzzResult<'a> {
    pub world_name: &'a str,
    pub version: &'a str,
    pub checksum: &'a str,
    pub total: i32,
    pub success: i32,
    pub failure: i32,
    pub timeout: i32,
    pub ignored: i32,
    pub task_id: &'a str,
    pub pr_number: Option<i32>,
    pub extra_args: Option<&'a str>,
}

pub async fn insert_fuzz_results(
    conn: &mut AsyncPgConnection,
    results: Vec<NewFuzzResult<'_>>,
) -> QueryResult<()> {
    diesel::insert_into(fuzz_results::table)
        .values(&results)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn get_fuzz_results_for_world(
    conn: &mut AsyncPgConnection,
    world_name: &str,
    limit: i64,
    offset: i64,
) -> QueryResult<Vec<FuzzResult>> {
    fuzz_results::table
        .filter(fuzz_results::world_name.eq(world_name))
        .order(fuzz_results::recorded_at.desc())
        .limit(limit)
        .offset(offset)
        .select(FuzzResult::as_select())
        .load(conn)
        .await
}

#[derive(Debug, Serialize)]
pub struct PreviousResult {
    pub match_type: MatchType,
    #[serde(flatten)]
    pub result: FuzzResult,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    SameVersion,
    MostRecent,
    Main,
}

type BoxedFilter<'a> = Box<
    dyn BoxableExpression<fuzz_results::table, diesel::pg::Pg, SqlType = diesel::sql_types::Bool>
        + 'a,
>;

async fn fetch_one_for_world(
    conn: &mut AsyncPgConnection,
    world_name: &str,
    extra_args: Option<&str>,
    filter: BoxedFilter<'_>,
) -> QueryResult<Option<FuzzResult>> {
    let mut query = fuzz_results::table
        .filter(fuzz_results::world_name.eq(world_name))
        .filter(filter)
        .order(fuzz_results::recorded_at.desc())
        .into_boxed();

    query = match extra_args {
        Some(args) => query.filter(fuzz_results::extra_args.eq(args)),
        None => query.filter(fuzz_results::extra_args.is_null()),
    };

    query
        .select(FuzzResult::as_select())
        .first(conn)
        .await
        .optional()
}

pub async fn get_previous_results(
    conn: &mut AsyncPgConnection,
    world_name: &str,
    version: &str,
    checksum: &str,
    extra_args: Option<&str>,
) -> QueryResult<Vec<PreviousResult>> {
    let latest_main = fetch_one_for_world(
        conn,
        world_name,
        extra_args,
        Box::new(fuzz_results::pr_number.is_null()),
    )
    .await?;

    let most_recent = fetch_one_for_world(
        conn,
        world_name,
        extra_args,
        Box::new(
            fuzz_results::version
                .ne(version)
                .or(fuzz_results::checksum.ne(checksum)),
        ),
    )
    .await?;

    let same_version = fetch_one_for_world(
        conn,
        world_name,
        extra_args,
        Box::new(
            fuzz_results::version
                .eq(version)
                .and(fuzz_results::checksum.ne(checksum)),
        ),
    )
    .await?;

    let mut results = Vec::new();

    let mut seen_ids = std::collections::HashSet::new();

    for (result, match_type) in [
        (latest_main, MatchType::Main),
        (most_recent, MatchType::MostRecent),
        (same_version, MatchType::SameVersion),
    ] {
        if let Some(r) = result {
            // Prevent inserting the same results twice if they're the same for latest PR/latest main
            if seen_ids.insert(r.id) {
                results.push(PreviousResult {
                    match_type,
                    result: r,
                });
            }
        }
    }

    Ok(results)
}
