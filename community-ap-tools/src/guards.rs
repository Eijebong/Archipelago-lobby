use std::{fmt::Display, str::FromStr, sync::OnceLock};

use aprs::proto::server::DataPackage;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{
    Url,
    header::{HeaderName, HeaderValue},
};
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

#[derive(Deserialize, Debug)]
pub struct YamlInfo {
    pub id: Uuid,
    pub discord_handle: String,
    pub slot_number: usize,
    pub has_patch: bool,
}

#[derive(Deserialize, Debug)]
pub struct LobbyRoom {
    pub id: Uuid,
    pub name: String,
    pub yamls: Vec<YamlInfo>,
}

#[derive(Deserialize, Debug)]
pub struct RoomStatus {
    pub tracker: String,
    pub last_port: u16,
}

pub struct ApRoom {
    pub id: String,
    pub room_status: RoomStatus,
    pub tracker_info: TrackerInfo,
}

#[derive(Debug)]
pub struct TrackerInfo {
    pub slots: Vec<SlotInfo>,
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Clone)]
pub enum SlotStatus {
    Disconnected,
    Connected,
    Ready,
    GoalCompleted,
    Unkown(String),
}

impl FromStr for SlotStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Goal Completed" => Self::GoalCompleted,
            "Disconnected" => Self::Disconnected,
            "Connected" => Self::Connected,
            "Ready" => Self::Ready,
            _ => Self::Unkown(s.to_string()),
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
            Self::Unkown(s) => f.write_fmt(format_args!("Unknown ({})", s)),
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
            $e.map_err(|e| e.into())
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

#[derive(Deserialize)]
pub struct ApMsg {
    #[serde(flatten)]
    data: DataPackage,
}
#[derive(Deserialize)]
pub struct DPackage(Vec<ApMsg>);

pub static DATA_PACKAGE: OnceLock<DataPackage> = OnceLock::new();
#[rocket::async_trait]
impl<'r> FromRequest<'r> for ApRoom {
    type Error = crate::error::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let config = request.rocket().state::<Config>().unwrap();
        let result = try_err_outcome!(
            reqwest::get(
                Url::from_str(&format!(
                    "https://archipelago.gg/api/room_status/{}",
                    config.ap_room_id
                ))
                .unwrap()
            )
            .await
        );
        let room_status: RoomStatus = try_err_outcome!(result.json().await);

        let tracker_url = try_err_outcome!(Url::from_str(&format!(
            "https://archipelago.gg/tracker/{}",
            suuid(room_status.tracker.parse().unwrap())
        )));
        let tracker_page = try_err_outcome!(reqwest::get(tracker_url).await);
        let tracker_body = try_err_outcome!(tracker_page.text().await);

        let tracker_info = try_err_outcome!(parse_tracker(tracker_body));
        DATA_PACKAGE.get_or_init(|| {
            let url = format!("wss://archipelago.gg:{}", room_status.last_port);
            let (mut socket, _) = tungstenite::connect(&url).unwrap();
            let msg = "[{\"cmd\": \"GetDataPackage\"}]";
            let _ = socket.read().unwrap();
            socket.send(tungstenite::Message::Text(msg.into())).unwrap();
            socket.flush().unwrap();
            let raw_datapackage = socket.read().unwrap();
            socket.close(None).unwrap();

            dbg!(&raw_datapackage);
            let mut dp: DPackage =
                serde_json::from_str(raw_datapackage.to_text().unwrap()).unwrap();
            dp.0.pop().unwrap().data
        });

        Outcome::Success(ApRoom {
            id: config.ap_room_id.clone(),
            room_status,
            tracker_info,
        })
    }
}

fn suuid(uuid: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(uuid.as_bytes())
}

fn parse_tracker(body: String) -> crate::error::Result<TrackerInfo> {
    let mut slots = Vec::new();
    let html = Html::parse_document(&body);
    let slot_lines_selector = Selector::parse("#checks-table > tbody > tr").unwrap();
    let td_selector = Selector::parse("td").unwrap();
    let a_selector = Selector::parse("a").unwrap();
    let slot_lines = html.select(&slot_lines_selector);

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
        let slot_name = htmlize::unescape(cells.next().unwrap().inner_html().trim().to_string());
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
