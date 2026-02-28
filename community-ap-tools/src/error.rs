use std::io::Cursor;

use rocket::{
    Request, Response,
    http::Status,
    response::{self, Responder},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error {
    pub error: anyhow::Error,
    pub status: Status,
}

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Error {
            error: error.into(),
            status: Status::InternalServerError,
        }
    }
}

pub fn forbidden(msg: impl Into<String>) -> Error {
    Error {
        error: anyhow::anyhow!("{}", msg.into()),
        status: Status::Forbidden,
    }
}

pub fn bad_request(msg: impl Into<String>) -> Error {
    Error {
        error: anyhow::anyhow!("{}", msg.into()),
        status: Status::BadRequest,
    }
}

pub fn not_found(msg: impl Into<String>) -> Error {
    Error {
        error: anyhow::anyhow!("{}", msg.into()),
        status: Status::NotFound,
    }
}

impl Responder<'_, 'static> for Error {
    fn respond_to(self, req: &Request<'_>) -> response::Result<'static> {
        let error = self.error.to_string();
        if self.status == Status::InternalServerError {
            eprintln!("[ERROR] {} {}: {}", req.method(), req.uri(), error);
            eprintln!("[ERROR] Backtrace: {:?}", self.error);
        }
        Response::build()
            .status(self.status)
            .sized_body(error.len(), Cursor::new(error))
            .ok()
    }
}
