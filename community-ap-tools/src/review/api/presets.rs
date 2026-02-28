use anyhow::anyhow;
use chrono::Utc;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use rocket::{State, routes, serde::json::Json};
use serde::Deserialize;

use crate::auth::{AdminSession, LoggedInSession};
use crate::review::Role;
use crate::review::builtin;
use crate::review::db::{self, NewPreset, NewPresetRule, UpdatePreset, UpdatePresetRule};
use crate::review::rules::Rule;

#[rocket::get("/presets")]
async fn list_presets(
    session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::PresetSummary>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let presets =
        db::list_presets_for_user(session.user_id(), session.is_super_admin(), &mut conn).await?;
    Ok(Json(presets))
}

#[rocket::get("/presets/<id>")]
async fn get_preset(
    session: LoggedInSession,
    id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::ReviewPreset>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_preset_role(id, Role::Viewer, &mut conn)
        .await?;
    let preset = db::get_preset(id, &mut conn).await?;
    Ok(Json(preset))
}

#[rocket::post("/presets", data = "<body>")]
async fn create_preset(
    _session: AdminSession,
    body: Json<NewPreset>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::ReviewPreset>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let preset = db::create_preset(body.into_inner(), &mut conn).await?;
    Ok(Json(preset))
}

#[rocket::put("/presets/<id>", data = "<body>")]
async fn update_preset(
    _session: AdminSession,
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
    _session: AdminSession,
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

#[rocket::get("/presets/<preset_id>/rules")]
async fn list_preset_rules(
    session: LoggedInSession,
    preset_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::PresetRule>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_preset_role(preset_id, Role::Viewer, &mut conn)
        .await?;
    let rules = db::list_rules_for_preset(preset_id, &mut conn).await?;
    Ok(Json(rules))
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

#[rocket::post("/presets/<preset_id>/rules", data = "<body>")]
async fn create_preset_rule(
    session: LoggedInSession,
    preset_id: i32,
    body: Json<NewPresetRuleRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::PresetRule>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_preset_role(preset_id, Role::RuleEditor, &mut conn)
        .await?;
    let req = body.into_inner();
    let parsed: Rule =
        serde_json::from_value(req.rule.clone()).map_err(|e| anyhow!("Invalid rule: {}", e))?;
    parsed.validate().map_err(|e| anyhow!("{}", e))?;
    let now = Utc::now();
    let rule = db::create_rule(
        NewPresetRule {
            preset_id,
            rule: req.rule,
            position: req.position,
            last_edited_by: Some(session.user_id()),
            last_edited_at: Some(now),
            last_edited_by_name: Some(session.username().to_string()),
        },
        &mut conn,
    )
    .await?;
    Ok(Json(rule))
}

#[rocket::put("/presets/<preset_id>/rules/<rule_id>", data = "<body>")]
async fn update_preset_rule(
    session: LoggedInSession,
    preset_id: i32,
    rule_id: i32,
    body: Json<UpdatePresetRuleRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::PresetRule>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_preset_role(preset_id, Role::RuleEditor, &mut conn)
        .await?;
    let req = body.into_inner();
    if let Some(ref rule_value) = req.rule {
        let parsed: Rule = serde_json::from_value(rule_value.clone())
            .map_err(|e| anyhow!("Invalid rule: {}", e))?;
        parsed.validate().map_err(|e| anyhow!("{}", e))?;
    }
    let now = Utc::now();
    let rule = db::update_rule(
        preset_id,
        rule_id,
        UpdatePresetRule {
            rule: req.rule,
            position: req.position,
            last_edited_by: Some(session.user_id()),
            last_edited_at: Some(now),
            last_edited_by_name: Some(session.username().to_string()),
        },
        &mut conn,
    )
    .await?;
    Ok(Json(rule))
}

#[rocket::delete("/presets/<preset_id>/rules/<rule_id>")]
async fn delete_preset_rule(
    session: LoggedInSession,
    preset_id: i32,
    rule_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_preset_role(preset_id, Role::RuleEditor, &mut conn)
        .await?;
    db::delete_rule(preset_id, rule_id, &mut conn).await?;
    Ok(())
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        list_presets,
        get_preset,
        create_preset,
        update_preset,
        delete_preset,
        list_builtin_rules,
        list_preset_rules,
        create_preset_rule,
        update_preset_rule,
        delete_preset_rule,
    ]
}
