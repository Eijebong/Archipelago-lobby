use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc};

use anyhow::Result;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::AsyncPgConnection;
use semver::Version;
use serde::{Deserialize, Serialize};
use tracing::info;
use wq::{JobDesc, JobResult, JobStatus, WorkQueue};

use crate::db::{self, GenerationStatus, RoomId, YamlId, YamlValidationStatus};

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

#[derive(Serialize, Deserialize, Clone)]
pub struct GenerationResponse {
    pub error: Option<String>,
}

pub type GenerationQueue = WorkQueue<GenerationParams, GenerationResponse>;
pub struct GenerationOutDir(pub PathBuf);

pub fn get_yaml_validation_callback(
    db_pool: Pool<AsyncPgConnection>,
) -> wq::ResolveCallback<YamlValidationParams, YamlValidationResponse> {
    let callback = move |desc: JobDesc<YamlValidationParams>,
                         result: JobResult<YamlValidationResponse>|
          -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        let inner_pool = db_pool.clone();

        Box::pin(async move {
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
                        info!("Received job result for a YAML that no longer exists, ignoring it");
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
                wq::JobStatus::Failure => (YamlValidationStatus::Failed, result.result.error),
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
        })
    };

    Arc::pin(callback)
}

pub fn get_generation_callback(
    db_pool: Pool<AsyncPgConnection>,
) -> wq::ResolveCallback<GenerationParams, GenerationResponse> {
    let callback = move |desc: JobDesc<GenerationParams>,
                         result: JobResult<GenerationResponse>|
          -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        let inner_pool = db_pool.clone();
        Box::pin(async move {
            let mut conn = inner_pool.get().await?;

            let status = if result.status == JobStatus::Success {
                GenerationStatus::Done
            } else {
                GenerationStatus::Failed
            };

            db::update_generation_status_for_room(desc.params.room_id, status, &mut conn)
                .await
                .map_err(|e| e.0)?;

            Ok(true)
        })
    };

    Arc::pin(callback)
}
