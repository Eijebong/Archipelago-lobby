use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    io::BufReader,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use anyhow::{Context, Result};
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::AsyncPgConnection;
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wq::{JobDesc, JobId, JobResult, JobStatus, WorkQueue};

use crate::{
    db::{self, GenerationStatus, RoomId, YamlId, YamlValidationStatus, YamlWithoutContent},
    generation::{get_generation_info, get_slots},
};

#[derive(Serialize, Deserialize)]
pub struct YamlValidationParams {
    pub apworlds: Vec<(String, Version)>,
    pub yaml: String,
    pub otlp_context: HashMap<String, String>,
    pub yaml_id: Option<YamlId>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct YamlValidationResponse {
    pub error: Option<String>,
}

pub type YamlValidationQueue = WorkQueue<YamlValidationParams, YamlValidationResponse>;

#[derive(Serialize, Deserialize)]
pub struct GenerationParams {
    pub apworlds: Vec<(String, Version)>,
    pub room_id: RoomId,
    pub meta_file: String,
    pub otlp_context: HashMap<String, String>,
}

pub type GenerationQueue = WorkQueue<GenerationParams, ()>;
pub struct GenerationOutDir(pub PathBuf);

#[derive(Serialize, Deserialize)]
pub struct OptionsGenParams {
    pub apworld: (String, Version),
    pub otlp_context: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct OptionDef {
    pub default: Value,
    pub description: String,
    pub ty: String,
    pub display_name: String,
    #[serde(default)]
    pub range: Option<(i64, i64)>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub suggestions: Option<Vec<String>>,
    #[serde(default)]
    pub valid_keys: Option<Vec<String>>,
}

impl OptionDef {
    pub fn range_bounds(&self) -> (i64, i64) {
        self.range.expect("range_bounds called on non-range option")
    }

    pub fn choices(&self) -> &[String] {
        self.choices
            .as_ref()
            .expect("choices called on non-choice option")
    }

    pub fn suggestions(&self) -> &[String] {
        self.suggestions
            .as_ref()
            .expect("suggestions called on non-named_range/text_choice option")
    }

    pub fn valid_keys(&self) -> &[String] {
        self.valid_keys
            .as_ref()
            .expect("valid_keys called on non-set/list option")
    }

    pub fn default_str(&self) -> &str {
        self.default.as_str().unwrap_or_default()
    }

    pub fn default_contains(&self, value: &str) -> bool {
        self.default
            .as_array()
            .map(|arr| arr.iter().any(|v| v.as_str() == Some(value)))
            .unwrap_or(false)
    }

    pub fn default_json(&self) -> String {
        self.default.to_string()
    }
}
pub type OptionsDef = IndexMap<String, IndexMap<String, OptionDef>>;

#[derive(Serialize, Deserialize, Clone)]
pub struct OptionsGenResponse {
    // Option group -> (Option name, Option def)
    pub options: OptionsDef,
    pub error: Option<String>,
}

pub type OptionsGenQueue = WorkQueue<OptionsGenParams, OptionsGenResponse>;

pub fn get_yaml_validation_callback(
    db_pool: Pool<AsyncPgConnection>,
) -> wq::ResolveCallback<YamlValidationParams, YamlValidationResponse> {
    let callback = move |desc: JobDesc<YamlValidationParams>,
                         result: JobResult<YamlValidationResponse>|
          -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        let inner_pool = db_pool.clone();

        // Extract the OTLP context from the job params to link this callback to the original request
        let parent_cx = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&desc.params.otlp_context)
        });
        let span = tracing::info_span!(
            "yaml_validation_callback",
            yaml_id = ?desc.params.yaml_id,
            job_status = ?result.status,
        );
        let _ = span.set_parent(parent_cx);

        Box::pin(
            async move {
                // If the job doesn't specify a yaml ID we have nothing to do, it's a new insertion and
                // the caller is in charge of waiting for the result
                let Some(yaml_id) = desc.params.yaml_id else {
                    return Ok(false);
                };

                let mut conn = inner_pool.get().await?;

                let yaml = db::get_yaml_by_id(yaml_id, &mut conn).await;
                let yaml = match yaml {
                    Ok(yaml) => yaml,
                    Err(e) => {
                        if e.0.is::<diesel::result::Error>()
                            && e.0.downcast_ref::<diesel::result::Error>().unwrap()
                                == &diesel::result::Error::NotFound
                        {
                            info!(
                                "Received job result for a YAML that no longer exists, ignoring it"
                            );
                            return Ok(true);
                        }

                        return Err(e.0);
                    }
                };

                if yaml.last_validation_time.and_utc() > desc.submitted_at {
                    info!("Received a job older than the last validation, ignoring it");
                    return Ok(true);
                }

                let (status, error) = match result.status {
                    wq::JobStatus::Success => (YamlValidationStatus::Validated, None),
                    wq::JobStatus::Failure => (
                        YamlValidationStatus::Failed,
                        result.result.and_then(|r| r.error),
                    ),
                    wq::JobStatus::InternalError => (
                        YamlValidationStatus::Failed,
                        Some("Internal error".to_string()),
                    ),
                    _ => unreachable!(),
                };

                db::update_yaml_status(
                    yaml_id,
                    status,
                    error.clone(),
                    desc.params.apworlds,
                    desc.submitted_at,
                    &mut conn,
                )
                .await
                .map_err(|e| e.0)?;

                Ok(true)
            }
            .instrument(span),
        )
    };

    Arc::pin(callback)
}

