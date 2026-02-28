use std::str::FromStr;

use crate::{Discord, error, error::Result, review::Role, review::db as review_db};
use anyhow::anyhow;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use reqwest::{Url, header::HeaderValue};
use rocket::figment::{Figment, Profile, Provider, value::Dict};
use rocket::time::ext::NumericalDuration;
use rocket::{
    Request, State, get,
    http::{Cookie, CookieJar, SameSite, Status},
    request::{FromRequest, Outcome},
    response::Redirect,
    routes,
    time::OffsetDateTime,
};
use rocket_oauth2::{OAuth2, TokenResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub is_logged_in: bool,
    pub redirect_on_login: Option<String>,
    #[serde(default)]
    pub is_super_admin: bool,
}

impl Session {
    pub fn from_request_sync(request: &Request<'_>) -> Self {
        let cookies = request.cookies();
        // The cookie is named differently than the lobby because of the Lax website origin
        // Otherwise it clashes with the lobby and it's annoying when deving
        if let Some(session_str) = cookies.get_private("apsession") {
            let Ok(session) = serde_json::from_str::<Session>(session_str.value()) else {
                cookies.remove_private("apsession");
                return Session {
                    user_id: None,
                    username: None,
                    is_logged_in: false,
                    redirect_on_login: None,
                    is_super_admin: false,
                };
            };

            return session;
        }

        Session {
            user_id: None,
            username: None,
            is_logged_in: false,
            redirect_on_login: None,
            is_super_admin: false,
        }
    }

    pub fn save(&self, cookies: &CookieJar) -> Result<()> {
        let serialized = serde_json::to_string(&self)?;

        let cookie = Cookie::build(("apsession", serialized))
            .expires(OffsetDateTime::now_utc() + 31.days())
            .same_site(SameSite::Lax)
            .build();

        cookies.add_private(cookie);

        Ok(())
    }
}

pub struct LoggedInSession(Session);

impl LoggedInSession {
    pub fn user_id(&self) -> i64 {
        self.0.user_id.expect("LoggedInSession must have a user_id")
    }

    pub fn username(&self) -> &str {
        self.0.username.as_deref().unwrap_or("Unknown")
    }

    pub fn is_super_admin(&self) -> bool {
        self.0.is_super_admin
    }

    pub async fn require_room_role(
        &self,
        room_id: Uuid,
        minimum: Role,
        conn: &mut AsyncPgConnection,
    ) -> Result<()> {
        if self.is_super_admin() {
            return Ok(());
        }
        let role = review_db::get_user_role_for_room(self.user_id(), room_id, conn).await?;
        match role {
            Some(r) if r >= minimum => Ok(()),
            _ => Err(error::forbidden("Forbidden")),
        }
    }

    pub async fn require_team_role(
        &self,
        team_id: i32,
        minimum: Role,
        conn: &mut AsyncPgConnection,
    ) -> Result<()> {
        if self.is_super_admin() {
            return Ok(());
        }
        let role = review_db::get_user_role_for_team(self.user_id(), team_id, conn).await?;
        match role {
            Some(r) if r >= minimum => Ok(()),
            _ => Err(error::forbidden("Forbidden")),
        }
    }

    pub async fn require_preset_role(
        &self,
        preset_id: i32,
        minimum: Role,
        conn: &mut AsyncPgConnection,
    ) -> Result<()> {
        if self.is_super_admin() {
            return Ok(());
        }
        let role = review_db::get_user_role_for_preset(self.user_id(), preset_id, conn).await?;
        match role {
            Some(r) if r >= minimum => Ok(()),
            _ => Err(error::forbidden("Forbidden")),
        }
    }
}

