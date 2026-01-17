use std::{collections::BTreeMap, fmt::Display, str::FromStr, sync::OnceLock};

use reqwest::header::{HeaderName, HeaderValue};
use rocket::{
    Request,
    http::Status,
    outcome::{IntoOutcome, try_outcome},
    request::{FromRequest, Outcome},
};
use scraper::{Html, Selector};
use serde::Deserialize;
use uuid::Uuid;

use crate::Config;
use crate::datapackage::DataPackage;

#[derive(Deserialize, Debug)]
pub struct YamlInfo {
    pub id: Uuid,
    pub discord_handle: String,
    pub slot_number: usize,
    pub has_patch: bool,
}

#[derive(Deserialize, Debug)]
pub struct SlotPasswordInfo {
    pub password: Option<String>,
}

#[derive(Debug)]
pub struct SlotPasswords(pub Vec<SlotPasswordInfo>);

#[derive(Deserialize, Debug)]
pub struct LobbyRoom {
    pub id: Uuid,
    pub name: String,
    pub yamls: Vec<YamlInfo>,
}

#[derive(Deserialize, Debug)]
pub struct RoomStatus {
    pub tracker: String,
}

pub struct ApRoom {
    pub tracker_info: TrackerInfo,
}

#[derive(Debug)]
pub struct TrackerInfo {
    pub slots: Vec<SlotInfo>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum SlotStatus {
    Disconnected,
    Connected,
    Ready,
    Playing,
    GoalCompleted,
    Unknown(String),
}

impl SlotStatus {
    fn sort_key(&self) -> u8 {
        match self {
            Self::Unknown(_) => 0,
            Self::Disconnected => 1,
            Self::Connected | Self::Ready | Self::Playing => 2,
            Self::GoalCompleted => 3,
        }
    }
}

impl PartialOrd for SlotStatus {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SlotStatus {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl FromStr for SlotStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Goal Completed" => Self::GoalCompleted,
            "Disconnected" => Self::Disconnected,
            "Connected" => Self::Connected,
            "Ready" => Self::Ready,
            "Playing" => Self::Playing,
            _ => Self::Unknown(s.to_string()),
        })
    }
}

impl Display for SlotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GoalCompleted => f.write_str("Goal Completed"),
            Self::Disconnected => f.write_str("Disconnected"),
            Self::Connected => f.write_str("Connected"),
            Self::Ready => f.write_str("Ready"),
            Self::Playing => f.write_str("Playing"),
            Self::Unknown(s) => f.write_fmt(format_args!("Unknown ({s})")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub id: usize,
    pub name: String,
    pub game: String,
    pub checks: (u64, u64),
    pub status: SlotStatus,
    pub last_activity: Option<f64>,
}

