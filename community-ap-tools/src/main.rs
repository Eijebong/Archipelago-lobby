use std::{borrow::Cow, ffi::OsStr, path::PathBuf, str::FromStr};

use anyhow::{Context, anyhow};
use askama::Template;
use askama_web::WebTemplate;
use auth::{LoggedInSession, Session};
use guards::{ApRoom, DATA_PACKAGE, LobbyRoom, SlotInfo, SlotPasswords, SlotStatus};
use itertools::Itertools;
use reqwest::{
    Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use rocket::catchers;
use rocket::{
    Request, State, catch, http::ContentType, response::Redirect, routes, serde::json::Json,
};
use rocket_oauth2::OAuth2;
use rustls::crypto::ring;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tungstenite::{Message, connect};
use uuid::Uuid;

mod auth;
mod datapackage;
mod error;
mod filters;
mod guards;

pub struct Discord;

#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
pub struct RunIndexTpl {
    lobby_room: LobbyRoom,
    ap_room: ApRoom,
    lobby_root_url: String,
    is_session_valid: bool,
    unclaimed_slots: Vec<SlotInfo>,
    slot_passwords: SlotPasswords,
}

#[derive(rust_embed::RustEmbed)]
#[folder = "./static/"]
struct Assets;

#[rocket::get("/static/<file..>")]
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let filename = file.display().to_string();
    let asset = Assets::get(&filename)?;
    let content_type = file
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

#[catch(401)]
fn unauthorized(req: &Request) -> crate::error::Result<Redirect> {
    let session = Session::from_request_sync(req);
    if session.is_logged_in {
        Err(anyhow::anyhow!("You're not allowed here"))?
    }

    Ok(Redirect::to(format!(
        "/auth/login?redirect={}",
        req.uri().path()
    )))
}

#[rocket::get("/")]
async fn root_run(
    _session: LoggedInSession,
    lobby_room: LobbyRoom,
    mut ap_room: ApRoom,
    slot_passwords: SlotPasswords,
    config: &State<Config>,
) -> crate::error::Result<RunIndexTpl> {
    if lobby_room.yamls.len() != ap_room.tracker_info.slots.len() {
        Err(anyhow!(
            "The AP room slot number doesn't match the lobby, this won't work"
        ))?;
    }

    ap_room
        .tracker_info
        .slots
        .sort_by_key(|slot| slot.status.clone());

    ap_room.tracker_info.slots.sort_by_key(|slot| {
        match filters::slot_status(slot, &()).unwrap_or("green") {
            "green" => 2,
            "yellow" => 1,
            "red" => 0,
            _ => 0,
        }
    });

    let unclaimed_slots = ap_room
        .tracker_info
        .slots
        .iter()
        .filter(|slot| {
            if slot.status != SlotStatus::Disconnected {
                return false;
            }

            if slot.checks.0 != 0 {
                return false;
            }

            true
        })
        .unique_by(|slot| &lobby_room.yamls.get(slot.id - 1).unwrap().discord_handle)
        .cloned()
        .collect();

    let index = RunIndexTpl {
        lobby_room,
        ap_room,
        lobby_root_url: config.lobby_root_url.to_string(),
        unclaimed_slots,
        is_session_valid: config.is_session_valid,
        slot_passwords,
    };

    Ok(index)
}

#[rocket::get("/")]
async fn root(
    session: LoggedInSession,
    lobby_room: LobbyRoom,
    ap_room: ApRoom,
    slot_passwords: SlotPasswords,
    config: &State<Config>,
) -> crate::error::Result<RunIndexTpl> {
    root_run(session, lobby_room, ap_room, slot_passwords, config).await
}

#[rocket::get("/hint/<ty>/<slot_name>/<item_name>")]
async fn hint(
    _session: LoggedInSession,
    ty: &str,
    slot_name: &str,
    item_name: &str,
    config: &State<Config>,
) -> crate::error::Result<Redirect> {
    if !["item", "location"].contains(&ty) {
        Err(anyhow::anyhow!(
            "Wrong hint type. Only item/location are supported"
        ))?;
    }

    let cmd = if ty == "item" {
        "/hint"
    } else {
        "/hint_location"
    };

    let cmd = format!(
        "{} {} {}",
        cmd,
        shlex::try_quote(slot_name)?,
        shlex::try_quote(item_name)?
    );

    ap_cmd(cmd, config).await?;

    Ok(Redirect::to("/"))
}

#[rocket::get("/give/<ty>/<slot_name>/<item_name>")]
async fn give(
    _session: LoggedInSession,
    ty: &str,
    slot_name: &str,
    item_name: &str,
    config: &State<Config>,
) -> crate::error::Result<Redirect> {
    if !["item", "location"].contains(&ty) {
        Err(anyhow::anyhow!(
            "Wrong give type. Only item/location are supported"
        ))?;
    }

    let cmd = if ty == "item" {
        "/send"
    } else {
        "/send_location"
    };

    let cmd = format!(
        "{} {} {}",
        cmd,
        shlex::try_quote(slot_name)?,
        shlex::try_quote(item_name)?
    );

    ap_cmd(cmd, config).await?;

    Ok(Redirect::to("/"))
}

/// Check that the currently provided session cookie is valid by checking for the presence of the
/// `#cmd` element on the page.
async fn check_session(config: &Config) -> crate::error::Result<bool> {
    eprintln!(
        "[STARTUP] Checking session validity at: {}",
        config.ap_room_url
    );
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_str(&config.ap_session_cookie)?,
    );
    let res = client
        .get(config.ap_room_url.clone())
        .headers(headers)
        .send()
        .await;

    let res = match res {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[STARTUP] Failed to fetch session check URL: {}", e);
            return Err(e.into());
        }
    };

    eprintln!("[STARTUP] Session check response status: {}", res.status());

    let body = res.text().await?;

    let html = Html::parse_document(&body);
    let cmd_selector = Selector::parse("#cmd").unwrap();
    let cmd_input = html.select(&cmd_selector);

    let is_valid = cmd_input.count() == 1;
    eprintln!("[STARTUP] Session is valid: {}", is_valid);

    Ok(is_valid)
}

