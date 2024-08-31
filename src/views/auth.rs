use std::str::FromStr;

use crate::{Context, Discord};
use anyhow::anyhow;
use ap_lobby::error::Result;
use ap_lobby::session::Session;
use headers::HeaderValue;
use reqwest::Url;
use rocket::figment::{Figment, Profile, Provider};
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket::{get, State};
use rocket_oauth2::{OAuth2, TokenResponse};

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

    let mut conn = ctx.db_pool.get().await?;
    ap_lobby::db::upsert_discord_user(discord_id, &response.user.username, &mut conn).await?;
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
