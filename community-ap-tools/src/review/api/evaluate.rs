use anyhow::anyhow;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use rayon::prelude::*;
use rocket::{State, routes, serde::json::Json};
use saphyr::{LoadableYamlNode, YamlOwned as Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Config;
use crate::auth::LoggedInSession;
use crate::error;
use crate::review::Role;
use crate::review::builtin::{self, RoomYaml};
use crate::review::db;
use crate::review::rules::{self, Outcome, Predicate, Rule, Severity};
use crate::review::triggers;

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    games: Vec<String>,
    created_at: String,
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

#[derive(Deserialize, Serialize)]
struct BulkYamlInfo {
    id: Uuid,
    player_name: String,
    discord_handle: String,
    game: String,
    content: String,
    created_at: String,
    last_edited_by_name: Option<String>,
    last_edited_at: Option<String>,
}

#[rocket::post("/review/<room_id>/evaluate", data = "<body>")]
async fn evaluate(
    session: LoggedInSession,
    room_id: &str,
    body: Json<EvaluateRequest>,
    config: &State<Config>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<EvaluateResponse>> {
    let room_id: Uuid = room_id
        .parse()
        .map_err(|_| error::bad_request("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_room_role(room_id, Role::Viewer, &mut conn)
        .await?;

    let request = body.into_inner();

    let (custom_rules, enabled_builtins) = if let Some(preset_id) = request.preset_id {
        session
            .require_preset_role(preset_id, Role::Viewer, &mut conn)
            .await?;
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
    let url = config
        .lobby_root_url
        .join(&format!("/api/room/{}/yamls", room_id))?;
    let resp = client
        .get(url)
        .header("x-api-key", &config.lobby_api_key)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Failed to fetch room YAMLs: {}", resp.status()).into());
    }
    let bulk_yamls: Vec<BulkYamlInfo> = resp.json().await?;

    let room_yamls: Vec<RoomYaml> = bulk_yamls
        .iter()
        .map(|y| RoomYaml {
            player_name: y.player_name.clone(),
        })
        .collect();

    let builtin_rules_registry = builtin::builtin_rules();
    let active_builtins: Vec<&dyn builtin::BuiltinRule> = builtin_rules_registry
        .iter()
        .filter(|r| enabled_builtins.contains(&r.id().to_string()))
        .map(|r| r.as_ref())
        .collect();

    let results: Vec<YamlEvalResult> = bulk_yamls
        .par_iter()
        .map(|yaml| evaluate_single_yaml(yaml, &custom_rules, &active_builtins, &room_yamls))
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
    info: &BulkYamlInfo,
    custom_rules: &[Rule],
    active_builtins: &[&dyn builtin::BuiltinRule],
    room_yamls: &[RoomYaml],
) -> YamlEvalResult {
    let parsed = Value::load_from_str(&info.content)
        .ok()
        .and_then(|mut docs| docs.pop());

    let Some(yaml) = parsed else {
        return YamlEvalResult {
            yaml_id: info.id,
            player_name: info.player_name.clone(),
            discord_handle: info.discord_handle.clone(),
            game: info.game.clone(),
            games: vec![],
            created_at: info.created_at.clone(),
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
        vec![info.game.clone()]
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
            let br = builtin_rule.evaluate(&resolved, game_name, &info.player_name, room_yamls);
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

    let (display_game, games) = if multi_game {
        (format!("Random ({})", game_names.len()), game_names)
    } else {
        let single = game_names
            .into_iter()
            .next()
            .unwrap_or_else(|| info.game.clone());
        (single, vec![])
    };

    YamlEvalResult {
        yaml_id: info.id,
        player_name: info.player_name.clone(),
        discord_handle: info.discord_handle.clone(),
        game: display_game,
        games,
        created_at: info.created_at.clone(),
        results: all_results,
    }
}

pub fn routes() -> Vec<rocket::Route> {
    routes![evaluate]
}
