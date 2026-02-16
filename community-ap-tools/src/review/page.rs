use anyhow::anyhow;
use askama::Template;
use askama_web::WebTemplate;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use rocket::{State, routes};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::db;
use crate::Config;
use crate::auth::LoggedInSession;

#[derive(Deserialize)]
struct LobbyRoomBasic {
    name: String,
}

// -- Review page (results-focused) --

#[derive(Template, WebTemplate)]
#[template(path = "review.html")]
pub struct ReviewTpl {
    room_id: String,
    room_name: String,
    assigned_preset_id: Option<i32>,
    lobby_root_url: String,
}

#[rocket::get("/review/<room_id>")]
async fn review_page(
    _session: LoggedInSession,
    room_id: &str,
    config: &State<Config>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<ReviewTpl> {
    let room_uuid: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;

    let client = reqwest::Client::new();
    let room_url = config
        .lobby_root_url
        .join(&format!("/api/room/{}", room_uuid))?;
    let room_resp = client
        .get(room_url)
        .header("x-api-key", &config.lobby_api_key)
        .send()
        .await?;
    if !room_resp.status().is_success() {
        return Err(anyhow!("Failed to fetch room info: {}", room_resp.status()).into());
    }
    let room_info: LobbyRoomBasic = room_resp.json().await?;

    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let room_config = db::get_room_config(room_uuid, &mut conn).await?;

    Ok(ReviewTpl {
        room_id: room_uuid.to_string(),
        room_name: room_info.name,
        assigned_preset_id: room_config.map(|c| c.preset_id),
        lobby_root_url: config.lobby_root_url.to_string(),
    })
}

// -- Preset list page --

#[derive(Template, WebTemplate)]
#[template(path = "presets.html")]
pub struct PresetsListTpl {
    presets: Vec<db::PresetSummary>,
}

#[rocket::get("/presets")]
async fn presets_list(
    _session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<PresetsListTpl> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let presets = db::list_presets(&mut conn).await?;
    Ok(PresetsListTpl { presets })
}

// -- Preset edit page --

#[derive(Serialize)]
struct PresetForTemplate {
    id: i32,
    name: String,
    rules: String,
    builtin_rules: String,
}

#[derive(Serialize)]
struct RuleForTemplate {
    id: i32,
    rule: serde_json::Value,
    position: i32,
    last_edited_by_name: Option<String>,
    last_edited_at: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "preset_edit.html")]
pub struct PresetEditTpl {
    preset: PresetForTemplate,
    back_url: Option<String>,
    current_username: String,
}

#[rocket::get("/presets/<id>?<from_room>")]
async fn preset_edit(
    session: LoggedInSession,
    id: i32,
    from_room: Option<String>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<PresetEditTpl> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let current_username = db::get_editor_username(session.user_id(), &mut conn)
        .await?
        .unwrap_or_default();
    let preset = db::get_preset(id, &mut conn).await?;
    let db_rules = db::list_rules_for_preset(id, &mut conn).await?;

    let editor_ids: Vec<i64> = db_rules.iter().filter_map(|r| r.last_edited_by).collect();
    let editor_names = db::get_editor_usernames(&editor_ids, &mut conn).await?;
    let name_map: std::collections::HashMap<i64, String> = editor_names.into_iter().collect();

    let rules_for_tpl: Vec<RuleForTemplate> = db_rules
        .into_iter()
        .map(|r| {
            let editor_name = r.last_edited_by.and_then(|id| name_map.get(&id).cloned());
            let edited_at = r
                .last_edited_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string());
            RuleForTemplate {
                id: r.id,
                rule: r.rule,
                position: r.position,
                last_edited_by_name: editor_name,
                last_edited_at: edited_at,
            }
        })
        .collect();

    let back_url = from_room.map(|room_id| format!("/review/{}", room_id));

    Ok(PresetEditTpl {
        preset: PresetForTemplate {
            id: preset.id,
            name: preset.name,
            rules: serde_json::to_string(&rules_for_tpl)?,
            builtin_rules: serde_json::to_string(&preset.builtin_rules)?,
        },
        back_url,
        current_username,
    })
}

pub fn routes() -> Vec<rocket::Route> {
    routes![review_page, presets_list, preset_edit]
}
