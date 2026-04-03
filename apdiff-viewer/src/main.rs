use std::{borrow::Cow, collections::BTreeMap, ffi::OsStr, io::Cursor, path::PathBuf};

use apwm::changes::{Changes, Checksum};
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
use semver::Version;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};
use syntect::{
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};
use taskcluster::{ClientBuilder, Credentials, Index, Queue};

mod api;
mod apworld;
mod db;
mod diff;
mod guards;
mod schema;
mod tc;

use diff::{Annotations, FileDiff};

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

struct TcConfig {
    index_namespace_prefix: String,
}

struct TreeCache(Mutex<lru::LruCache<String, Arc<apworld::FileTree>>>);

#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
struct IndexPage {
    task_id: String,
    apworld_diffs: Vec<ApworldDiff>,
}

const BASE_MANUAL: &str = "manual_ultimatemarvelvscapcom3_manualteam";
const BASE_MANUAL_PREFIX: &str = "base:";

#[derive(Debug)]
struct FromVersion {
    label: String,
    value: String,
}

#[derive(Debug)]
struct ApworldDiff {
    apworld_name: String,
    world_name: String,
    from_versions: Vec<FromVersion>,
    selected_from: Option<String>,
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

#[derive(serde::Deserialize)]
struct TestResult {
    traceback: String,
    description: Option<String>,
}

#[derive(serde::Deserialize)]
struct UnexpectedSuccess {
    description: Option<String>,
}

#[derive(serde::Deserialize)]
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

fn deserialize_json<T: for<'de> serde::Deserialize<'de>>(text: &str) -> Result<T> {
    let mut deser = serde_json::Deserializer::from_str(text);
    Ok(serde_path_to_error::deserialize(&mut deser)?)
}

#[rocket::get("/<task_id>?<params..>")]
async fn get_task_diffs(
    task_id: &str,
    params: HashMap<String, String>,
    queue: &State<Queue>,
    index: &State<Index>,
    tc_config: &State<TcConfig>,
    tree_cache: &State<TreeCache>,
) -> Result<IndexPage> {
    let (artifacts, changes_text) = futures::try_join!(
        tc::get_task_artifacts(queue, task_id),
        tc::fetch_artifact_text(queue, task_id, "public/output/changes.json"),
    )?;
    let changes: Changes = deserialize_json(&changes_text)?;

    let apworld_diffs = try_join_all(
        changes
            .worlds
            .into_iter()
            .filter(|(_, wc)| {
                wc.added_versions
                    .iter()
                    .any(|v| !matches!(wc.checksums.get(v), Some(Checksum::Supported)))
            })
            .map(|(apworld_name, world_changes)| {
                let artifacts = &artifacts;
                let queue: &Queue = queue;
                let index: &Index = index;
                let prefix = &tc_config.index_namespace_prefix;
                let params = &params;
                let tree_cache: &TreeCache = tree_cache;
                async move {
                    let from_override = params.get(&format!("{apworld_name}_from"));
                    process_world(
                        queue,
                        index,
                        prefix,
                        task_id,
                        artifacts,
                        &apworld_name,
                        world_changes,
                        from_override.map(|s| s.as_str()),
                        tree_cache,
                    )
                    .await
                }
            }),
    )
    .await?;

    Ok(IndexPage {
        task_id: task_id.to_string(),
        apworld_diffs,
    })
}

async fn process_world(
    queue: &Queue,
    index: &Index,
    namespace_prefix: &str,
    task_id: &str,
    artifacts: &[String],
    apworld_name: &str,
    world_changes: apwm::changes::WorldChanges,
    from_override: Option<&str>,
    tree_cache: &TreeCache,
) -> Result<ApworldDiff> {
    let mut added_sorted = world_changes.added_versions.clone();
    added_sorted.retain(|v| !matches!(world_changes.checksums.get(v), Some(Checksum::Supported)));
    added_sorted.sort();

    let (indexed, to_trees) = futures::join!(
        async {
            tc::list_indexed_versions(index, namespace_prefix, apworld_name)
                .await
                .unwrap_or_default()
        },
        try_join_all(added_sorted.iter().map(|v| {
            let version = v.to_string();
            async move {
                let tree = cached_resolve_and_extract(
                    queue,
                    index,
                    namespace_prefix,
                    task_id,
                    artifacts,
                    apworld_name,
                    &version,
                    tree_cache,
                )
                .await?;
                let annotations =
                    fetch_annotations(queue, task_id, artifacts, apworld_name, &version).await?;
                Ok::<_, Error>((version, tree, annotations))
            }
        })),
    );
    let to_trees = to_trees?;

    let mut from_versions: Vec<FromVersion> = indexed
        .iter()
        .filter(|(v, _)| !world_changes.added_versions.contains(v))
        .map(|(v, _)| FromVersion {
            label: v.to_string(),
            value: v.to_string(),
        })
        .collect();

    let is_manual = apworld_name.starts_with("manual_") && apworld_name != BASE_MANUAL;

    let base_indexed = if is_manual {
        let base = tc::list_indexed_versions(index, namespace_prefix, BASE_MANUAL)
            .await
            .unwrap_or_default();
        for (v, _) in &base {
            from_versions.push(FromVersion {
                label: format!("base manual {v}"),
                value: format!("{BASE_MANUAL_PREFIX}{v}"),
            });
        }
        base
    } else {
        Vec::new()
    };

    let latest_added = added_sorted
        .last()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let selected_from = match from_override {
        Some("") => None,
        Some(v) => Some(v.to_string()),
        None => find_previous_version(&latest_added, &indexed).or_else(|| {
            base_indexed
                .last()
                .map(|(v, _)| format!("{BASE_MANUAL_PREFIX}{v}"))
        }),
    };

    let (selected_from, from_tree) = match &selected_from {
        Some(v) => {
            let is_base_manual = v.starts_with(BASE_MANUAL_PREFIX);
            let (resolve_name, resolve_version) =
                if let Some(base_v) = v.strip_prefix(BASE_MANUAL_PREFIX) {
                    (BASE_MANUAL, base_v)
                } else {
                    (apworld_name, v.as_str())
                };
            match cached_resolve_and_extract(
                queue,
                index,
                namespace_prefix,
                task_id,
                artifacts,
                resolve_name,
                resolve_version,
                tree_cache,
            )
            .await
            {
                Ok(tree) => {
                    let tree = if is_base_manual {
                        Arc::new(apworld::rekey_tree(&tree, apworld_name))
                    } else {
                        tree
                    };
                    (selected_from, Some(tree))
                }
                Err(e) => {
                    tracing::warn!(
                        "Error fetching from version {v} for {apworld_name}: {}",
                        e.0
                    );
                    (None, None)
                }
            }
        }
        None => (None, None),
    };

    let empty_tree = apworld::FileTree::new();
    let old_tree = from_tree.as_deref().unwrap_or(&empty_tree);

    let versions: Vec<VersionDiff> = to_trees
        .into_iter()
        .map(|(version, new_tree, annotations)| {
            let files = diff::compute::compute_file_tree_diff(old_tree, &new_tree, &annotations);
            let version_range = match &selected_from {
                Some(v) => format!("{v}...{version}"),
                None => format!("...{version}"),
            };
            VersionDiff {
                version_range,
                version_id: version,
                files,
            }
        })
        .collect();

    Ok(ApworldDiff {
        apworld_name: apworld_name.to_string(),
        world_name: world_changes.world_name,
        from_versions,
        selected_from,
        versions,
    })
}

fn find_previous_version(current: &str, indexed: &[(Version, String)]) -> Option<String> {
    let current_v = Version::parse(current).ok()?;
    indexed
        .iter()
        .filter(|(v, _)| v < &current_v)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(v, _)| v.to_string())
}

