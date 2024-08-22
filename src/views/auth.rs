use std::str::FromStr;

use crate::error::Result;
use crate::{AdminToken, Context, Discord};
use anyhow::anyhow;
use headers::authorization::{Basic, Credentials};
use headers::HeaderValue;
use reqwest::Url;
use rocket::figment::{Figment, Profile, Provider};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::time::ext::NumericalDuration;
use rocket::time::OffsetDateTime;
use rocket::{get, Request, State};
use rocket_oauth2::{OAuth2, TokenResponse};

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Session {
    pub is_admin: bool,
    pub is_logged_in: bool,
    pub err_msg: Vec<String>,
    pub warning_msg: Vec<String>,
    pub user_id: Option<i64>,
    pub redirect_on_login: Option<String>,
}

#[derive(serde::Deserialize, Default, Debug)]
pub struct SessionRecovery {
    pub is_admin: bool,
    pub is_logged_in: bool,
    pub user_id: Option<i64>,
}

impl From<SessionRecovery> for Session {
    fn from(val: SessionRecovery) -> Self {
        Session {
            is_admin: val.is_admin,
            is_logged_in: val.is_logged_in,
            err_msg: vec![],
            warning_msg: vec![],
            user_id: val.user_id,
            redirect_on_login: None,
        }
    }
}

pub struct LoggedInSession(pub Session);
pub struct AdminSession(());

impl LoggedInSession {
    pub fn user_id(&self) -> i64 {
        // Since we're taking from a logged in session, user_id can't be None here.
        self.0.user_id.unwrap()
    }
}

impl Session {
    #[tracing::instrument("parse_session", skip_all)]
    pub fn from_request_sync(request: &Request) -> Self {
        let admin_token = request.rocket().state::<AdminToken>();
        let admin_token = admin_token.map(|t| t.0.as_str());

        let authorization = request.headers().get_one("Authorization");
        if let Some(authorization) = authorization {
            let creds = Basic::decode(&HeaderValue::from_str(authorization).unwrap());
            if let Some(creds) = creds {
                if creds.username() == "admin" && Some(creds.password()) == admin_token {
                    tracing::info!("Admin logged with authorization header");
                    return Session {
                        is_admin: true,
                        is_logged_in: true,
                        ..Default::default()
                    };
                }
            }
        }

        let x_api_key = request.headers().get_one("X-Api-Key");
        if x_api_key == admin_token {
            tracing::info!("Admin logged with API key");
            return Session {
                is_admin: true,
                is_logged_in: true,
                ..Default::default()
            };
        }

        let cookies = request.cookies();
        if let Some(session_str) = cookies.get_private("session") {
            let session = serde_json::from_str::<Session>(session_str.value());
            if let Ok(session) = session {
                tracing::event!(
                    tracing::Level::INFO,
                    message = "Session already established",
                    session = session.user_id.map(|id| id.to_string())
                );
                return session;
            }
            let session_recovery = serde_json::from_str::<SessionRecovery>(session_str.value());
            if let Ok(session_recovery) = session_recovery {
                let session: Session = session_recovery.into();
                tracing::event!(
                    tracing::Level::INFO,
                    message = "Session recovered",
                    session = session.user_id.map(|id| id.to_string())
                );
                session.save(cookies).unwrap();
                return session;
            }
        }

        let new_session = Session::default();
        new_session.save(cookies).unwrap();

        new_session
    }

    pub fn save(&self, cookies: &CookieJar) -> Result<()> {
        let serialized = serde_json::to_string(&self).unwrap();

        let cookie = Cookie::build(("session", serialized))
            .expires(OffsetDateTime::now_utc() + 31.days())
            .same_site(SameSite::Lax)
            .build();

        cookies.add_private(cookie);

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
impl<'r> FromRequest<'r> for LoggedInSession {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let new_session = Session::from_request_sync(request);

        if new_session.is_admin {
            return Outcome::Success(LoggedInSession(new_session));
        }

        match new_session.user_id {
            Some(_) => Outcome::Success(LoggedInSession(new_session)),
            None => Outcome::Error((Status::new(401), anyhow!("Not logged in").into())),
        }
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminSession {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let session = Session::from_request(request).await;
        let Outcome::Success(session) = session else {
            return Outcome::Error((
                Status::Unauthorized,
                crate::error::Error(anyhow!("You need to be admin")),
            ));
        };

        if session.is_admin {
            return Outcome::Success(AdminSession(()));
        }

        Outcome::Error((
            Status::Unauthorized,
            crate::error::Error(anyhow!("You need to be admin")),
        ))
    }
}

#[get("/login?<redirect>")]
#[tracing::instrument(skip_all)]
fn login_discord(
    oauth2: OAuth2<Discord>,
    mut session: Session,
    redirect: Option<String>,
    cookies: &CookieJar,
) -> Result<Redirect> {
    if let Some(redirect) = redirect {
        if redirect.starts_with('/') {
            session.redirect_on_login = Some(redirect);
        }
    }

    session.save(cookies)?;

    Ok(oauth2.get_redirect(cookies, &["identify"])?)
}

#[derive(serde::Deserialize)]
struct DiscordMeRespone {
    pub user: DiscordUser,
}

#[derive(serde::Deserialize)]
struct DiscordUser {
    pub id: String,
    pub username: String,
}

#[get("/oauth")]
#[tracing::instrument(skip_all)]
async fn login_discord_callback(
    mut session: Session,
    token: TokenResponse<Discord>,
    cookies: &CookieJar<'_>,
    config: &State<Figment>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    let mut request = reqwest::Request::new(
        reqwest::Method::GET,
        Url::from_str("https://discord.com/api/oauth2/@me")?,
    );
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token.access_token()))?,
    );
    let response = reqwest::Client::new().execute(request).await?;
    let body = response.text().await?;
    let response = serde_json::from_str::<DiscordMeRespone>(&body)?;

    let discord_id = response.user.id.parse()?;
    crate::db::upsert_discord_user(discord_id, &response.user.username, ctx).await?;
    let config = config.data()?;
    let admins = config
        .get(&Profile::Default)
        .ok_or(anyhow!("No default profile in config"))?
        .get("oauth")
        .ok_or(anyhow!("No oauth section in default profile"))?
        .as_dict()
        .ok_or(anyhow!("oauth section isn't a map"))?
        .get("discord")
        .ok_or(anyhow!("no discord section in oauth"))?
        .as_dict()
        .ok_or(anyhow!("discord section isn't a dict"))?
        .get("admins")
        .ok_or(anyhow!("no admins in discord section"))?
        .as_array()
        .ok_or(anyhow!("admins isn't an array"))?;

    session.is_admin = admins.contains(&discord_id.into());
    session.user_id = Some(response.user.id.parse()?);
    session.is_logged_in = true;
    session.save(cookies).unwrap();

    if let Some(redirect) = session.redirect_on_login {
        return Ok(Redirect::to(redirect));
    }

    Ok(Redirect::to("/"))
}

#[get("/logout")]
#[tracing::instrument(skip_all)]
fn logout(cookies: &CookieJar) -> Result<Redirect> {
    let session = Session::default();
    session.save(cookies)?;

    Ok(Redirect::to("/"))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![logout, login_discord, login_discord_callback]
}
