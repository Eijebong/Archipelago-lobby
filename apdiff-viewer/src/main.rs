//! APDiff Viewer - Server-side rendered git diff viewer for Archipelago World packages
//!
//! This application displays git diffs for APWorld packages with syntax highlighting
//! and annotation support. It's been converted from React to server-side rendering
//! using Askama templates for better performance and simpler deployment.

use std::{borrow::Cow, collections::BTreeMap, ffi::OsStr, io::Cursor, path::PathBuf};

use apwm::diff::CombinedDiff;
use askama::Template;
use askama_web::WebTemplate;
use rocket::{
    http::{ContentType, Status},
    response::{self, Responder},
    routes,
    serde::json::Json,
    Request, Response, State,
};
use serde::Deserialize;
use std::sync::OnceLock;
use syntect::{
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};
use taskcluster::{ClientBuilder, Queue};

mod diff;

use diff::{parse_git_diff, Annotations, FileDiff};

// Global syntax set and theme - initialized once for performance
static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

pub fn get_syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

pub fn get_theme() -> &'static Theme {
    THEME.get_or_init(|| {
        // Try to load custom GitHub Dark theme from embedded assets
        if let Some(theme_file) = Asset::get("github-dark.tmTheme") {
            let theme_xml = std::str::from_utf8(&theme_file.data).unwrap_or("");
            match ThemeSet::load_from_reader(&mut std::io::Cursor::new(theme_xml)) {
                Ok(theme) => {
                    return theme;
                }
                Err(e) => {
                    eprintln!("Failed to parse embedded GitHub Dark theme: {e}");
                }
            }
        }

        // Fallback to built-in theme
        let theme_set = ThemeSet::load_defaults();
        theme_set
            .themes
            .get("base16-eighties.dark")
            .unwrap()
            .clone()
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

#[rocket::get("/api/diffs/<task_id>")]
async fn get_task_diffs_api(
    task_id: &str,
    queue: &State<Queue>,
) -> Result<
    Json<
        Vec<(
            CombinedDiff,
            BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
        )>,
    >,
> {
    let artifacts = get_task_artifacts(queue, task_id).await?;
    let diff_artifacts = artifacts
        .iter()
        .filter(|path| path.starts_with("public/diffs/") && path.ends_with(".apdiff"))
        .collect::<Vec<_>>();
    if diff_artifacts.is_empty() {
        Err(anyhow::anyhow!(
            "This doesn't look like a supported task, it contains no apdiffs"
        ))?
    }
    let mut diffs = vec![];

    for name in diff_artifacts {
        let diff_url = queue.getLatestArtifact_url(task_id, name)?;
        let diff = reqwest::get(&diff_url).await?.text().await?;
        let mut deser = serde_json::Deserializer::from_str(&diff);
        let diff: CombinedDiff = serde_path_to_error::deserialize(&mut deser)?;

        let annotations_files = artifacts
            .iter()
            .filter(|path| {
                path.starts_with(&format!("public/diffs/{}-", diff.apworld_name))
                    && path.ends_with(".aplint")
            })
            .collect::<Vec<_>>();
        let mut annotations = BTreeMap::new();
        for file in &annotations_files {
            let version = file
                .strip_prefix(&format!("public/diffs/{}-", diff.apworld_name))
                .unwrap()
                .strip_suffix(".aplint")
                .unwrap()
                .to_string();
            let aplint_url = queue.getLatestArtifact_url(task_id, file)?;
            let aplint = reqwest::get(&aplint_url).await?.text().await?;
            let mut deser = serde_json::Deserializer::from_str(&aplint);
            let annotation: BTreeMap<String, Vec<Annotations>> =
                serde_path_to_error::deserialize(&mut deser)?;

            annotations.insert(version, annotation);
        }

        diffs.push((diff, annotations));
    }

    Ok(Json(diffs))
}

#[rocket::get("/<task_id>")]
async fn get_task_diffs(task_id: &str, queue: &State<Queue>) -> Result<Index> {
    let artifacts = get_task_artifacts(queue, task_id).await?;
    let diff_artifacts = artifacts
        .iter()
        .filter(|path| path.starts_with("public/diffs/") && path.ends_with(".apdiff"))
        .collect::<Vec<_>>();
    if diff_artifacts.is_empty() {
        Err(anyhow::anyhow!(
            "This doesn't look like a supported task, it contains no apdiffs"
        ))?
    }
    let mut diffs = vec![];

    for name in diff_artifacts {
        let diff_url = queue.getLatestArtifact_url(task_id, name)?;
        let diff = reqwest::get(&diff_url).await?.text().await?;
        let mut deser = serde_json::Deserializer::from_str(&diff);
        let diff: CombinedDiff = serde_path_to_error::deserialize(&mut deser)?;

        let annotations_files = artifacts
            .iter()
            .filter(|path| {
                path.starts_with(&format!("public/diffs/{}-", diff.apworld_name))
                    && path.ends_with(".aplint")
            })
            .collect::<Vec<_>>();
        let mut annotations = BTreeMap::new();
        for file in &annotations_files {
            let version = file
                .strip_prefix(&format!("public/diffs/{}-", diff.apworld_name))
                .unwrap()
                .strip_suffix(".aplint")
                .unwrap()
                .to_string();
            let aplint_url = queue.getLatestArtifact_url(task_id, file)?;
            let aplint = reqwest::get(&aplint_url).await?.text().await?;
            let mut deser = serde_json::Deserializer::from_str(&aplint);
            let annotation: BTreeMap<String, Vec<Annotations>> =
                serde_path_to_error::deserialize(&mut deser)?;

            annotations.insert(version, annotation);
        }

        diffs.push((diff, annotations));
    }

    let processed_diffs = diffs
        .into_iter()
        .map(|(diff, annotations)| {
            let versions = diff
                .diffs
                .iter()
                .filter_map(|(version_range, diff_content)| {
                    if let apwm::diff::Diff::VersionAdded(git_diff) = diff_content {
                        let version_string = serde_json::to_string(version_range)
                            .unwrap_or_else(|_| "unknown...unknown".to_string());
                        let version_range_clean = version_string.trim_matches('"');

                        let version_id = version_range_clean
                            .split("...")
                            .nth(1)
                            .filter(|s| !s.is_empty())
                            .unwrap_or("unknown");

                        let files = parse_git_diff(git_diff, &annotations, version_id);

                        Some(VersionDiff {
                            version_range: version_range_clean.to_string(),
                            version_id: version_id.to_string(),
                            files,
                        })
                    } else {
                        None
                    }
                })
                .collect();

            ApworldDiff {
                world_name: if diff.world_name.is_empty() {
                    "Unknown".to_string()
                } else {
                    diff.world_name
                },
                versions,
            }
        })
        .collect();

    Ok(Index {
        task_id: task_id.to_string(),
        apworld_diffs: processed_diffs,
    })
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
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let client_builder = ClientBuilder::new(std::env::var("TASKCLUSTER_ROOT_URL")?);
    let queue = Queue::new(client_builder)?;

    rocket::build()
        .manage(queue)
        .mount(
            "/",
            routes![
                get_task_diffs,
                dist_static,
                get_test_results,
                get_task_diffs_api
            ],
        )
        .launch()
        .await
        .unwrap();

    Ok(())
}

async fn get_task_artifacts(queue: &Queue, task_id: &str) -> anyhow::Result<Vec<String>> {
    let mut continuation_token = None;
    let mut artifacts = vec![];
    loop {
        let artifacts_page = queue
            .listLatestArtifacts(task_id, continuation_token.as_deref(), None)
            .await?;

        continuation_token = artifacts_page
            .get("continuationToken")
            .and_then(|token| token.as_str().map(String::from));

        if let Some(values) = artifacts_page.get("artifacts").and_then(|v| v.as_array()) {
            artifacts.extend(
                values
                    .iter()
                    .filter_map(|v| {
                        v.get("name")
                            .and_then(|name| name.as_str().map(String::from))
                    })
                    .collect::<Vec<_>>(),
            );
        }

        if continuation_token.is_none() {
            break;
        }
    }

    Ok(artifacts)
}
