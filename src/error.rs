use std::sync::OnceLock;

use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder};
use rocket::Request;

use crate::views::auth::Session;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(pub anyhow::Error);

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Error(error.into())
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

#[derive(Debug)]
pub struct RedirectTo(pub OnceLock<String>);

impl RedirectTo {
    pub fn set(&self, value: &str) {
        self.0
            .set(value.to_string())
            .expect("Failed to set value for RedirectTo");
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for &'r RedirectTo {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(request.local_cache(|| RedirectTo(OnceLock::new())))
    }
}

impl<'r> Responder<'r, 'static> for Error {
    fn respond_to(self, request: &Request<'_>) -> response::Result<'static> {
        let redirect = request.local_cache(|| {
            let lock = OnceLock::new();
            lock.set("/".to_string()).unwrap();
            RedirectTo(lock)
        });
        let error_message = self.0.to_string();

        let mut session = Session::from_request_sync(request);
        session.err_msg = Some(error_message);
        session.save(request.cookies()).unwrap();

        response::Redirect::to(redirect.0.get().unwrap().to_owned()).respond_to(request)
    }
}
