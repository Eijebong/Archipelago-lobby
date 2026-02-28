use anyhow::anyhow;
use askama::Template;
use askama_web::WebTemplate;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use rocket::response::Redirect;
use rocket::{State, routes};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Role;
use super::db;
use crate::Config;
use crate::auth::LoggedInSession;

pub struct OrgTplContext {
    pub is_super_admin: bool,
    pub is_team_admin: bool,
    pub username: String,
    pub static_version: &'static str,
    pub cur_page: &'static str,
}

impl OrgTplContext {
    pub async fn new(
        session: &LoggedInSession,
        cur_page: &'static str,
        pool: &DieselPool<AsyncPgConnection>,
    ) -> anyhow::Result<Self> {
        let is_super_admin = session.is_super_admin();
        let is_team_admin = if is_super_admin {
            true
        } else {
            let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
            let user_teams = db::get_user_teams(session.user_id(), &mut conn).await?;
            user_teams
                .iter()
                .any(|(_, m)| m.role.parse::<Role>().ok() >= Some(Role::Admin))
        };

        Ok(OrgTplContext {
            is_super_admin,
            is_team_admin,
            username: session.username().to_string(),
            static_version: crate::STATIC_VERSION,
            cur_page,
        })
    }
}

#[derive(Deserialize)]
struct LobbyRoomBasic {
    name: String,
    locked: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "rooms.html")]
pub struct RoomsListTpl {
    base: OrgTplContext,
    rooms: Vec<db::RoomSummary>,
}

#[rocket::get("/rooms")]
async fn rooms_list(
    session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<RoomsListTpl> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let rooms = if session.is_super_admin() {
        db::list_all_rooms(&mut conn).await?
    } else {
        db::list_user_rooms(session.user_id(), &mut conn).await?
    };

    let base = OrgTplContext::new(&session, "rooms", pool).await?;
    Ok(RoomsListTpl { base, rooms })
}

#[derive(Template, WebTemplate)]
#[template(path = "review.html")]
pub struct ReviewTpl {
    base: OrgTplContext,
    room_id: String,
    room_name: String,
    assigned_preset_id: Option<i32>,
    lobby_root_url: String,
    is_locked: bool,
    user_id: i64,
    user_role: String,
}

#[rocket::get("/room/<room_id>/review")]
async fn review_page(
    session: LoggedInSession,
    room_id: &str,
    config: &State<Config>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<ReviewTpl> {
    let room_uuid: Uuid = room_id
        .parse()
        .map_err(|_| crate::error::bad_request("Invalid room ID"))?;

    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let user_role = if session.is_super_admin() {
        Role::Editor.as_str().to_string()
    } else {
        let role = db::get_user_role_for_room(session.user_id(), room_uuid, &mut conn).await?;
        match role {
            Some(r) => r.as_str().to_string(),
            None => return Err(crate::error::forbidden("Forbidden")),
        }
    };

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

    db::update_room_name(room_uuid, &room_info.name, &mut conn).await?;

    let room_config = db::get_room_config(room_uuid, &mut conn).await?;
    let base = OrgTplContext::new(&session, "room", pool).await?;

    Ok(ReviewTpl {
        base,
        room_id: room_uuid.to_string(),
        room_name: room_info.name,
        assigned_preset_id: room_config.map(|c| c.preset_id),
        lobby_root_url: config.lobby_root_url.to_string(),
        is_locked: room_info.locked,
        user_id: session.user_id(),
        user_role,
    })
}

#[rocket::get("/review/<room_id>")]
async fn review_redirect(room_id: &str) -> Redirect {
    Redirect::permanent(format!("/room/{}/review", room_id))
}

#[rocket::get("/room/<room_id>", rank = 10)]
async fn room_redirect(room_id: &str) -> Redirect {
    Redirect::to(format!("/room/{}/review", room_id))
}

// -- Preset list page --

#[derive(Template, WebTemplate)]
#[template(path = "presets.html")]
pub struct PresetsListTpl {
    base: OrgTplContext,
    presets: Vec<db::PresetSummary>,
}

#[rocket::get("/presets")]
async fn presets_list(
    session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<PresetsListTpl> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let presets =
        db::list_presets_for_user(session.user_id(), session.is_super_admin(), &mut conn).await?;
    let base = OrgTplContext::new(&session, "presets", pool).await?;
    Ok(PresetsListTpl { base, presets })
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
    base: OrgTplContext,
    preset: PresetForTemplate,
    back_url: Option<String>,
    can_edit_rules: bool,
}

#[rocket::get("/presets/<id>?<from_room>")]
async fn preset_edit(
    session: LoggedInSession,
    id: i32,
    from_room: Option<String>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<PresetEditTpl> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let can_edit_rules = if session.is_super_admin() {
        true
    } else {
        let role = db::get_user_role_for_preset(session.user_id(), id, &mut conn).await?;
        match role {
            Some(r) if r >= Role::RuleEditor => true,
            Some(_) => false,
            None => return Err(crate::error::forbidden("Forbidden")),
        }
    };

    let preset = db::get_preset(id, &mut conn).await?;
    let db_rules = db::list_rules_for_preset(id, &mut conn).await?;

    let rules_for_tpl: Vec<RuleForTemplate> = db_rules
        .into_iter()
        .map(|r| {
            let edited_at = r
                .last_edited_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string());
            RuleForTemplate {
                id: r.id,
                rule: r.rule,
                position: r.position,
                last_edited_by_name: r.last_edited_by_name,
                last_edited_at: edited_at,
            }
        })
        .collect();

    let back_url = from_room.map(|room_id| format!("/room/{}/review", room_id));
    let base = OrgTplContext::new(&session, "preset_edit", pool).await?;

    Ok(PresetEditTpl {
        base,
        preset: PresetForTemplate {
            id: preset.id,
            name: preset.name,
            rules: serde_json::to_string(&rules_for_tpl)?,
            builtin_rules: serde_json::to_string(&preset.builtin_rules)?,
        },
        back_url,
        can_edit_rules,
    })
}

// -- Admin teams page --

#[derive(Template, WebTemplate)]
#[template(path = "admin_teams.html")]
pub struct AdminTeamsTpl {
    base: OrgTplContext,
}

#[rocket::get("/admin/teams")]
async fn admin_teams_page(
    session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<AdminTeamsTpl> {
    if !session.is_super_admin() {
        let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
        let user_teams = db::get_user_teams(session.user_id(), &mut conn).await?;
        let is_team_admin = user_teams
            .iter()
            .any(|(_, m)| m.role.parse::<Role>().ok() >= Some(Role::Admin));
        if !is_team_admin {
            return Err(anyhow!("Forbidden").into());
        }
    }

    let base = OrgTplContext::new(&session, "teams", pool).await?;
    Ok(AdminTeamsTpl { base })
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        rooms_list,
        review_page,
        review_redirect,
        room_redirect,
        presets_list,
        preset_edit,
        admin_teams_page
    ]
}