async fn cached_resolve_and_extract(
    queue: &Queue,
    index: &Index,
    namespace_prefix: &str,
    task_id: &str,
    artifacts: &[String],
    apworld_name: &str,
    version: &str,
    cache: &TreeCache,
) -> Result<Arc<apworld::FileTree>> {
    let key = format!("{apworld_name}:{version}");

    if let Some(tree) = cache.0.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(tree.clone());
    }

    let tree = Arc::new(
        resolve_and_extract(
            queue,
            index,
            namespace_prefix,
            task_id,
            artifacts,
            apworld_name,
            version,
        )
        .await?,
    );

    cache
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .put(key, tree.clone());
    Ok(tree)
}

async fn resolve_and_extract(
    queue: &Queue,
    index: &Index,
    namespace_prefix: &str,
    task_id: &str,
    artifacts: &[String],
    apworld_name: &str,
    version: &str,
) -> Result<apworld::FileTree> {
    let pr_artifact = format!("public/output/apworlds/{apworld_name}-{version}.apworld");

    let bytes = if artifacts.contains(&pr_artifact) {
        tc::fetch_artifact_bytes(queue, task_id, &pr_artifact).await?
    } else {
        let index_path = tc::index_path(namespace_prefix, apworld_name, version);
        let indexed_task_id = tc::find_indexed_task(index, &index_path)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Version {version} of {apworld_name} not found in index")
            })?;

        let artifact_name = format!("public/{apworld_name}-{version}.apworld");
        tc::fetch_artifact_bytes(queue, &indexed_task_id, &artifact_name).await?
    };

    Ok(apworld::extract_apworld(&bytes)?)
}

