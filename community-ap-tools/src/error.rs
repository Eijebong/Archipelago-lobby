use std::io::Cursor;

use rocket::{
    Request, Response,
    http::Status,
    response::{self, Responder},
};

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

impl Responder<'_, 'static> for Error {
    fn respond_to(self, req: &Request<'_>) -> response::Result<'static> {
        let error = self.0.to_string();
        eprintln!("[ERROR] {} {}: {}", req.method(), req.uri(), error);
        eprintln!("[ERROR] Backtrace: {:?}", self.0);
        Response::build()
            .status(Status::InternalServerError)
            .sized_body(error.len(), Cursor::new(error))
            .ok()
    }
}
