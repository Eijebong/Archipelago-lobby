use std::io::Cursor;
use std::sync::OnceLock;

use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder};
use rocket::{Request, Response};

use crate::session::Session;
use crate::Context;

pub type Result<T> = std::result::Result<T, Error>;
pub type ApiResult<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
pub struct Error(pub anyhow::Error);

#[derive(Debug)]
pub struct ApiError {
    pub error: anyhow::Error,
    pub status: Status,
}

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Error(error.into())
    }
}

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        Self {
            error: error.0,
            status: Status::InternalServerError,
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self {
            error: error.into(),
            status: Status::InternalServerError,
        }
    }
}

pub trait WithStatus<T> {
    fn status(self, status: Status) -> ApiResult<T>;
}

impl<T> WithStatus<T> for Result<T> {
    fn status(self, status: Status) -> ApiResult<T> {
        self.map_err(|error| ApiError {
            error: error.0,
            status,
        })
    }
}

pub trait WithContext<T> {
    fn context(self, context: &'static str) -> Self;
}

impl<T> WithContext<T> for Result<T> {
    fn context(self, context: &'static str) -> Self {
        Ok(anyhow::Context::context(self.map_err(|s| s.0), context)?)
    }
}

impl<T> WithContext<T> for ApiResult<T> {
    fn context(self, context: &'static str) -> Self {
        Ok(anyhow::Context::context(
            self.map_err(|s| s.error),
            context,
        )?)
    }
}

#[derive(Debug)]
pub struct RedirectTo(pub OnceLock<String>);

impl RedirectTo {
    pub fn set(&self, value: &str) {
        let _ = self.0.set(value.to_string());
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for &'r RedirectTo {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(request.local_cache(|| RedirectTo(OnceLock::new())))
    }
}

impl Responder<'_, 'static> for Error {
    fn respond_to(self, request: &Request<'_>) -> response::Result<'static> {
        let ctx = request.rocket().state::<Context>().unwrap();
        let redirect = request.local_cache(|| {
            let lock = OnceLock::new();
            lock.set("/".to_string()).unwrap();
            RedirectTo(lock)
        });
        let error_message = self.0.to_string();

        let session = Session::from_request_sync(request);
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(session.push_error(&error_message, ctx))
                .expect("Failed to record error");
        });

        response::Redirect::to(redirect.0.get().unwrap().to_owned()).respond_to(request)
    }
}

impl Responder<'_, 'static> for ApiError {
    fn respond_to(self, _: &Request<'_>) -> response::Result<'static> {
        let error = self.error.to_string();
        Response::build()
            .status(self.status)
            .sized_body(error.len(), Cursor::new(error))
            .ok()
    }
}