pub fn get_generation_callback(
    db_pool: Pool<AsyncPgConnection>,
    generation_output_dir: PathBuf,
) -> wq::ResolveCallback<GenerationParams, ()> {
    let callback = move |desc: JobDesc<GenerationParams>,
                         result: JobResult<()>|
          -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        let inner_pool = db_pool.clone();
        let inner_generation_output_dir = generation_output_dir.clone();

        // Extract the OTLP context from the job params to link this callback to the original request
        let parent_cx = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&desc.params.otlp_context)
        });
        let span = tracing::info_span!(
            "generation_callback",
            room_id = %desc.params.room_id,
            job_status = ?result.status,
        );
        let _ = span.set_parent(parent_cx);

        Box::pin(
            async move {
                let mut conn = inner_pool.get().await?;

                let status = if result.status == JobStatus::Success {
                    GenerationStatus::Done
                } else {
                    GenerationStatus::Failed
                };

                db::update_generation_status_for_room(desc.params.room_id, status, &mut conn)
                    .await
                    .map_err(|e| e.0)?;

                if result.status == JobStatus::Success {
                    refresh_gen_patches(
                        desc.params.room_id,
                        &inner_generation_output_dir,
                        &mut conn,
                    )
                    .await?;
                }

                Ok(true)
            }
            .instrument(span),
        )
    };

    Arc::pin(callback)
}

static AP_PATCH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new("AP[_-][0-9]+[_-]P([0-9]+)[_-](.*)\\..*").unwrap());

#[tracing::instrument(skip(conn))]
pub async fn refresh_gen_patches(
    room_id: RoomId,
    generation_output_dir: &Path,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let gen = db::get_generation_for_room(room_id, conn)
        .await
        .map_err(|e| e.0)?
        .context("Couldn't find generation for room")?;
    let room_yamls = db::get_yamls_for_room_with_author_names(room_id, conn)
        .await
        .map_err(|e| e.0)?;
    let associations = get_yamls_patches_association(
        gen.job_id,
        generation_output_dir,
        room_yamls.into_iter().map(|(y, _)| y).collect(),
    )?;
    db::associate_patch_files(associations, room_id, conn)
        .await
        .map_err(|e| e.0)?;

    Ok(())
}

#[tracing::instrument(skip(room_yamls))]
pub fn get_yamls_patches_association(
    job_id: JobId,
    generation_output_dir: &Path,
    room_yamls: Vec<YamlWithoutContent>,
) -> Result<HashMap<YamlId, String>> {
    let generation_info = get_generation_info(job_id, generation_output_dir).map_err(|e| e.0)?;
    let gen_file = generation_output_dir.join(job_id.to_string()).join(
        generation_info
            .output_file
            .context("Couldn't find generation output")?,
    );
    let reader = BufReader::new(File::open(gen_file)?);
    let zip = zip::ZipArchive::new(reader)?;

    let room_yamls_with_resolved_names = get_slots(&room_yamls);

    let mut association = HashMap::new();
    for file_name in zip.file_names() {
        if let Some(patch) = AP_PATCH_RE.captures(file_name) {
            let slot_number: usize = patch[1].parse()?;
            let Some(associated_yaml) = room_yamls_with_resolved_names.get(slot_number - 1) else {
                continue;
            };
            association.insert(associated_yaml.1, file_name.to_string());
        }
    }

    Ok(association)
}
