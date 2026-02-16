use anyhow::anyhow;
use chrono::Utc;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use futures::future::join_all;
use rocket::{State, routes, serde::json::Json};
use saphyr::{LoadableYamlNode, YamlOwned as Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::builtin::{self, RoomYaml};
use super::db::{self, NewPreset, NewPresetRule, UpdatePreset, UpdatePresetRule};
use super::rules::{self, Outcome, Predicate, Rule, Severity};
use super::triggers;
use crate::Config;
use crate::auth::LoggedInSession;

#[rocket::get("/presets")]
async fn list_presets(
    _session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::PresetSummary>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let presets = db::list_presets(&mut conn).await?;
    Ok(Json(presets))
}

#[rocket::get("/presets/<id>")]
async fn get_preset(
    _session: LoggedInSession,
    id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::ReviewPreset>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let preset = db::get_preset(id, &mut conn).await?;
    Ok(Json(preset))
}

#[rocket::post("/presets", data = "<body>")]
async fn create_preset(
    _session: LoggedInSession,
    body: Json<NewPreset>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::ReviewPreset>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let preset = db::create_preset(body.into_inner(), &mut conn).await?;
    Ok(Json(preset))
}

#[rocket::put("/presets/<id>", data = "<body>")]
async fn update_preset(
    _session: LoggedInSession,
    id: i32,
    body: Json<UpdatePreset>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::ReviewPreset>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let preset = db::update_preset(id, body.into_inner(), &mut conn).await?;
    Ok(Json(preset))
}

#[rocket::delete("/presets/<id>")]
async fn delete_preset(
    _session: LoggedInSession,
    id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    db::delete_preset(id, &mut conn).await?;
    Ok(())
}

#[rocket::get("/builtin_rules")]
async fn list_builtin_rules(_session: LoggedInSession) -> Json<Vec<builtin::BuiltinRuleInfo>> {
    Json(builtin::builtin_rule_info())
}

#[derive(Deserialize)]
struct EvaluateRequest {
    #[serde(default)]
    preset_id: Option<i32>,
    #[serde(default)]
    rules: Option<Vec<Rule>>,
    #[serde(default)]
    builtin_rules: Option<Vec<String>>,
}

#[derive(Serialize)]
struct EvaluateResponse {
    yamls: Vec<YamlEvalResult>,
}

#[derive(Serialize)]
struct YamlEvalResult {
    yaml_id: Uuid,
    player_name: String,
    discord_handle: String,
    game: String,
    created_at: String,
    content: String,
    results: Vec<RuleResultResponse>,
}

#[derive(Serialize)]
struct RuleResultResponse {
    rule_name: String,
    outcome: Outcome,
    severity: Severity,
    detail: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    builtin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    condition: Option<Predicate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    predicate: Option<Predicate>,
}

#[derive(Deserialize)]
struct LobbyRoomInfo {
    yamls: Vec<LobbyYamlInfo>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct LobbyYamlInfo {
    id: Uuid,
    player_name: String,
    discord_handle: String,
    game: String,
    created_at: String,
}

#[derive(Deserialize)]
struct LobbyYamlDetail {
    content: String,
    game: String,
    player_name: String,
}

#[rocket::post("/review/<room_id>/evaluate", data = "<body>")]
async fn evaluate(
    _session: LoggedInSession,
    room_id: &str,
    body: Json<EvaluateRequest>,
    config: &State<Config>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<EvaluateResponse>> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let request = body.into_inner();

    let (custom_rules, enabled_builtins) = if let Some(preset_id) = request.preset_id {
        let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
        let preset = db::get_preset(preset_id, &mut conn).await?;
        let db_rules = db::list_rules_for_preset(preset_id, &mut conn).await?;
        let custom: Vec<Rule> = db_rules
            .iter()
            .filter_map(|r| serde_json::from_value(r.rule.clone()).ok())
            .collect();
        let builtins: Vec<String> = serde_json::from_value(preset.builtin_rules)?;
        (custom, builtins)
    } else {
        (
            request.rules.unwrap_or_default(),
            request.builtin_rules.unwrap_or_default(),
        )
    };

    let client = reqwest::Client::new();
    let room_url = config
        .lobby_root_url
        .join(&format!("/api/room/{}", room_id))?;
    let room_resp = client
        .get(room_url)
        .header("x-api-key", &config.lobby_api_key)
        .send()
        .await?;
    if !room_resp.status().is_success() {
        return Err(anyhow!("Failed to fetch room info: {}", room_resp.status()).into());
    }
    let room_info: LobbyRoomInfo = room_resp.json().await?;

    let yaml_futures: Vec<_> = room_info
        .yamls
        .iter()
        .map(|yaml_info| {
            let client = &client;
            let config = config.inner();
            let yaml_id = yaml_info.id;
            let discord_handle = yaml_info.discord_handle.clone();
            let created_at = yaml_info.created_at.clone();
            async move {
                let url = config
                    .lobby_root_url
                    .join(&format!("/api/room/{}/info/{}", room_id, yaml_id))?;
                let resp = client
                    .get(url)
                    .header("x-api-key", &config.lobby_api_key)
                    .send()
                    .await?;
                let detail: LobbyYamlDetail = resp.json().await?;
                Ok::<_, anyhow::Error>((yaml_id, discord_handle, created_at, detail))
            }
        })
        .collect();

    let yaml_details: Vec<(Uuid, String, String, LobbyYamlDetail)> = join_all(yaml_futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let room_yamls: Vec<RoomYaml> = yaml_details
        .iter()
        .map(|(_, _, _, detail)| RoomYaml {
            player_name: detail.player_name.clone(),
        })
        .collect();

    let builtin_rules_registry = builtin::builtin_rules();
    let active_builtins: Vec<&dyn builtin::BuiltinRule> = builtin_rules_registry
        .iter()
        .filter(|r| enabled_builtins.contains(&r.id().to_string()))
        .map(|r| r.as_ref())
        .collect();

    let results: Vec<YamlEvalResult> = yaml_details
        .iter()
        .map(|(yaml_id, discord_handle, created_at, detail)| {
            evaluate_single_yaml(
                *yaml_id,
                discord_handle,
                created_at,
                detail,
                &custom_rules,
                &active_builtins,
                &room_yamls,
            )
        })
        .collect();

    Ok(Json(EvaluateResponse { yamls: results }))
}

fn extract_game_names(yaml: &Value) -> Vec<String> {
    let Some(game_val) = yaml.as_mapping_get("game") else {
        return vec![];
    };

    if let Some(name) = game_val.as_str() {
        return vec![name.to_string()];
    }

    if let Some(map) = game_val.as_mapping() {
        return map
            .iter()
            .filter(|(_, weight)| weight.as_integer().unwrap_or(0) != 0)
            .filter_map(|(key, _)| key.as_str().map(|s| s.to_string()))
            .collect();
    }

    vec![]
}

fn evaluate_single_yaml(
    yaml_id: Uuid,
    discord_handle: &str,
    created_at: &str,
    detail: &LobbyYamlDetail,
    custom_rules: &[Rule],
    active_builtins: &[&dyn builtin::BuiltinRule],
    room_yamls: &[RoomYaml],
) -> YamlEvalResult {
    let parsed = Value::load_from_str(&detail.content)
        .ok()
        .and_then(|mut docs| docs.pop());

    let Some(yaml) = parsed else {
        return YamlEvalResult {
            yaml_id,
            player_name: detail.player_name.clone(),
            discord_handle: discord_handle.to_string(),
            game: detail.game.clone(),
            created_at: created_at.to_string(),
            content: detail.content.clone(),
            results: vec![RuleResultResponse {
                rule_name: "YAML Parse".into(),
                outcome: Outcome::Error,
                severity: Severity::Error,
                detail: Some("Failed to parse YAML".into()),
                builtin: false,
                condition: None,
                predicate: None,
            }],
        };
    };

    let resolved = triggers::resolve_triggers(&yaml);

    let game_names = extract_game_names(&resolved);
    let game_names = if game_names.is_empty() {
        vec![detail.game.clone()]
    } else {
        game_names
    };
    let multi_game = game_names.len() > 1;

    let mut all_results: Vec<RuleResultResponse> = Vec::new();

    for game_name in &game_names {
        let rule_results = rules::evaluate_rules_for_yaml(custom_rules, &resolved, game_name);

        all_results.extend(
            rule_results
                .into_iter()
                .zip(custom_rules.iter())
                .filter(|(r, _)| r.outcome == Outcome::Fail || r.outcome == Outcome::Error)
                .map(|(r, rule)| {
                    let (condition, predicate) = if r.outcome == Outcome::Fail {
                        (rule.when.clone(), Some(rule.then.clone()))
                    } else {
                        (None, None)
                    };
                    let detail = match (multi_game, r.detail) {
                        (true, Some(d)) => Some(format!("[{}] {}", game_name, d)),
                        (true, None) => Some(format!("[{}]", game_name)),
                        (false, d) => d,
                    };
                    RuleResultResponse {
                        rule_name: r.rule_name,
                        outcome: r.outcome,
                        severity: r.severity,
                        detail,
                        builtin: false,
                        condition,
                        predicate,
                    }
                }),
        );

        for builtin_rule in active_builtins {
            let br = builtin_rule.evaluate(&resolved, game_name, &detail.player_name, room_yamls);
            if br.outcome != Outcome::Fail && br.outcome != Outcome::Error {
                continue;
            }
            let detail = match (multi_game, br.detail) {
                (true, Some(d)) => Some(format!("[{}] {}", game_name, d)),
                (true, None) => Some(format!("[{}]", game_name)),
                (false, d) => d,
            };
            all_results.push(RuleResultResponse {
                rule_name: br.rule_name,
                outcome: br.outcome,
                severity: br.severity,
                detail,
                builtin: true,
                condition: None,
                predicate: None,
            });
        }
    }

    let display_game = if multi_game {
        format!("Random ({})", game_names.len())
    } else {
        game_names
            .into_iter()
            .next()
            .unwrap_or_else(|| detail.game.clone())
    };

    YamlEvalResult {
        yaml_id,
        player_name: detail.player_name.clone(),
        discord_handle: discord_handle.to_string(),
        game: display_game,
        created_at: created_at.to_string(),
        content: detail.content.clone(),
        results: all_results,
    }
}

#[derive(Deserialize)]
struct SetRoomPresetRequest {
    preset_id: i32,
}

#[rocket::get("/room/<room_id>/preset")]
async fn get_room_preset(
    _session: LoggedInSession,
    room_id: &str,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Option<db::RoomReviewConfig>>> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let config = db::get_room_config(room_id, &mut conn).await?;
    Ok(Json(config))
}

#[rocket::put("/room/<room_id>/preset", data = "<body>")]
async fn set_room_preset(
    _session: LoggedInSession,
    room_id: &str,
    body: Json<SetRoomPresetRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    db::set_room_preset(room_id, body.preset_id, &mut conn).await?;
    Ok(())
}

#[rocket::delete("/room/<room_id>/preset")]
async fn remove_room_preset(
    _session: LoggedInSession,
    room_id: &str,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    db::remove_room_preset(room_id, &mut conn).await?;
    Ok(())
}

#[rocket::get("/games")]
async fn proxy_games(
    _session: LoggedInSession,
    config: &State<Config>,
) -> crate::error::Result<Json<serde_json::Value>> {
    let client = reqwest::Client::new();
    let url = config.lobby_root_url.join("/api/games")?;
    let resp = client
        .get(url)
        .header("x-api-key", &config.lobby_api_key)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Failed to fetch games: {}", resp.status()).into());
    }
    let data: serde_json::Value = resp.json().await?;
    Ok(Json(data))
}

#[rocket::get("/games/<apworld>/options")]
async fn proxy_game_options(
    _session: LoggedInSession,
    apworld: &str,
    config: &State<Config>,
) -> crate::error::Result<Json<serde_json::Value>> {
    let client = reqwest::Client::new();
    let url = config
        .lobby_root_url
        .join(&format!("/api/games/{}/options", apworld))?;
    let resp = client
        .get(url)
        .header("x-api-key", &config.lobby_api_key)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Failed to fetch game options: {}", resp.status()).into());
    }
    let data: serde_json::Value = resp.json().await?;
    Ok(Json(data))
}

#[rocket::get("/presets/<preset_id>/rules")]
async fn list_preset_rules(
    _session: LoggedInSession,
    preset_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::PresetRule>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let rules = db::list_rules_for_preset(preset_id, &mut conn).await?;
    Ok(Json(rules))
}

#[rocket::post("/presets/<preset_id>/rules", data = "<body>")]
async fn create_preset_rule(
    session: LoggedInSession,
    preset_id: i32,
    body: Json<NewPresetRuleRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::PresetRule>> {
    let req = body.into_inner();
    let parsed: Rule =
        serde_json::from_value(req.rule.clone()).map_err(|e| anyhow!("Invalid rule: {}", e))?;
    parsed.validate().map_err(|e| anyhow!("{}", e))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let now = Utc::now();
    let rule = db::create_rule(
        NewPresetRule {
            preset_id,
            rule: req.rule,
            position: req.position,
            last_edited_by: Some(session.user_id()),
            last_edited_at: Some(now),
        },
        &mut conn,
    )
    .await?;
    Ok(Json(rule))
}

#[derive(Deserialize)]
struct NewPresetRuleRequest {
    rule: serde_json::Value,
    position: i32,
}

#[derive(Deserialize)]
struct UpdatePresetRuleRequest {
    rule: Option<serde_json::Value>,
    position: Option<i32>,
}

#[rocket::put("/presets/<preset_id>/rules/<rule_id>", data = "<body>")]
async fn update_preset_rule(
    session: LoggedInSession,
    preset_id: i32,
    rule_id: i32,
    body: Json<UpdatePresetRuleRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::PresetRule>> {
    let req = body.into_inner();
    if let Some(ref rule_value) = req.rule {
        let parsed: Rule = serde_json::from_value(rule_value.clone())
            .map_err(|e| anyhow!("Invalid rule: {}", e))?;
        parsed.validate().map_err(|e| anyhow!("{}", e))?;
    }
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let now = Utc::now();
    let rule = db::update_rule(
        preset_id,
        rule_id,
        UpdatePresetRule {
            rule: req.rule,
            position: req.position,
            last_edited_by: Some(session.user_id()),
            last_edited_at: Some(now),
        },
        &mut conn,
    )
    .await?;
    Ok(Json(rule))
}

#[rocket::delete("/presets/<preset_id>/rules/<rule_id>")]
async fn delete_preset_rule(
    _session: LoggedInSession,
    preset_id: i32,
    rule_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    db::delete_rule(preset_id, rule_id, &mut conn).await?;
    Ok(())
}

#[derive(Serialize)]
struct ReviewStatusResponse {
    room_id: Uuid,
    yaml_id: Uuid,
    status: String,
    changed_by: i64,
    changed_by_name: Option<String>,
    changed_at: String,
}

#[rocket::get("/review/<room_id>/statuses")]
async fn get_review_statuses(
    _session: LoggedInSession,
    room_id: &str,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<ReviewStatusResponse>>> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let statuses = db::get_review_statuses(room_id, &mut conn).await?;

    let user_ids: Vec<i64> = statuses.iter().map(|s| s.changed_by).collect();
    let usernames = db::get_editor_usernames(&user_ids, &mut conn).await?;
    let username_map: std::collections::HashMap<i64, String> = usernames.into_iter().collect();

    let response: Vec<ReviewStatusResponse> = statuses
        .into_iter()
        .map(|s| {
            let name = username_map.get(&s.changed_by).cloned();
            ReviewStatusResponse {
                room_id: s.room_id,
                yaml_id: s.yaml_id,
                status: s.status,
                changed_by: s.changed_by,
                changed_by_name: name,
                changed_at: s.changed_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(response))
}

#[derive(Deserialize)]
struct SetReviewStatusRequest {
    status: String,
}

#[rocket::put("/review/<room_id>/status/<yaml_id>", data = "<body>")]
async fn set_review_status(
    session: LoggedInSession,
    room_id: &str,
    yaml_id: &str,
    body: Json<SetReviewStatusRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<ReviewStatusResponse>> {
    let room_id: Uuid = room_id.parse().map_err(|_| anyhow!("Invalid room ID"))?;
    let yaml_id: Uuid = yaml_id.parse().map_err(|_| anyhow!("Invalid YAML ID"))?;
    let req = body.into_inner();

    let valid_statuses = ["unreviewed", "reported", "ok"];
    if !valid_statuses.contains(&req.status.as_str()) {
        return Err(anyhow!("Invalid status: {}", req.status).into());
    }

    let user_id = session.user_id();
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let status = db::set_review_status(room_id, yaml_id, &req.status, user_id, &mut conn).await?;
    let username = db::get_editor_username(user_id, &mut conn).await?;

    Ok(Json(ReviewStatusResponse {
        room_id: status.room_id,
        yaml_id: status.yaml_id,
        status: status.status,
        changed_by: status.changed_by,
        changed_by_name: username,
        changed_at: status.changed_at.to_rfc3339(),
    }))
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        list_presets,
        get_preset,
        create_preset,
        update_preset,
        delete_preset,
        list_builtin_rules,
        evaluate,
        list_preset_rules,
        create_preset_rule,
        update_preset_rule,
        delete_preset_rule,
        get_room_preset,
        set_room_preset,
        remove_room_preset,
        proxy_games,
        proxy_game_options,
        get_review_statuses,
        set_review_status,
    ]
}