async fn ap_cmd(cmd: String, config: &State<Config>) -> crate::error::Result<()> {
    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("cmd", cmd);

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_str(&config.ap_session_cookie)?,
    );

    // There's no point in looking at the response here. AP doesn't have a proper API for rooms
    // since sending a command just inserts something in database that gets polled by the room
    // process later on so they don't provide responses. If anything fails, it just ignores the
    // input and nothing happens...
    let _ = client
        .post(config.ap_room_url.clone())
        .multipart(form)
        .headers(headers)
        .send()
        .await?;

    Ok(())
}

#[rocket::get("/release/<slot_name>")]
async fn release(
    _session: LoggedInSession,
    ap_room: ApRoom,
    slot_name: &str,
    config: &State<Config>,
) -> crate::error::Result<Redirect> {
    let url = format!("wss://{}:{}", config.ap_room_host, config.ap_room_port);
    let slot = ap_room
        .tracker_info
        .slots
        .iter()
        .find(|slot| slot.name == slot_name)
        .unwrap();
    let (mut socket, _) = connect(&url)?;
    let msg = format!(
        "[{{\"cmd\": \"Connect\", \"version\": {{ \"major\": 9000, \"minor\": 0, \"build\": 1, \"class\": \"Version\"}}, \"items_handling\": 7, \"uuid\": \"\", \"tags\": [\"Admin\"], \"password\": null, \"game\": \"{}\", \"name\": \"{}\"}}, {{\"cmd\": \"StatusUpdate\", \"status\": 30}}]",
        slot.game, slot_name
    );
    socket.send(Message::Text(msg.into()))?;
    socket.flush()?;
    socket.close(None)?;

    Ok(Redirect::to("/"))
}

#[rocket::get("/completion/<ty>/<game_name>")]
async fn autocompletion(
    _session: LoggedInSession,
    ty: &str,
    game_name: &str,
) -> crate::error::Result<Json<Vec<String>>> {
    let datapackage = DATA_PACKAGE.get().context("No datapackage loaded")?;
    let game = datapackage
        .data
        .games
        .get(game_name)
        .context("Couldn't find game")?;
    let names = if ty == "item" {
        game.game_data.item_name_to_id.keys().cloned().collect()
    } else {
        game.game_data.location_name_to_id.keys().cloned().collect()
    };

    Ok(Json(names))
}

#[derive(Deserialize, Serialize)]
struct SetPasswordRequest {
    password: Option<String>,
}