pub struct AdminSession(#[allow(dead_code)] LoggedInSession);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Session {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Session::from_request_sync(request))
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LoggedInSession {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let session = Session::from_request_sync(request);

        if session.is_logged_in {
            return Outcome::Success(LoggedInSession(session));
        }

        Outcome::Error((Status::Unauthorized, anyhow!("Not logged in").into()))
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminSession {
    type Error = crate::error::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let session = Session::from_request_sync(request);

        if !session.is_logged_in {
            return Outcome::Error((Status::Unauthorized, anyhow!("Not logged in").into()));
        }

        if !session.is_super_admin {
            return Outcome::Error((Status::Forbidden, anyhow!("Forbidden").into()));
        }

        Outcome::Success(AdminSession(LoggedInSession(session)))
    }
}

pub fn get_discord_config(figment: &Figment) -> anyhow::Result<Dict> {
    let config = figment.data()?;
    let discord_config = config
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
        .clone();
    Ok(discord_config)
}

pub fn is_super_admin(user_id: i64, discord_config: &Dict) -> bool {
    let Some(admins) = discord_config.get("admins").and_then(|v| v.as_array()) else {
        return false;
    };
    admins.contains(&user_id.into())
}

#[get("/login?<redirect>")]
fn login(
    oauth2: OAuth2<Discord>,
    mut session: Session,
    redirect: Option<String>,
    cookies: &CookieJar,
) -> Result<Redirect> {
    if let Some(redirect) = redirect
        && redirect.starts_with('/')
    {
        session.redirect_on_login = Some(redirect);
    }

    session.save(cookies)?;

    Ok(oauth2.get_redirect(cookies, &["identify"])?)
}

#[get("/logout")]
fn logout(cookies: &CookieJar<'_>) -> Redirect {
    cookies.remove_private("apsession");

    Redirect::to("/")
}

#[get("/oauth")]
async fn login_discord_callback(
    mut session: Session,
    token: TokenResponse<Discord>,
    cookies: &CookieJar<'_>,
    figment: &State<Figment>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> Result<Redirect> {
    let token = token.access_token();

    let client = reqwest::Client::new();
    let user = get_discord_user(&client, token).await?;

    let discord_config = get_discord_config(figment)?;
    let client_id = discord_config
        .get("client_id")
        .ok_or(anyhow!("client id not present in discord config"))?
        .as_str()
        .ok_or(anyhow!("client id isn't a string"))?;
    let client_secret = discord_config
        .get("client_secret")
        .ok_or(anyhow!("client secret not present in discord config"))?
        .as_str()
        .ok_or(anyhow!("client secret isn't a string"))?;
    revoke_token(&client, client_id, client_secret, token).await?;

    let discord_id: i64 = user.id.parse()?;
    let super_admin = is_super_admin(discord_id, &discord_config);

    if !super_admin {
        let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
        let in_team = review_db::is_user_in_any_team(discord_id, &mut conn).await?;
        if !in_team {
            Err(anyhow::anyhow!("Not allowed"))?
        }
    }

    session.user_id = Some(user.id.parse()?);
    session.username = Some(user.username.clone());
    session.is_logged_in = true;
    session.is_super_admin = super_admin;
    session.save(cookies).unwrap();

    if let Some(redirect) = session.redirect_on_login {
        return Ok(Redirect::to(redirect));
    }

    Ok(Redirect::to("/"))
}

async fn revoke_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    token: &str,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct RevokeForm<'a> {
        token: &'a str,
    }

    let _ = client
        .post("https://discord.com/api/oauth2/token/revoke")
        .basic_auth(client_id, Some(client_secret))
        .form(&RevokeForm { token })
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

#[derive(serde::Deserialize)]
struct DiscordMeResponse {
    pub user: DiscordUser,
}

#[derive(serde::Deserialize)]
struct DiscordUser {
    pub id: String,
    pub username: String,
}

async fn get_discord_user(client: &reqwest::Client, token: &str) -> Result<DiscordUser> {
    let mut request = reqwest::Request::new(
        reqwest::Method::GET,
        Url::from_str("https://discord.com/api/oauth2/@me")?,
    );
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    let response = client.execute(request).await?;
    let body = response.text().await?;
    let response = serde_json::from_str::<DiscordMeResponse>(&body)?;

    Ok(response.user)
}

pub fn routes() -> Vec<rocket::Route> {
    routes![login, logout, login_discord_callback]
}
