//! APDiff Viewer - Server-side rendered git diff viewer for Archipelago World packages
//!
//! This application displays git diffs for APWorld packages with syntax highlighting
//! and annotation support. It's been converted from React to server-side rendering
//! using Askama templates for better performance and simpler deployment.

use std::{borrow::Cow, collections::BTreeMap, ffi::OsStr, io::Cursor, path::PathBuf};

use apwm::diff::CombinedDiff;
use askama::Template;
use askama_web::WebTemplate;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::AsyncPgConnection;
use futures::future::try_join_all;
use rocket::{
    http::{ContentType, Status},
    response::{self, Responder},
    routes, Request, Response, State,
};
use serde::Deserialize;
use std::sync::OnceLock;
use syntect::{
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};
use taskcluster::{ClientBuilder, Queue};

mod api;
mod db;
mod diff;
mod guards;
mod schema;

use diff::{parse_git_diff, Annotations, FileDiff};

// Global syntax set and theme - initialized once for performance
static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

pub fn get_syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

pub fn get_theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let theme_file = Asset::get("github-dark.tmTheme")
            .expect("github-dark.tmTheme should be embedded in binary");
        let theme_xml = std::str::from_utf8(&theme_file.data)
            .expect("github-dark.tmTheme should be valid UTF-8");
        ThemeSet::load_from_reader(&mut std::io::Cursor::new(theme_xml))
            .expect("github-dark.tmTheme should be valid theme XML")
    })
}

#[derive(Debug)]
pub struct Error(pub anyhow::Error);
pub type Result<T> = std::result::Result<T, Error>;

impl Responder<'_, 'static> for Error {
    fn respond_to(self, _: &Request<'_>) -> response::Result<'static> {
        let error = self.0.to_string();
        Response::build()
            .status(Status::InternalServerError)
            .sized_body(error.len(), Cursor::new(error))
            .ok()
    }
}

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Error(error.into())
    }
}

#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
struct Index {
    task_id: String,
    apworld_diffs: Vec<ApworldDiff>,
}

#[derive(Debug)]
struct ApworldDiff {
    world_name: String,
    versions: Vec<VersionDiff>,
}

#[derive(Debug)]
struct VersionDiff {
    version_range: String,
    version_id: String,
    files: Vec<FileDiff>,
}

#[derive(Template, WebTemplate)]
#[template(path = "tests.html")]
struct TestPage {
    results: TestResults,
}

#[rocket::get("/<task_id>")]
async fn get_task_diffs(task_id: &str, queue: &State<Queue>) -> Result<Index> {
    let artifacts = get_task_artifacts(queue, task_id).await?;

    let diff_artifacts: Vec<_> = artifacts
        .iter()
        .filter(|path| path.starts_with("public/diffs/") && path.ends_with(".apdiff"))
        .collect();

    if diff_artifacts.is_empty() {
        return Err(anyhow::anyhow!(
            "This doesn't look like a supported task, it contains no apdiffs"
        )
        .into());
    }

    let diffs = try_join_all(
        diff_artifacts
            .into_iter()
            .map(|name| process_diff_artifact(queue, task_id, name, &artifacts)),
    )
    .await?;

    let apworld_diffs = diffs
        .into_iter()
        .map(|(diff, annotations)| process_apworld_diff(diff, annotations))
        .collect();

    Ok(Index {
        task_id: task_id.to_string(),
        apworld_diffs,
    })
}

/// Process a single diff artifact and its annotations
async fn process_diff_artifact(
    queue: &Queue,
    task_id: &str,
    name: &str,
    artifacts: &[String],
) -> Result<(
    CombinedDiff,
    BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
)> {
    // Fetch and deserialize the diff
    let diff_url = queue.getLatestArtifact_url(task_id, name)?;
    let diff_text = reqwest::get(&diff_url).await?.text().await?;
    let diff = deserialize_json::<CombinedDiff>(&diff_text)?;

    let annotation_prefix = format!("public/diffs/{}-", diff.apworld_name);
    let annotations = try_join_all(
        artifacts
            .iter()
            .filter(|path| path.starts_with(&annotation_prefix) && path.ends_with(".aplint"))
            .map(|file| process_annotation_file(queue, task_id, file, &annotation_prefix)),
    )
    .await?
    .into_iter()
    .collect();

    Ok((diff, annotations))
}

/// Process a single annotation file
async fn process_annotation_file(
    queue: &Queue,
    task_id: &str,
    file: &str,
    prefix: &str,
) -> Result<(String, BTreeMap<String, Vec<Annotations>>)> {
    let version = file
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(".aplint"))
        .ok_or_else(|| anyhow::anyhow!("Invalid aplint filename: {}", file))?;

    let aplint_url = queue.getLatestArtifact_url(task_id, file)?;
    let aplint_text = reqwest::get(&aplint_url).await?.text().await?;
    let annotation = deserialize_json::<BTreeMap<String, Vec<Annotations>>>(&aplint_text)?;

    Ok((version.to_string(), annotation))
}

