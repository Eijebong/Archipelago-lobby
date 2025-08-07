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
use serde::{Deserialize, Serialize};
use taskcluster::{ClientBuilder, Queue};

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

#[derive(Deserialize, Debug, Serialize)]
struct Annotations {
    pub ty: u64,
    pub desc: String,
    pub severity: u64,
    pub line: Option<u64>,
    pub col_start: Option<u64>,
    pub col_end: Option<u64>,
    pub extra: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "../frontend/build/index.html")]
struct Index {}

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
            let annotation = serde_path_to_error::deserialize(&mut deser)?;

            annotations.insert(version, annotation);
        }

        diffs.push((diff, annotations));
    }

    Ok(Json(diffs))
}

#[rocket::get("/<_task_id>")]
async fn get_task_diffs(_task_id: &str) -> Result<Index> {
    return Ok(Index {});
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

#[derive(rust_embed::RustEmbed)]
#[folder = "./frontend/build"]
struct Dist;

#[rocket::get("/dist/<file..>")]
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let filename = file.display().to_string();
    let asset = Dist::get(&filename)?;
    let content_type = file
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

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
                dist,
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