#[rocket::post("/set_password/<yaml_id>", data = "<request>")]
async fn set_password(
    _session: LoggedInSession,
    yaml_id: &str,
    request: Json<SetPasswordRequest>,
    config: &State<Config>,
) -> crate::error::Result<()> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(&config.lobby_api_key)?,
    );

    let url = config.lobby_root_url.join(&format!(
        "/api/room/{}/set_password/{}",
        config.lobby_room_id, yaml_id
    ))?;

    let response = client
        .post(url)
        .headers(headers)
        .json(&request.into_inner())
        .send()
        .await?;

    if !response.status().is_success() {
        Err(anyhow!("Failed to set password: {}", response.status()))?;
    }

    if let (Some(apx_root), Some(apx_key)) = (&config.apx_api_root, &config.apx_api_key) {
        let apx_url = apx_root.join("/refresh_passwords")?;
        let client = reqwest::Client::new();

        let result = client
            .post(apx_url)
            .header(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_str(apx_key)?,
            )
            .send()
            .await;

        if let Err(e) = result {
            eprintln!("Failed to notify APX API about password change: {}", e);
        }
    }

    Ok(())
}

pub struct Config {
    pub lobby_root_url: Url,
    pub lobby_room_id: Uuid,
    pub lobby_api_key: String,
    pub ap_room_id: String,
    pub ap_room_url: Url,
    pub ap_api_root: Url,
    pub ap_room_host: String,
    pub ap_room_port: u16,
    pub ap_session_cookie: String,
    pub is_session_valid: bool,
    pub apx_api_root: Option<Url>,
    pub apx_api_key: Option<String>,
}

#[rocket::main]
async fn main() -> crate::error::Result<()> {
    let _ = dotenvy::dotenv().ok();

    let lobby_root_url =
        std::env::var("LOBBY_ROOT_URL").expect("Provide a `LOBBY_ROOT_URL` env variable");
    let lobby_room_id =
        std::env::var("LOBBY_ROOM_ID").expect("Provide a `LOBBY_ROOM_ID` env variable");
    let lobby_api_key =
        std::env::var("LOBBY_API_KEY").expect("Provide a `LOBBY_API_KEY` env variable");
    let ap_room_id = std::env::var("AP_ROOM_ID").expect("Provide an `AP_ROOM_ID` env variable");
    let ap_room_host =
        std::env::var("AP_ROOM_HOST").expect("Provide an `AP_ROOM_HOST` env variable");
    let ap_room_port = std::env::var("AP_ROOM_PORT")
        .expect("Provide an `AP_ROOM_PORT` env variable")
        .parse::<u16>()
        .expect("AP_ROOM_PORT must be a valid port number");
    let ap_session_cookie =
        std::env::var("AP_SESSION_COOKIE").expect("Provide an `AP_SESSION_COOKIE` env variable");

    let ap_api_root = std::env::var("AP_API_ROOT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| Url::from_str(&format!("https://{}", ap_room_host)).unwrap());

    eprintln!("[STARTUP] AP_API_ROOT: {}", ap_api_root);
    eprintln!("[STARTUP] AP_ROOM_HOST: {}", ap_room_host);
    eprintln!("[STARTUP] AP_ROOM_PORT: {}", ap_room_port);
    eprintln!("[STARTUP] AP_ROOM_ID: {}", ap_room_id);

    let apx_api_root = std::env::var("APX_API_ROOT")
        .ok()
        .and_then(|s| s.parse().ok());
    let apx_api_key = std::env::var("APX_API_KEY").ok();

    ring::default_provider()
        .install_default()
        .expect("Failed to set ring as crypto provider");

    let ap_room_url = ap_api_root.join(&format!("/room/{}", ap_room_id))?;
    eprintln!("[STARTUP] Constructed AP_ROOM_URL: {}", ap_room_url);

    let mut config = Config {
        lobby_root_url: lobby_root_url.parse()?,
        lobby_room_id: lobby_room_id.parse()?,
        lobby_api_key,
        ap_session_cookie,
        ap_room_url,
        ap_api_root,
        ap_room_host,
        ap_room_port,
        ap_room_id,
        is_session_valid: false,
        apx_api_root,
        apx_api_key,
    };

    config.is_session_valid = check_session(&config).await?;

    rocket::build()
        .mount(
            "/",
            routes![
                dist,
                root,
                release,
                hint,
                autocompletion,
                give,
                set_password
            ],
        )
        .mount("/auth", auth::routes())
        .register("/", catchers![unauthorized])
        .manage(rocket::Config::figment())
        .manage(config)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await
        .unwrap();

    Ok(())
}
