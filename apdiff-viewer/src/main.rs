use std::{borrow::Cow, collections::HashMap, ffi::OsStr, io::Cursor, path::PathBuf};

use apwm::diff::CombinedDiff;
use askama::Template;
use rocket::{
    http::{ContentType, Status},
    response::{self, Responder},
    routes, Request, Response, State,
};
use serde::Deserialize;
use taskcluster::{ClientBuilder, Queue};

mod filters {
    use apwm::diff::VersionRange;

    pub fn fmt_version(range: &VersionRange) -> askama::Result<String> {
        Ok(match (&range.0, &range.1) {
            (None, Some(new_version)) => {
                format!("<span>✅ {}</span>", new_version)
            }
            (Some(old_version), None) => {
                format!("<span>❌ {}</span>", old_version)
            }
            (Some(old_version), Some(new_version)) => {
                format!("<span>{} -> {}</span>", old_version, new_version)
            }
            (None, None) => unreachable!(),
        })
    }

    pub fn base64(b64: &String) -> askama::Result<String> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(b64))
    }

    pub fn dashify(original: &str) -> askama::Result<String> {
        Ok(original.replace([' ', '\'', '&', ';'], "-"))
    }
}

#[derive(Debug)]
pub struct Error(pub anyhow::Error);
pub type Result<T> = std::result::Result<T, Error>;

impl<'r> Responder<'r, 'static> for Error {
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

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    diffs: Vec<CombinedDiff>,
}

#[derive(Template)]
#[template(path = "tests.html")]
struct TestPage {
    results: TestResults,
}

#[rocket::get("/<task_id>")]
async fn get_task_diffs(task_id: &str, queue: &State<Queue>) -> Result<Index> {
    let artifacts = get_task_artifacts(queue, task_id).await?;
    let diff_artifacts = artifacts
        .iter()
        .filter(|path| path.starts_with("public/diffs/"))
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
        diffs.push(diff);
    }

    Ok(Index { diffs })
}

#[derive(Deserialize)]
struct TestResult {
    traceback: String,
    description: String,
}

#[derive(Deserialize)]
struct TestResults {
    failures: HashMap<String, TestResult>,
    errors: HashMap<String, TestResult>,
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
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
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
        .mount("/", routes![get_task_diffs, dist, get_test_results])
        .launch()
        .await?;

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
