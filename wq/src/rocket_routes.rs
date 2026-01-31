use std::collections::HashMap;
use std::io::Cursor;

use anyhow::anyhow;
use rocket::http::Status;
use rocket::response::{self, Responder};
use rocket::{Request, Response};

use crate::{JobId, JobStatus, WorkQueueError};

#[derive(Debug)]
pub struct QueueApiError(pub Status, pub anyhow::Error);

impl std::fmt::Display for QueueApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1)
    }
}

impl std::error::Error for QueueApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1.source()
    }
}

impl From<anyhow::Error> for QueueApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(Status::InternalServerError, error)
    }
}

impl From<WorkQueueError> for QueueApiError {
    fn from(err: WorkQueueError) -> Self {
        match err {
            WorkQueueError::JobCancelled => Self(Status::Gone, anyhow!("Job has been cancelled")),
            WorkQueueError::JobNotFound => Self(Status::NotFound, anyhow!("Job not found")),
            WorkQueueError::WorkerMismatch => {
                Self(Status::Forbidden, anyhow!("Worker does not own this job"))
            }
            WorkQueueError::InvalidJobStatus(msg) => {
                Self(Status::BadRequest, anyhow!("Invalid job status: {}", msg))
            }
            e => Self(Status::InternalServerError, e.into()),
        }
    }
}

impl<'r> Responder<'r, 'static> for QueueApiError {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let error = self.1.to_string();
        Response::build()
            .status(self.0)
            .sized_body(error.len(), Cursor::new(error))
            .ok()
    }
}

pub struct QueueTokens<'a>(pub HashMap<&'a str, String>);

#[derive(serde::Deserialize)]
pub struct ClaimJobForm {
    pub worker_id: String,
}

#[derive(serde::Deserialize)]
pub struct ReclaimJobForm {
    pub worker_id: String,
    pub job_id: JobId,
}

#[derive(serde::Deserialize)]
pub struct ResolveJobForm<R: Clone> {
    pub worker_id: String,
    pub job_id: JobId,
    pub status: JobStatus,
    pub result: Option<R>,
}

pub type QueueApiResult<T> = std::result::Result<T, QueueApiError>;

#[macro_export]
macro_rules! declare_queues {
    ($($mod_name:ident<$param_ty:ty, $resp_ty:ty>),*) => {
        $(pub mod $mod_name {
            use $crate::rocket_routes::{QueueApiError, QueueApiResult, QueueTokens, ClaimJobForm, ReclaimJobForm, ResolveJobForm};
            use rocket::State;
            use rocket::serde::json::Json;
            use rocket::{http::Status, request::{FromRequest, Outcome}, Request};
            use $crate::{Job, WorkQueue};
            use super::*;

            struct QueueAuth;
            #[rocket::async_trait]
            impl<'r> FromRequest<'r> for QueueAuth {
                type Error = QueueApiError;

                async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                    let Some(queue_auth) = request.rocket().state::<QueueTokens>() else {
                        return Outcome::Error((
                            Status::InternalServerError,
                            QueueApiError(
                                Status::InternalServerError,
                                anyhow::anyhow!("Internal error during authentication"),
                            )
                        ))
                    };
                    let Some(expected_token) = queue_auth.0.get(stringify!($mod_name)) else {
                        return Outcome::Error((
                            Status::InternalServerError,
                            QueueApiError(
                                Status::InternalServerError,
                                anyhow::anyhow!("Internal error during authentication. That queue wasn't declared properly."),
                            )
                        ))
                    };

                    let current_token = request.headers().get("X-Worker-Auth").next();

                    if current_token == Some(expected_token) {
                        return Outcome::Success(QueueAuth);
                    }

                    return Outcome::Error((
                        Status::Unauthorized,
                        QueueApiError(
                            Status::Unauthorized,
                            anyhow::anyhow!("Invalid token passed as `X-Worker-Auth` or header missing"),
                        )
                    ));
                }
            }

            #[rocket::post("/claim_job", data="<data>")]
            #[tracing::instrument(skip(auth, queue, data), fields(queue = stringify!($mod_name)))]
            async fn claim_job(auth: QueueApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ClaimJobForm>) -> QueueApiResult<Json<Option<Job<$param_ty>>>> {
                auth?;

                Ok(Json(queue.claim_job(&data.worker_id).await.map_err(QueueApiError::from)?))
            }

            #[rocket::post("/reclaim_job", data="<data>")]
            #[tracing::instrument(skip_all, fields(queue = stringify!($mod_name), job_id))]
            async fn reclaim_job(auth: QueueApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ReclaimJobForm>) -> QueueApiResult<()> {
                tracing::Span::current().record("job_id", tracing::field::display(&data.job_id));
                auth?;

                queue.reclaim_job(&data.job_id, &data.worker_id).await.map_err(QueueApiError::from)?;

                Ok(())
            }

            #[rocket::post("/resolve_job", data="<data>")]
            #[tracing::instrument(skip_all, fields(queue = stringify!($mod_name), job_id, job_status))]
            async fn resolve_job(auth: QueueApiResult<QueueAuth>, queue: &State<WorkQueue<$param_ty, $resp_ty>>, data: Json<ResolveJobForm<$resp_ty>>) -> QueueApiResult<()> {
                tracing::Span::current().record("job_id", tracing::field::display(&data.job_id));
                tracing::Span::current().record("job_status", tracing::field::debug(&data.status));
                auth?;

                queue.resolve_job(&data.worker_id, data.job_id, data.status, data.result.clone()).await.map_err(QueueApiError::from)?;
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