macro_rules! try_err_outcome {
    ($e: expr) => {
        try_outcome!(
            $e.map_err(|e| {
                eprintln!("[GUARD ERROR] {}: {:?}", stringify!($e), e);
                e.into()
            })
            .or_error(Status::InternalServerError)
        )
    };
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LobbyRoom {
    type Error = crate::error::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let config = request.rocket().state::<Config>().unwrap();
        let url = try_err_outcome!(
            config
                .lobby_root_url
                .join(&format!("/api/room/{}", config.lobby_room_id))
        );
        let client = reqwest::Client::new();
        let result = try_err_outcome!(
            client
                .get(url)
                .header(
                    HeaderName::from_static("x-api-key"),
                    HeaderValue::from_str(&config.lobby_api_key).unwrap()
                )
                .send()
                .await
        );
        let mut room: LobbyRoom = try_err_outcome!(result.json().await);
        room.yamls.sort_by_key(|yaml| yaml.slot_number);

        Outcome::Success(room)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SlotPasswords {
    type Error = crate::error::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let config = request.rocket().state::<Config>().unwrap();
        let url = try_err_outcome!(config.lobby_root_url.join(&format!(
            "/api/room/{}/slots_passwords",
            config.lobby_room_id
        )));
        let client = reqwest::Client::new();
        let result = try_err_outcome!(
            client
                .get(url)
                .header(
                    HeaderName::from_static("x-api-key"),
                    HeaderValue::from_str(&config.lobby_api_key).unwrap()
                )
                .send()
                .await
        );
        let passwords: Vec<SlotPasswordInfo> = try_err_outcome!(result.json().await);

        Outcome::Success(SlotPasswords(passwords))
    }
}

#[derive(Deserialize)]
pub struct ApMsg {
    #[serde(flatten)]
    data: DataPackage,
}
#[derive(Deserialize)]
pub struct DPackage(Vec<ApMsg>);

pub static DATA_PACKAGE: OnceLock<DataPackage> = OnceLock::new();
pub static SLOT_MAPPING: OnceLock<BTreeMap<usize, String>> = OnceLock::new();

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ApRoom {
    type Error = crate::error::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let config = request.rocket().state::<Config>().unwrap();
        let room_status_url = try_err_outcome!(
            config
                .ap_api_root
                .join(&format!("/api/room_status/{}", config.ap_room_id))
        );
        eprintln!("[GUARD] Fetching room status from: {}", room_status_url);
        let result = try_err_outcome!(reqwest::get(room_status_url).await);
        let room_status: RoomStatus = try_err_outcome!(result.json().await);

        if SLOT_MAPPING.get().is_none() {
            let response = try_err_outcome!(reqwest::get(config.ap_room_url.clone()).await);
            let body = try_err_outcome!(response.text().await);
            let slots = try_err_outcome!(parse_room(body));

            SLOT_MAPPING.set(slots).unwrap();
        }

        let tracker_url = try_err_outcome!(
            config
                .ap_api_root
                .join(&format!("/tracker/{}", room_status.tracker))
        );
        eprintln!("[GUARD] Fetching tracker from: {}", tracker_url);
        let tracker_page = try_err_outcome!(reqwest::get(tracker_url).await);
        let tracker_body = try_err_outcome!(tracker_page.text().await);

        let tracker_info = try_err_outcome!(parse_tracker(tracker_body));
        let ap_host = config.ap_room_host.clone();
        let ap_port = config.ap_room_port;
        let ws_scheme = if config.ap_api_root.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        DATA_PACKAGE.get_or_init(move || {
            let url = format!("{}://{}:{}", ws_scheme, ap_host, ap_port);
            eprintln!("[GUARD] Connecting to WebSocket at: {}", url);
            let (mut socket, _) = tungstenite::connect(&url).unwrap();
            let msg = "[{\"cmd\": \"GetDataPackage\"}]";
            let _ = socket.read().unwrap();
            socket.send(tungstenite::Message::Text(msg.into())).unwrap();
            socket.flush().unwrap();
            let raw_datapackage = socket.read().unwrap();
            socket.close(None).unwrap();

            let mut dp: DPackage =
                serde_json::from_str(raw_datapackage.to_text().unwrap()).unwrap();
            dp.0.pop().unwrap().data
        });

        Outcome::Success(ApRoom { tracker_info })
    }
}

fn parse_room(body: String) -> crate::error::Result<BTreeMap<usize, String>> {
    let mut slots = BTreeMap::new();
    let html = Html::parse_document(&body);
    let slot_lines_selector = Selector::parse("#slots-table > tbody > tr").unwrap();
    let slot_lines = html.select(&slot_lines_selector);
    let td_selector = Selector::parse("td").unwrap();
    let a_selector = Selector::parse("a").unwrap();

    for slot_line in slot_lines {
        let mut cells = slot_line.select(&td_selector);
        let slot_id = cells.next().unwrap().inner_html().trim().parse::<usize>()?;
        let slot_name = htmlize::unescape(
            cells
                .next()
                .unwrap()
                .select(&a_selector)
                .next()
                .unwrap()
                .inner_html()
                .trim()
                .to_string(),
        );

        slots.insert(slot_id, slot_name.to_string());
    }

    Ok(slots)
}

fn parse_tracker(body: String) -> crate::error::Result<TrackerInfo> {
    let mut slots = Vec::new();
    let html = Html::parse_document(&body);
    let slot_lines_selector = Selector::parse("#checks-table > tbody > tr").unwrap();
    let td_selector = Selector::parse("td").unwrap();
    let a_selector = Selector::parse("a").unwrap();
    let slot_lines = html.select(&slot_lines_selector);
    let slot_map = SLOT_MAPPING.get().unwrap();

    for slot_line in slot_lines {
        let mut cells = slot_line.select(&td_selector);

        let slot_id = cells
            .next()
            .unwrap()
            .select(&a_selector)
            .next()
            .unwrap()
            .inner_html()
            .trim()
            .parse::<usize>()?;
        let _ = cells.next(); // Jump over the slot name
        let slot_name = slot_map.get(&slot_id).unwrap();
        let slot_game = htmlize::unescape(cells.next().unwrap().inner_html().trim().to_string());
        let status = cells.next().unwrap().inner_html().trim().to_string();
        let checks = cells
            .next()
            .unwrap()
            .inner_html()
            .trim()
            .to_string()
            .split_once('/')
            .map(|(v1, v2)| (v1.parse::<u64>().unwrap(), v2.parse::<u64>().unwrap()))
            .unwrap();
        let _percent = cells.next();
        let last_activity = cells
            .next()
            .unwrap()
            .inner_html()
            .trim()
            .to_string()
            .parse::<f64>()
            .ok();
        let slot_info = SlotInfo {
            id: slot_id,
            name: slot_name.to_string(),
            game: slot_game.to_string(),
            status: status.parse().unwrap(),
            checks,
            last_activity,
        };

        slots.push(slot_info);
    }

    Ok(TrackerInfo { slots })
}
