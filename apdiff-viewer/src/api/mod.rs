use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::AsyncPgConnection;
use rocket::serde::json::Json;
use rocket::{routes, Route, State};
use serde::Deserialize;

use crate::db::{self, FuzzResult, NewFuzzResult, PreviousResult};
use crate::Result;

#[derive(Debug, Deserialize)]
pub struct RecordFuzzResultsRequest {
    pub task_id: String,
    pub pr_number: Option<i32>,
    pub extra_args: Option<String>,
    pub results: Vec<FuzzResultInput>,
}

#[derive(Debug, Deserialize)]
pub struct FuzzResultInput {
    pub world_name: String,
    pub version: String,
    pub checksum: String,
    pub total: i32,
    pub success: i32,
    pub failure: i32,
    pub timeout: i32,
    pub ignored: i32,
}

#[rocket::post("/fuzz-results", data = "<request>")]
async fn record_fuzz_results(
    pool: &State<Pool<AsyncPgConnection>>,
    request: Json<RecordFuzzResultsRequest>,
) -> Result<()> {
    let mut conn = pool.get().await?;

    let new_results: Vec<NewFuzzResult> = request
        .results
        .iter()
        .map(|r| NewFuzzResult {
            world_name: &r.world_name,
            version: &r.version,
            checksum: &r.checksum,
            total: r.total,
            success: r.success,
            failure: r.failure,
            timeout: r.timeout,
            ignored: r.ignored,
            task_id: &request.task_id,
            pr_number: request.pr_number,
            extra_args: request.extra_args.as_deref(),
        })
        .collect();

    db::insert_fuzz_results(&mut conn, new_results).await?;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct FuzzResultsResponse {
    pub results: Vec<FuzzResult>,
}

#[rocket::get("/fuzz-results/<world_name>?<limit>&<offset>")]
async fn get_fuzz_results(
    pool: &State<Pool<AsyncPgConnection>>,
    world_name: &str,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Json<FuzzResultsResponse>> {
    let mut conn = pool.get().await?;

    let results = db::get_fuzz_results_for_world(
        &mut conn,
        world_name,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
    .await?;

    Ok(Json(FuzzResultsResponse { results }))
}

#[derive(Debug, serde::Serialize)]
pub struct PreviousResultsResponse {
    pub previous_results: Vec<PreviousResult>,
}

#[rocket::get("/fuzz-results/<world_name>/previous?<version>&<checksum>")]
async fn get_previous_results(
    pool: &State<Pool<AsyncPgConnection>>,
    world_name: &str,
    version: &str,
    checksum: &str,
) -> Result<Json<PreviousResultsResponse>> {
    let mut conn = pool.get().await?;

    let previous_results =
        db::get_previous_results(&mut conn, world_name, version, checksum).await?;

    Ok(Json(PreviousResultsResponse { previous_results }))
}

pub fn routes() -> Vec<Route> {
    routes![record_fuzz_results, get_fuzz_results, get_previous_results]
}
