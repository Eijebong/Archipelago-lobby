use std::collections::HashMap;

use ap_lobby::jobs::{
    GenerationParams, GenerationResponse, YamlValidationParams, YamlValidationResponse,
};
use wq::{JobId, JobStatus};

#[derive(serde::Deserialize)]
pub struct ClaimJobForm {
    worker_id: String,
}

#[derive(serde::Deserialize)]
pub struct ReclaimJobForm {
    worker_id: String,
    job_id: JobId,
}

#[derive(serde::Deserialize)]
pub struct ResolveJobForm<R: Clone> {
    worker_id: String,
    job_id: JobId,
    status: JobStatus,
    result: R,
}

macro_rules! declare_queues {
    ($($mod_name:ident<$param_ty:ty, $resp_ty:ty>),*) => {
        $(pub mod $mod_name {
            use anyhow::anyhow;
            use ap_lobby::error::{ApiResult, ApiError};
            use rocket::State;
            use rocket::serde::json::Json;
            use rocket::{http::Status, request::{FromRequest, Outcome}, Request};
            use wq::{Job,WorkQueue};
            use super::*;

            struct QueueAuth;
            #[rocket::async_trait]
            impl<'r> FromRequest<'r> for QueueAuth {
                type Error = ap_lobby::error::ApiError;

                async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                    let Some(queue_auth) = request.rocket().state::<QueueTokens>() else {
                        return Outcome::Error((
                            Status::InternalServerError,
                            ApiError {
                                error: anyhow!("Internal error during authentication"),
                                status: Status::InternalServerError,
                            }
                        ))
                    };
                    let Some(expected_token) = queue_auth.0.get(stringify!($mod_name)) else {
                        return Outcome::Error((
                            Status::InternalServerError,
                            ApiError {
                                error: anyhow!("Internal error during authentication. That queue wasn't declared properly."),
                                status: Status::InternalServerError,
                            }
                        ))
                    };

                    let current_token = request.headers().get("X-Worker-Auth").next();

                    if current_token == Some(expected_token) {
                        return Outcome::Success(QueueAuth);
                    }

                    return Outcome::Error((
                        Status::Unauthorized,
                        ApiError {
                            status: Status::Unauthorized,
                            error: anyhow!("Invalid token passed as `X-Worker-Auth` or header missing")
                        }
                    ));
                }
            }

            #[rocket::post("/claim_job", data="<data>")]
            async fn claim_job(auth: ApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ClaimJobForm>) -> ApiResult<Json<Option<Job<$param_ty>>>> {
                auth?;

                Ok(Json(queue.claim_job(&data.worker_id).await?))
            }

            #[rocket::post("/reclaim_job", data="<data>")]
            async fn reclaim_job(auth: ApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ReclaimJobForm>) -> ApiResult<()> {
                auth?;

                queue.reclaim_job(&data.job_id, &data.worker_id).await?;
                Ok(())
            }

            #[rocket::post("/resolve_job", data="<data>")]
            #[tracing::instrument(skip_all)]
            async fn resolve_job(auth: ApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ResolveJobForm<$resp_ty>>) -> ApiResult<()> {
                // TODO: Attach this to the sent otlp context
                auth?;

                queue.resolve_job(&data.worker_id, data.job_id, data.status, data.result.clone()).await.unwrap();

                Ok(())
            }

            pub fn routes() -> Vec<rocket::Route> {
                rocket::routes![claim_job, reclaim_job, resolve_job]
            }
        })*

        pub fn routes() -> Vec<rocket::Route> {
            let mut routes = vec![];
            $(
                routes
                    .extend(
                        $mod_name::routes()
                            .into_iter()
                            .map(|route|
                                route
                                    .map_base(|base| format!("/{}{}", stringify!($mod_name), base))
                                    .unwrap()
                            )
                    );
            )*

            routes
        }
    };
}

pub struct QueueTokens<'a>(pub HashMap<&'a str, String>);

declare_queues!(
    yaml_validation<YamlValidationParams, YamlValidationResponse>,
    generation<GenerationParams, GenerationResponse>
);
