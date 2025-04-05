use std::{borrow::Cow, ffi::OsStr, path::PathBuf, str::FromStr};

use anyhow::{Context, Result, anyhow};
use askama::Template;
use askama_web::WebTemplate;
use auth::{LoggedInSession, Session};
use guards::{ApRoom, DATA_PACKAGE, LobbyRoom, SlotInfo, SlotStatus};
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
use tungstenite::{Message, connect};
use uuid::Uuid;

mod auth;
mod error;
mod filters;
mod guards;

pub struct Discord;

#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
pub struct IndexTpl {
    lobby_room: LobbyRoom,
    ap_room: ApRoom,
    lobby_root_url: String,
    unclaimed_slots: Vec<SlotInfo>,
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
fn root(
    _session: LoggedInSession,
    lobby_room: LobbyRoom,
    mut ap_room: ApRoom,
    config: &State<Config>,
) -> crate::error::Result<IndexTpl> {
    if lobby_room.yamls.len() != ap_room.tracker_info.slots.len() {
        Err(anyhow!(
            "The AP room slot number doesn't match the lobby, this won't work"
        ))?;
    }

    ap_room
        .tracker_info
        .slots
        .sort_by_key(|slot| slot.status.clone());

    let unclaimed_slots = ap_room
        .tracker_info
        .slots
        .iter()
        .filter_map(|slot| {
            if slot.status != SlotStatus::Disconnected {
                return None;
            }

            if slot.checks.0 != 0 {
                return None;
            }

            return Some(slot);
        })
        .unique_by(|slot| &lobby_room.yamls.get(slot.id - 1).unwrap().discord_handle)
        .cloned()
        .collect();

    let index = IndexTpl {
        lobby_room,
        ap_room,
        lobby_root_url: config.lobby_root_url.to_string(),
        unclaimed_slots,
    };

    Ok(index)
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

async fn ap_cmd(cmd: String, config: &State<Config>) -> crate::error::Result<()> {
    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("cmd", cmd);

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_str(&config.ap_session_cookie)?,
    );
    let _ = client
        .post(
            Url::from_str(&format!(
                "https://archipelago.gg/room/{}",
                config.ap_room_id
            ))
            .unwrap(),
        )
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
) -> crate::error::Result<Redirect> {
    let url = format!("wss://archipelago.gg:{}", ap_room.room_status.last_port);
    let slot = ap_room
        .tracker_info
        .slots
        .iter()
        .find(|slot| slot.name == slot_name)
        .unwrap();
    let (mut socket, _) = connect(&url)?;
    let msg = format!(
        "[{{\"cmd\": \"Connect\", \"version\": {{ \"major\": 0, \"minor\": 5, \"build\": 1, \"class\": \"Version\"}}, \"items_handling\": 7, \"uuid\": \"\", \"tags\": [\"Admin\"], \"password\": null, \"game\": \"{}\", \"name\": \"{}\"}}, {{\"cmd\": \"StatusUpdate\", \"status\": 30}}]",
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

pub struct Config {
    pub lobby_root_url: Url,
    pub lobby_room_id: Uuid,
    pub lobby_api_key: String,
    pub ap_room_id: String,
    pub ap_session_cookie: String,
}

#[rocket::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv().ok();

    let lobby_root_url =
        std::env::var("LOBBY_ROOT_URL").expect("Provide a `LOBBY_ROOT_URL` env variable");
    let lobby_room_id =
        std::env::var("LOBBY_ROOM_ID").expect("Provide a `LOBBY_ROOM_ID` env variable");
    let lobby_api_key =
        std::env::var("LOBBY_API_KEY").expect("Provide a `LOBBY_API_KEY` env variable");
    let ap_room_id = std::env::var("AP_ROOM_ID").expect("Provide an `AP_ROOM_ID` env variable");
    let ap_session_cookie =
        std::env::var("AP_SESSION_COOKIE").expect("Provide an `AP_SESSION_COOKIE` env variable");

    let config = Config {
        lobby_root_url: lobby_root_url.parse()?,
        lobby_room_id: lobby_room_id.parse()?,
        lobby_api_key,
        ap_room_id,
        ap_session_cookie,
    };

    rocket::build()
        .mount(
            "/",
            routes![dist, root, release, hint, autocompletion, give],
        )
        .mount("/auth", auth::routes())
        .register("/", catchers![unauthorized])
        .manage(rocket::Config::figment())
        .manage(config)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await?;

    Ok(())
}