async fn fetch_annotations(
    queue: &Queue,
    task_id: &str,
    artifacts: &[String],
    apworld_name: &str,
    version: &str,
) -> Result<BTreeMap<String, Vec<Annotations>>> {
    let aplint_name = format!("public/output/{apworld_name}-{version}.aplint");

    if !artifacts.iter().any(|a| a == &aplint_name) {
        return Ok(BTreeMap::new());
    }

    let text = tc::fetch_artifact_text(queue, task_id, &aplint_name).await?;
    Ok(deserialize_json(&text)?)
}

#[rocket::get("/tests/<task_id>")]
async fn get_test_results(task_id: &str, queue: &State<Queue>) -> Result<TestPage> {
    let artifacts = tc::get_task_artifacts(queue, task_id).await?;
    let Some(aptest_name) = artifacts
        .iter()
        .find(|path| path.starts_with("public/test_results/"))
    else {
        Err(anyhow::anyhow!(
            "This doesn't look like a supported task, it contains no test_results"
        ))?
    };

    let aptest_text = tc::fetch_artifact_text(queue, task_id, aptest_name).await?;
    let results: TestResults = deserialize_json(&aptest_text)?;

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
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }
    env_logger::init();

    let mut client_builder = ClientBuilder::new(std::env::var("TASKCLUSTER_ROOT_URL")?);
    if let (Ok(client_id), Ok(access_token)) = (
        std::env::var("TASKCLUSTER_CLIENT_ID"),
        std::env::var("TASKCLUSTER_ACCESS_TOKEN"),
    ) {
        client_builder = client_builder.credentials(Credentials {
            client_id,
            access_token,
            certificate: None,
        });
    }
    let queue = Queue::new(client_builder.clone())?;
    let tc_index = Index::new(client_builder)?;

    let db_url = std::env::var("DATABASE_URL")?;
    let db_pool: Pool<AsyncPgConnection> =
        common::db::get_database_pool(&db_url, MIGRATIONS).await?;

    let fuzz_api_key = guards::FuzzApiKeyConfig(std::env::var("FUZZ_API_KEY")?);

    let tc_config = TcConfig {
        index_namespace_prefix: std::env::var("APWORLD_INDEX_NAMESPACE")
            .unwrap_or_else(|_| "ap.index.world".into()),
    };

    let tree_cache = TreeCache(Mutex::new(lru::LruCache::new(
        NonZeroUsize::new(32).unwrap(),
    )));

    rocket::build()
        .manage(queue)
        .manage(tc_index)
        .manage(tc_config)
        .manage(tree_cache)
        .manage(db_pool)
        .manage(fuzz_api_key)
        .mount("/", routes![get_task_diffs, dist_static, get_test_results])
        .mount("/api", api::routes())
        .launch()
        .await
        .map_err(|e| anyhow::anyhow!("Rocket launch failed: {}", e))?;

    Ok(())
}
