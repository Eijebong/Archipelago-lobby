use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::schema::{discord_users, review_preset_rules, review_presets, room_review_config, yaml_review_status};

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = review_presets)]
pub struct ReviewPreset {
    pub id: i32,
    pub name: String,
    pub builtin_rules: serde_json::Value,
}

#[derive(Serialize, Debug)]
pub struct PresetSummary {
    pub id: i32,
    pub name: String,
}

#[derive(Insertable, Debug, Deserialize)]
#[diesel(table_name = review_presets)]
pub struct NewPreset {
    pub name: String,
    pub builtin_rules: serde_json::Value,
}

#[derive(AsChangeset, Debug, Deserialize)]
#[diesel(table_name = review_presets)]
pub struct UpdatePreset {
    pub name: Option<String>,
    pub builtin_rules: Option<serde_json::Value>,
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = review_preset_rules)]
pub struct PresetRule {
    pub id: i32,
    pub preset_id: i32,
    pub rule: serde_json::Value,
    pub position: i32,
    pub last_edited_by: Option<i64>,
    pub last_edited_at: Option<DateTime<Utc>>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = review_preset_rules)]
pub struct NewPresetRule {
    pub preset_id: i32,
    pub rule: serde_json::Value,
    pub position: i32,
    pub last_edited_by: Option<i64>,
    pub last_edited_at: Option<DateTime<Utc>>,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = review_preset_rules)]
pub struct UpdatePresetRule {
    pub rule: Option<serde_json::Value>,
    pub position: Option<i32>,
    pub last_edited_by: Option<i64>,
    pub last_edited_at: Option<DateTime<Utc>>,
}

pub async fn list_presets(
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<PresetSummary>> {
    let results = review_presets::table
        .select((review_presets::id, review_presets::name))
        .order_by(review_presets::name)
        .load::<(i32, String)>(conn)
        .await?;

    Ok(results
        .into_iter()
        .map(|(id, name)| PresetSummary { id, name })
        .collect())
}

pub async fn get_preset(
    preset_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<ReviewPreset> {
    Ok(review_presets::table
        .find(preset_id)
        .select(ReviewPreset::as_select())
        .get_result(conn)
        .await?)
}

pub async fn create_preset(
    new_preset: NewPreset,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<ReviewPreset> {
    Ok(diesel::insert_into(review_presets::table)
        .values(&new_preset)
        .returning(ReviewPreset::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn update_preset(
    preset_id: i32,
    changes: UpdatePreset,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<ReviewPreset> {
    Ok(diesel::update(review_presets::table.find(preset_id))
        .set(&changes)
        .returning(ReviewPreset::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn delete_preset(
    preset_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<()> {
    diesel::delete(review_presets::table.find(preset_id))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn list_rules_for_preset(
    preset_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<PresetRule>> {
    Ok(review_preset_rules::table
        .filter(review_preset_rules::preset_id.eq(preset_id))
        .order_by(review_preset_rules::position)
        .select(PresetRule::as_select())
        .load(conn)
        .await?)
}

pub async fn create_rule(
    new_rule: NewPresetRule,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<PresetRule> {
    Ok(diesel::insert_into(review_preset_rules::table)
        .values(&new_rule)
        .returning(PresetRule::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn update_rule(
    preset_id: i32,
    rule_id: i32,
    changes: UpdatePresetRule,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<PresetRule> {
    Ok(diesel::update(
        review_preset_rules::table
            .find(rule_id)
            .filter(review_preset_rules::preset_id.eq(preset_id)),
    )
    .set(&changes)
    .returning(PresetRule::as_returning())
    .get_result(conn)
    .await?)
}

pub async fn delete_rule(
    preset_id: i32,
    rule_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<()> {
    diesel::delete(
        review_preset_rules::table
            .find(rule_id)
            .filter(review_preset_rules::preset_id.eq(preset_id)),
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = room_review_config)]
pub struct RoomReviewConfig {
    pub room_id: Uuid,
    pub preset_id: i32,
}

#[derive(Insertable)]
#[diesel(table_name = room_review_config)]
struct NewRoomReviewConfig {
    room_id: Uuid,
    preset_id: i32,
}

pub async fn get_room_config(
    room_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Option<RoomReviewConfig>> {
    Ok(room_review_config::table
        .find(room_id)
        .select(RoomReviewConfig::as_select())
        .get_result(conn)
        .await
        .optional()?)
}

pub async fn set_room_preset(
    room_id: Uuid,
    preset_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<()> {
    diesel::insert_into(room_review_config::table)
        .values(&NewRoomReviewConfig { room_id, preset_id })
        .on_conflict(room_review_config::room_id)
        .do_update()
        .set(room_review_config::preset_id.eq(preset_id))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn remove_room_preset(
    room_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<()> {
    diesel::delete(room_review_config::table.find(room_id))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn get_editor_username(
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Option<String>> {
    Ok(discord_users::table
        .find(user_id)
        .select(discord_users::username)
        .get_result::<String>(conn)
        .await
        .optional()?)
}

pub async fn get_editor_usernames(
    user_ids: &[i64],
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<(i64, String)>> {
    Ok(discord_users::table
        .filter(discord_users::id.eq_any(user_ids))
        .select((discord_users::id, discord_users::username))
        .load(conn)
        .await?)
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = yaml_review_status)]
pub struct YamlReviewStatus {
    pub room_id: Uuid,
    pub yaml_id: Uuid,
    pub status: String,
    pub changed_by: i64,
    pub changed_at: DateTime<Utc>,
}

#[derive(Insertable)]
#[diesel(table_name = yaml_review_status)]
struct NewYamlReviewStatus {
    room_id: Uuid,
    yaml_id: Uuid,
    status: String,
    changed_by: i64,
}

pub async fn get_review_statuses(
    room_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<YamlReviewStatus>> {
    Ok(yaml_review_status::table
        .filter(yaml_review_status::room_id.eq(room_id))
        .select(YamlReviewStatus::as_select())
        .load(conn)
        .await?)
}

pub async fn set_review_status(
    room_id: Uuid,
    yaml_id: Uuid,
    status: &str,
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<YamlReviewStatus> {
    Ok(diesel::insert_into(yaml_review_status::table)
        .values(&NewYamlReviewStatus {
            room_id,
            yaml_id,
            status: status.to_string(),
            changed_by: user_id,
        })
        .on_conflict((yaml_review_status::room_id, yaml_review_status::yaml_id))
        .do_update()
        .set((
            yaml_review_status::status.eq(status),
            yaml_review_status::changed_by.eq(user_id),
            yaml_review_status::changed_at.eq(diesel::dsl::now),
        ))
        .returning(YamlReviewStatus::as_returning())
        .get_result(conn)
        .await?)
}
