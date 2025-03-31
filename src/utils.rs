use rocket::{fs::NamedFile, http::Header};

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/zip")]
pub struct ZipFile<'a> {
    pub content: Vec<u8>,
    pub headers: Header<'a>,
}

#[derive(rocket::Responder)]
pub struct RenamedFile<'a> {
    pub inner: NamedFile,
    pub headers: Header<'a>,
}

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/octet-stream")]
pub struct NamedBuf<'a> {
    pub content: Vec<u8>,
    pub headers: Header<'a>,
}
