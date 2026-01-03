use std::str::FromStr;

use crate::config::DiscordConfig;
use crate::error::Result;
use crate::session::{is_banned, Session};
use crate::{Context, Discord};
use http::HeaderValue;
use reqwest::Url;
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket::{get, State};
use rocket_oauth2::{OAuth2, TokenResponse};
use tracing::Instrument;

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
struct DiscordMeResponse {
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
    config: &State<DiscordConfig>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    let token = token.access_token();

    let client = reqwest::Client::new();
    let user = get_discord_user(&client, token).await?;

    let client_id = config.client_id.clone();
    let client_secret = config.client_secret.clone();
    let token = token.to_owned();
    let span = tracing::Span::current();
    tokio::spawn(
        async move {
            let _ = revoke_token(&client, &client_id, &client_secret, &token).await;
        }
        .instrument(span),
    );

    let discord_id = user.id.parse()?;

    let mut conn = ctx.db_pool.get().await?;
    crate::db::upsert_discord_user(discord_id, &user.username, &mut conn).await?;

    let user_id = user.id.parse()?;
    session.user_id = Some(user_id);
    session.is_admin = config.admins.contains(&discord_id);
    session.is_logged_in = true;
    session.save(cookies).unwrap();

    // Don't redirect loop a banned user to a privileged page
    // Instead, redirect them to / which will log them out immediately
    if is_banned(user_id, config) {
        return Ok(Redirect::to("/"));
    }

    if let Some(redirect) = session.redirect_on_login {
        return Ok(Redirect::to(redirect));
    }

    Ok(Redirect::to("/"))
}

#[tracing::instrument(skip_all)]
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

#[tracing::instrument(skip_all)]
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

#[get("/logout")]
#[tracing::instrument(skip_all)]
fn logout(cookies: &CookieJar) -> Result<Redirect> {
    cookies.remove_private("session");

    Ok(Redirect::to("/"))
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![logout, login_discord, login_discord_callback]
}
