use crate::error::Result;
use askama::Template;
use rocket::form::Form;
use rocket::http::{CookieJar, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::{get, post, Request};
use uuid::Uuid;

use crate::TplContext;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub user_id: Uuid,
    pub is_admin: bool,
    pub err_msg: Option<String>,
}

pub struct AdminSession(pub Session);

impl Session {
    pub fn from_request_sync(request: &Request) -> Self {
        let cookies = request.cookies();
        if let Some(session) = cookies.get_private("session") {
            let session = serde_json::from_str::<Session>(session.value());
            if let Ok(session) = session {
                return session;
            }
        }

        let new_session = Session {
            is_admin: false,
            user_id: Uuid::new_v4(),
            err_msg: None,
        };

        let serialized = serde_json::to_string(&new_session).unwrap();
        cookies.add_private(("session", serialized));

        new_session
    }

    pub fn save(&self, cookies: &CookieJar) -> Result<()> {
        let serialized = serde_json::to_string(&self).unwrap();
        cookies.add_private(("session", serialized));

        Ok(())
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Session {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let new_session = Session::from_request_sync(request);
        Outcome::Success(new_session)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminSession {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let x_api_key = request.headers().get("X-Api-Key").next();
        if x_api_key == std::env::var("ADMIN_TOKEN").ok().as_deref() {
            return Outcome::Success(AdminSession(Session {
                user_id: Uuid::new_v4(),
                is_admin: true,
                err_msg: None,
            }));
        }
        let session = Session::from_request(request).await;
        let Outcome::Success(session) = session else {
            return Outcome::Error((
                Status::Unauthorized,
                crate::error::Error(anyhow::anyhow!("You need to be admin")),
            ));
        };

        if session.is_admin {
            return Outcome::Success(AdminSession(session));
        }

        Outcome::Error((
            Status::Unauthorized,
            crate::error::Error(anyhow::anyhow!("You need to be admin")),
        ))
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTpl<'a> {
    base: TplContext<'a>,
}

#[get("/login")]
fn login<'a>(session: Session, cookies: &CookieJar) -> LoginTpl<'a> {
    LoginTpl {
        base: TplContext::from_session("login", session, cookies),
    }
}

#[post("/login", data = "<token>")]
fn submit_login(token: Form<&str>, mut session: Session, cookies: &CookieJar) -> Result<Redirect> {
    if token.to_string() == std::env::var("ADMIN_TOKEN").unwrap() {
        session.is_admin = true;
        session.save(cookies)?;
    }

    Ok(Redirect::to("/"))
}

#[get("/logout")]
fn logout(mut session: Session, cookies: &CookieJar) -> Result<Redirect> {
    session.is_admin = false;
    session.save(cookies)?;

    Ok(Redirect::to("/"))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![login, submit_login, logout]
}