/// Transform a combined diff into an ApworldDiff structure
fn process_apworld_diff(
    diff: CombinedDiff,
    annotations: BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
) -> ApworldDiff {
    let versions = diff
        .diffs
        .iter()
        .filter_map(|(version_range, diff_content)| {
            let git_diff = match diff_content {
                apwm::diff::Diff::VersionAdded { content, .. } => content,
                _ => return None,
            };

            let version_string = serde_json::to_string(version_range)
                .expect("Version range should be serializable to JSON");
            let version_range_clean = version_string.trim_matches('"');

            let version_id = version_range_clean.split("...").nth(1).unwrap_or("HEAD");

            let files = parse_git_diff(git_diff, &annotations, version_id);

            Some(VersionDiff {
                version_range: version_range_clean.to_string(),
                version_id: version_id.to_string(),
                files,
            })
        })
        .collect();

    ApworldDiff {
        world_name: diff.world_name,
        versions,
    }
}

/// Helper function for JSON deserialization with better error reporting
fn deserialize_json<T: for<'de> serde::Deserialize<'de>>(text: &str) -> Result<T> {
    let mut deser = serde_json::Deserializer::from_str(text);
    Ok(serde_path_to_error::deserialize(&mut deser)?)
}

#[derive(Deserialize)]
struct TestResult {
    traceback: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct UnexpectedSuccess {
    description: Option<String>,
}

#[derive(Deserialize)]
struct TestResults {
    failures: BTreeMap<String, TestResult>,
    errors: BTreeMap<String, TestResult>,
    #[serde(default)]
    unexpected_successes: BTreeMap<String, UnexpectedSuccess>,
    #[serde(default)]
    expected_failures: BTreeMap<String, TestResult>,
    version: String,
    world_name: String,
}

#[rocket::get("/tests/<task_id>")]
async fn get_test_results(task_id: &str, queue: &State<Queue>) -> Result<TestPage> {
    let artifacts = get_task_artifacts(queue, task_id).await?;
    let Some(aptest_name) = artifacts
        .iter()
        .find(|path| path.starts_with("public/test_results/"))
    else {
        Err(anyhow::anyhow!(
            "This doesn't look like a supported task, it contains no test_results"
        ))?
    };

    let aptest_url = queue.getLatestArtifact_url(task_id, aptest_name)?;
    let aptest = reqwest::get(&aptest_url).await?.text().await?;
    let mut deser = serde_json::Deserializer::from_str(&aptest);
    let results: TestResults = serde_path_to_error::deserialize(&mut deser)?;

    Ok(TestPage { results })
}

#[derive(rust_embed::RustEmbed)]
#[folder = "./static/"]
struct Asset;

#[rocket::get("/static/<file..>")]
fn dist_static(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let filename = file.display().to_string();
    let asset = Asset::get(&filename)?;
    let content_type = file
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Binary);

    Some((content_type, asset.data))
}

use diesel_migrations::{embed_migrations, EmbeddedMigrations};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let client_builder = ClientBuilder::new(std::env::var("TASKCLUSTER_ROOT_URL")?);
    let queue = Queue::new(client_builder)?;

    let db_url = std::env::var("DATABASE_URL")?;
    let db_pool: Pool<AsyncPgConnection> =
        common::db::get_database_pool(&db_url, MIGRATIONS).await?;

    let fuzz_api_key = guards::FuzzApiKeyConfig(std::env::var("FUZZ_API_KEY")?);

    rocket::build()
        .manage(queue)
        .manage(db_pool)
        .manage(fuzz_api_key)
        .mount("/", routes![get_task_diffs, dist_static, get_test_results])
        .mount("/api", api::routes())
        .launch()
        .await
        .map_err(|e| anyhow::anyhow!("Rocket launch failed: {}", e))?;

    Ok(())
}

async fn get_task_artifacts(queue: &Queue, task_id: &str) -> anyhow::Result<Vec<String>> {
    let mut continuation_token = None;
    let mut all_artifacts = Vec::new();

    loop {
        let artifacts_page = queue
            .listLatestArtifacts(task_id, continuation_token.as_deref(), None)
            .await?;

        continuation_token = artifacts_page
            .get("continuationToken")
            .and_then(|token| token.as_str().map(String::from));

        if let Some(artifacts) = artifacts_page.get("artifacts").and_then(|v| v.as_array()) {
            let page_artifacts: Vec<String> = artifacts
                .iter()
                .filter_map(|v| v.get("name")?.as_str().map(String::from))
                .collect();
            all_artifacts.extend(page_artifacts);
        }

        if continuation_token.is_none() {
            break;
        }
    }

    Ok(all_artifacts)
}
