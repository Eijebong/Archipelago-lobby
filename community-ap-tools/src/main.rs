use std::{borrow::Cow, collections::HashMap, ffi::OsStr, path::PathBuf, str::FromStr};

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
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tungstenite::{Message, connect};
use uuid::Uuid;

mod auth;
mod datapackage;
mod error;
mod filters;
mod guards;
mod review;
mod schema;

use diesel_migrations::{EmbeddedMigrations, embed_migrations};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/");

pub const STATIC_VERSION: &str = std::env!("STATIC_VERSION");

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

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Deathlink {
    pub slot: usize,
    pub source: String,
    pub cause: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct SlotDeathCount {
    pub slot: usize,
    pub name: String,
    pub count: usize,
}

#[derive(Deserialize)]
struct ExclusionsResponse {
    excluded_slots: Vec<usize>,
}

#[derive(Deserialize, Serialize)]
struct ProbabilityResponse {
    probability: f64,
}

#[derive(Deserialize, Serialize)]
struct SetProbabilityRequest {
    probability: f64,
}

pub struct DeathlinksSlot {
    pub id: usize,
    pub name: String,
    pub game: String,
    pub discord_handle: String,
    pub is_excluded: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "deathlinks.html")]
pub struct DeathlinksIndexTpl {
    lobby_room: LobbyRoom,
    lobby_root_url: String,
    is_session_valid: bool,
    slots: Vec<DeathlinksSlot>,
    deathlinks: Vec<Deathlink>,
    deaths_by_slot: Vec<SlotDeathCount>,
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

    ap_room.tracker_info.slots.sort_by(|a, b| {
        // by slot_status_fn color
        let color_a = match filters::slot_status_fn(a).unwrap_or("green") {
            "green" => 2,
            "yellow" => 1,
            _ => 0,
        };
        let color_b = match filters::slot_status_fn(b).unwrap_or("green") {
            "green" => 2,
            "yellow" => 1,
            _ => 0,
        };

        color_a
            .cmp(&color_b)
            // by status
            .then_with(|| a.status.cmp(&b.status))
            // by checks (0 checks first, only for disconnected slots)
            .then_with(|| {
                if a.status == SlotStatus::Disconnected && b.status == SlotStatus::Disconnected {
                    a.checks.0.cmp(&b.checks.0)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            // by last_activity (descending)
            .then_with(|| match (a.last_activity, b.last_activity) {
                (Some(x), Some(y)) => y.total_cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
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
        .unique_by(|slot| lobby_room.yamls.get(slot.id - 1).unwrap().discord_id)
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

async fn fetch_deathlinks(config: &Config, room_id: &str) -> crate::error::Result<Vec<Deathlink>> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/deathlinks/{}", apx_api_root, room_id))
        .header("X-API-Key", apx_api_key)
        .send()
        .await?;

    Ok(response.json().await?)
}

async fn fetch_exclusions(config: &Config) -> crate::error::Result<Vec<usize>> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/deathlink_exclusions", apx_api_root))
        .header("X-API-Key", apx_api_key)
        .send()
        .await?;

    let data: ExclusionsResponse = response.json().await?;
    Ok(data.excluded_slots)
}

#[rocket::get("/deathlinks")]
async fn deathlinks(
    _session: LoggedInSession,
    lobby_room: LobbyRoom,
    ap_room: ApRoom,
    config: &State<Config>,
) -> crate::error::Result<DeathlinksIndexTpl> {
    let room_id = lobby_room.id.to_string();

    let deathlinks = fetch_deathlinks(config, &room_id).await.unwrap_or_default();
    let excluded_slots = fetch_exclusions(config).await.unwrap_or_default();

    let slots: Vec<DeathlinksSlot> = ap_room
        .tracker_info
        .slots
        .iter()
        .zip(lobby_room.yamls.iter())
        .map(|(slot, lobby_slot)| DeathlinksSlot {
            id: slot.id,
            name: slot.name.clone(),
            game: slot.game.clone(),
            discord_handle: lobby_slot.discord_handle.clone(),
            is_excluded: excluded_slots.contains(&slot.id),
        })
        .collect();

    let slot_names: HashMap<usize, &str> = slots.iter().map(|s| (s.id, s.name.as_str())).collect();
    let mut deaths_by_slot: Vec<SlotDeathCount> = deathlinks
        .iter()
        .map(|dl| dl.slot)
        .counts()
        .into_iter()
        .map(|(slot, count)| SlotDeathCount {
            slot,
            name: slot_names[&slot].to_string(),
            count,
        })
        .collect();
    deaths_by_slot.sort_by(|a, b| b.count.cmp(&a.count));

    Ok(DeathlinksIndexTpl {
        lobby_room,
        lobby_root_url: config.lobby_root_url.to_string(),
        is_session_valid: config.is_session_valid,
        slots,
        deathlinks,
        deaths_by_slot,
    })
}

#[rocket::post("/api/deathlink_exclusions/<slot>")]
async fn proxy_add_exclusion(
    _session: LoggedInSession,
    slot: u32,
    config: &State<Config>,
) -> crate::error::Result<rocket::http::Status> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{}/api/deathlink_exclusions/{}",
            apx_api_root, slot
        ))
        .header("X-API-Key", apx_api_key)
        .send()
        .await?;

    Ok(rocket::http::Status::from_code(response.status().as_u16())
        .unwrap_or(rocket::http::Status::InternalServerError))
}

#[rocket::delete("/api/deathlink_exclusions/<slot>")]
async fn proxy_remove_exclusion(
    _session: LoggedInSession,
    slot: u32,
    config: &State<Config>,
) -> crate::error::Result<rocket::http::Status> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .delete(format!(
            "{}/api/deathlink_exclusions/{}",
            apx_api_root, slot
        ))
        .header("X-API-Key", apx_api_key)
        .send()
        .await?;

    Ok(rocket::http::Status::from_code(response.status().as_u16())
        .unwrap_or(rocket::http::Status::InternalServerError))
}

#[rocket::get("/api/deathlink_probability")]
async fn get_deathlink_probability(
    _session: LoggedInSession,
    config: &State<Config>,
) -> crate::error::Result<Json<ProbabilityResponse>> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/deathlink_probability", apx_api_root))
        .header("X-API-Key", apx_api_key)
        .send()
        .await?;

    let data: ProbabilityResponse = response.json().await?;
    Ok(Json(data))
}

#[rocket::put("/api/deathlink_probability", data = "<request>")]
async fn set_deathlink_probability(
    _session: LoggedInSession,
    config: &State<Config>,
    request: Json<SetProbabilityRequest>,
) -> crate::error::Result<Json<ProbabilityResponse>> {
    let apx_api_root = config
        .apx_api_root
        .as_ref()
        .ok_or_else(|| anyhow!("APX API not configured"))?;
    let apx_api_key = config
        .apx_api_key
        .as_ref()
        .ok_or_else(|| anyhow!("APX API key not configured"))?;

    let client = reqwest::Client::new();
    let response = client
        .put(format!("{}/api/deathlink_probability", apx_api_root))
        .header("X-API-Key", apx_api_key)
        .json(&request.into_inner())
        .send()
        .await?;

    let data: ProbabilityResponse = response.json().await?;
    Ok(Json(data))
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
    let url = format!("ws://{}:{}", config.ap_room_host, config.ap_room_port);
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

async fn notify_proxy_password_refresh(config: &State<Config>) {
    let (Some(apx_root), Some(apx_key)) = (&config.apx_api_root, &config.apx_api_key) else {
        return;
    };

    let apx_url = match apx_root.join("/api/refresh_passwords") {
        Ok(url) => url,
        Err(e) => {
            eprintln!("[REFRESH_PASSWORDS] Failed to build APX URL: {}", e);
            return;
        }
    };

    let Ok(header_value) = HeaderValue::from_str(apx_key) else {
        eprintln!("[REFRESH_PASSWORDS] Invalid APX API key");
        return;
    };

    let result = reqwest::Client::new()
        .post(apx_url)
        .header(HeaderName::from_static("x-api-key"), header_value)
        .send()
        .await;

    match result {
        Ok(resp) if !resp.status().is_success() => {
            eprintln!(
                "[REFRESH_PASSWORDS] APX API returned error: {}",
                resp.status()
            );
        }
        Err(e) => {
            eprintln!("[REFRESH_PASSWORDS] Failed to notify APX API: {}", e);
        }
        _ => {}
    }
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

    notify_proxy_password_refresh(config).await;

    Ok(())
}

fn deserialize_i64_from_string<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

#[derive(Deserialize, Serialize)]
struct ChangeYamlOwnerRequest {
    #[serde(deserialize_with = "deserialize_i64_from_string")]
    new_owner_id: i64,
    new_password: Option<String>,
}

#[rocket::put("/change_owner/<yaml_id>", data = "<request>")]
async fn change_yaml_owner(
    _session: LoggedInSession,
    yaml_id: &str,
    request: Json<ChangeYamlOwnerRequest>,
    config: &State<Config>,
) -> crate::error::Result<()> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(&config.lobby_api_key)?,
    );

    let url = config.lobby_root_url.join(&format!(
        "/api/room/{}/yaml/{}",
        config.lobby_room_id, yaml_id
    ))?;

    let response = client
        .put(url)
        .headers(headers)
        .json(&request.into_inner())
        .send()
        .await?;

    if !response.status().is_success() {
        Err(anyhow!(
            "Failed to change YAML owner: {}",
            response.status()
        ))?;
    }

    notify_proxy_password_refresh(config).await;

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

    let db_url = std::env::var("DATABASE_URL").expect("Provide a `DATABASE_URL` env variable");
    let db_pool = common::db::get_database_pool(&db_url, MIGRATIONS).await?;

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
                deathlinks,
                proxy_add_exclusion,
                proxy_remove_exclusion,
                get_deathlink_probability,
                set_deathlink_probability,
                release,
                hint,
                autocompletion,
                give,
                set_password,
                change_yaml_owner
            ],
        )
        .mount("/auth", auth::routes())
        .mount("/", review::page::routes())
        .mount("/api", review::api::routes())
        .register("/", catchers![unauthorized])
        .manage(rocket::Config::figment())
        .manage(config)
        .manage(db_pool)
        .attach(OAuth2::<Discord>::fairing("discord"))
        .launch()
        .await
        .unwrap();

    Ok(())
}
