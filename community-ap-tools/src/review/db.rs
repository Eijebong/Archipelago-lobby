use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::schema::{
    review_preset_rules, review_presets, room_review_config, team_members, team_rooms, teams,
    yaml_review_notes, yaml_review_status,
};

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = review_presets)]
pub struct ReviewPreset {
    pub id: i32,
    pub name: String,
    pub builtin_rules: serde_json::Value,
    pub team_id: Option<i32>,
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
    #[serde(default)]
    pub team_id: Option<i32>,
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
    pub last_edited_by_name: Option<String>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = review_preset_rules)]
pub struct NewPresetRule {
    pub preset_id: i32,
    pub rule: serde_json::Value,
    pub position: i32,
    pub last_edited_by: Option<i64>,
    pub last_edited_at: Option<DateTime<Utc>>,
    pub last_edited_by_name: Option<String>,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = review_preset_rules)]
pub struct UpdatePresetRule {
    pub rule: Option<serde_json::Value>,
    pub position: Option<i32>,
    pub last_edited_by: Option<i64>,
    pub last_edited_at: Option<DateTime<Utc>>,
    pub last_edited_by_name: Option<String>,
}

pub async fn list_presets(conn: &mut AsyncPgConnection) -> anyhow::Result<Vec<PresetSummary>> {
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

pub async fn delete_preset(preset_id: i32, conn: &mut AsyncPgConnection) -> anyhow::Result<()> {
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

pub async fn remove_room_preset(room_id: Uuid, conn: &mut AsyncPgConnection) -> anyhow::Result<()> {
    diesel::delete(room_review_config::table.find(room_id))
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = yaml_review_status)]
pub struct YamlReviewStatus {
    pub room_id: Uuid,
    pub yaml_id: Uuid,
    pub status: String,
    pub changed_by: i64,
    pub changed_at: DateTime<Utc>,
    pub changed_by_name: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = yaml_review_status)]
struct NewYamlReviewStatus {
    room_id: Uuid,
    yaml_id: Uuid,
    status: String,
    changed_by: i64,
    changed_by_name: Option<String>,
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
    username: &str,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<YamlReviewStatus> {
    Ok(diesel::insert_into(yaml_review_status::table)
        .values(&NewYamlReviewStatus {
            room_id,
            yaml_id,
            status: status.to_string(),
            changed_by: user_id,
            changed_by_name: Some(username.to_string()),
        })
        .on_conflict((yaml_review_status::room_id, yaml_review_status::yaml_id))
        .do_update()
        .set((
            yaml_review_status::status.eq(status),
            yaml_review_status::changed_by.eq(user_id),
            yaml_review_status::changed_by_name.eq(username),
            yaml_review_status::changed_at.eq(diesel::dsl::now),
        ))
        .returning(YamlReviewStatus::as_returning())
        .get_result(conn)
        .await?)
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = yaml_review_notes)]
pub struct YamlReviewNote {
    pub id: i32,
    pub room_id: Uuid,
    pub yaml_id: Uuid,
    pub content: String,
    pub author_id: i64,
    pub author_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Insertable)]
#[diesel(table_name = yaml_review_notes)]
struct NewYamlReviewNote {
    room_id: Uuid,
    yaml_id: Uuid,
    content: String,
    author_id: i64,
    author_name: Option<String>,
}

pub async fn get_notes(
    room_id: Uuid,
    yaml_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<YamlReviewNote>> {
    Ok(yaml_review_notes::table
        .filter(yaml_review_notes::room_id.eq(room_id))
        .filter(yaml_review_notes::yaml_id.eq(yaml_id))
        .order_by(yaml_review_notes::created_at)
        .select(YamlReviewNote::as_select())
        .load(conn)
        .await?)
}

pub async fn add_note(
    room_id: Uuid,
    yaml_id: Uuid,
    content: &str,
    author_id: i64,
    author_name: &str,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<YamlReviewNote> {
    Ok(diesel::insert_into(yaml_review_notes::table)
        .values(&NewYamlReviewNote {
            room_id,
            yaml_id,
            content: content.to_string(),
            author_id,
            author_name: Some(author_name.to_string()),
        })
        .returning(YamlReviewNote::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn delete_note(
    note_id: i32,
    room_id: Uuid,
    author_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<usize> {
    Ok(diesel::delete(
        yaml_review_notes::table
            .find(note_id)
            .filter(yaml_review_notes::room_id.eq(room_id))
            .filter(yaml_review_notes::author_id.eq(author_id)),
    )
    .execute(conn)
    .await?)
}

// --- Teams ---

#[derive(Queryable, Selectable, Serialize, Debug, Clone)]
#[diesel(table_name = teams)]
pub struct Team {
    pub id: i32,
    pub name: String,
    pub guild_id: i64,
}

#[derive(Insertable, Debug, Deserialize)]
#[diesel(table_name = teams)]
pub struct NewTeam {
    pub name: String,
    pub guild_id: i64,
}

#[derive(Queryable, Selectable, Serialize, Debug, Clone)]
#[diesel(table_name = team_members)]
pub struct TeamMember {
    pub team_id: i32,
    pub user_id: i64,
    pub username: Option<String>,
    pub role: String,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = team_members)]
struct NewTeamMember {
    team_id: i32,
    user_id: i64,
    username: Option<String>,
    role: String,
}

#[derive(Queryable, Selectable, Serialize, Debug, Clone)]
#[diesel(table_name = team_rooms)]
pub struct TeamRoom {
    pub team_id: i32,
    pub room_id: Uuid,
    pub room_name: String,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = team_rooms)]
struct NewTeamRoom {
    team_id: i32,
    room_id: Uuid,
    room_name: String,
}

pub async fn list_teams(conn: &mut AsyncPgConnection) -> anyhow::Result<Vec<Team>> {
    Ok(teams::table
        .select(Team::as_select())
        .order_by(teams::name)
        .load(conn)
        .await?)
}

pub async fn create_team(new_team: NewTeam, conn: &mut AsyncPgConnection) -> anyhow::Result<Team> {
    Ok(diesel::insert_into(teams::table)
        .values(&new_team)
        .returning(Team::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn delete_team(id: i32, conn: &mut AsyncPgConnection) -> anyhow::Result<usize> {
    Ok(diesel::delete(teams::table.find(id)).execute(conn).await?)
}

pub async fn list_team_members(
    team_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<TeamMember>> {
    Ok(team_members::table
        .filter(team_members::team_id.eq(team_id))
        .select(TeamMember::as_select())
        .order_by(team_members::username)
        .load(conn)
        .await?)
}

pub async fn add_team_member(
    team_id: i32,
    user_id: i64,
    username: Option<&str>,
    role: &str,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<TeamMember> {
    Ok(diesel::insert_into(team_members::table)
        .values(&NewTeamMember {
            team_id,
            user_id,
            username: username.map(|s| s.to_string()),
            role: role.to_string(),
        })
        .on_conflict((team_members::team_id, team_members::user_id))
        .do_update()
        .set((
            team_members::role.eq(role),
            team_members::username.eq(username),
        ))
        .returning(TeamMember::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn remove_team_member(
    team_id: i32,
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<usize> {
    Ok(diesel::delete(team_members::table.find((team_id, user_id)))
        .execute(conn)
        .await?)
}

pub async fn update_team_member_role(
    team_id: i32,
    user_id: i64,
    role: &str,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<TeamMember> {
    Ok(diesel::update(team_members::table.find((team_id, user_id)))
        .set(team_members::role.eq(role))
        .returning(TeamMember::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn list_team_rooms(
    team_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<TeamRoom>> {
    Ok(team_rooms::table
        .filter(team_rooms::team_id.eq(team_id))
        .select(TeamRoom::as_select())
        .load(conn)
        .await?)
}

pub async fn add_team_room(
    team_id: i32,
    room_id: Uuid,
    room_name: String,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<TeamRoom> {
    Ok(diesel::insert_into(team_rooms::table)
        .values(&NewTeamRoom {
            team_id,
            room_id,
            room_name: room_name.clone(),
        })
        .on_conflict((team_rooms::team_id, team_rooms::room_id))
        .do_update()
        .set(team_rooms::room_name.eq(room_name))
        .returning(TeamRoom::as_returning())
        .get_result(conn)
        .await?)
}

pub async fn update_room_name(
    room_id: Uuid,
    room_name: &str,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<()> {
    diesel::update(
        team_rooms::table
            .filter(team_rooms::room_id.eq(room_id))
            .filter(team_rooms::room_name.ne(room_name)),
    )
    .set(team_rooms::room_name.eq(room_name))
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn remove_team_room(
    team_id: i32,
    room_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<usize> {
    Ok(diesel::delete(team_rooms::table.find((team_id, room_id)))
        .execute(conn)
        .await?)
}

pub async fn get_user_teams(
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<(Team, TeamMember)>> {
    Ok(teams::table
        .inner_join(team_members::table)
        .filter(team_members::user_id.eq(user_id))
        .select((Team::as_select(), TeamMember::as_select()))
        .load(conn)
        .await?)
}

pub async fn get_user_role_for_team(
    user_id: i64,
    team_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Option<super::Role>> {
    let role: Option<String> = team_members::table
        .find((team_id, user_id))
        .select(team_members::role)
        .get_result(conn)
        .await
        .optional()?;

    Ok(role.and_then(|r| r.parse().ok()))
}

pub struct RoomSummary {
    pub room_id: Uuid,
    pub room_name: String,
}

pub async fn list_all_rooms(conn: &mut AsyncPgConnection) -> anyhow::Result<Vec<RoomSummary>> {
    Ok(team_rooms::table
        .select((team_rooms::room_id, team_rooms::room_name))
        .distinct_on(team_rooms::room_id)
        .load::<(Uuid, String)>(conn)
        .await?
        .into_iter()
        .map(|(room_id, room_name)| RoomSummary { room_id, room_name })
        .collect())
}

pub async fn list_user_rooms(
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<RoomSummary>> {
    Ok(team_members::table
        .inner_join(team_rooms::table.on(team_rooms::team_id.eq(team_members::team_id)))
        .filter(team_members::user_id.eq(user_id))
        .select((team_rooms::room_id, team_rooms::room_name))
        .distinct_on(team_rooms::room_id)
        .load::<(Uuid, String)>(conn)
        .await?
        .into_iter()
        .map(|(room_id, room_name)| RoomSummary { room_id, room_name })
        .collect())
}

pub async fn get_user_role_for_room(
    user_id: i64,
    room_id: Uuid,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Option<super::Role>> {
    let roles: Vec<String> = team_members::table
        .inner_join(team_rooms::table.on(team_rooms::team_id.eq(team_members::team_id)))
        .filter(team_members::user_id.eq(user_id))
        .filter(team_rooms::room_id.eq(room_id))
        .select(team_members::role)
        .load(conn)
        .await?;

    Ok(roles.into_iter().filter_map(|r| r.parse().ok()).max())
}

pub async fn get_user_role_for_preset(
    user_id: i64,
    preset_id: i32,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Option<super::Role>> {
    let roles: Vec<String> = team_members::table
        .inner_join(
            review_presets::table.on(review_presets::team_id.eq(team_members::team_id.nullable())),
        )
        .filter(team_members::user_id.eq(user_id))
        .filter(review_presets::id.eq(preset_id))
        .select(team_members::role)
        .load(conn)
        .await?;

    Ok(roles.into_iter().filter_map(|r| r.parse().ok()).max())
}

pub async fn is_user_in_any_team(
    user_id: i64,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<bool> {
    use diesel::dsl::exists;
    let result: bool = diesel::select(exists(
        team_members::table.filter(team_members::user_id.eq(user_id)),
    ))
    .get_result(conn)
    .await?;
    Ok(result)
}

pub async fn list_presets_for_user(
    user_id: i64,
    is_super_admin: bool,
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<PresetSummary>> {
    if is_super_admin {
        return list_presets(conn).await;
    }
    let user_teams = get_user_teams(user_id, conn).await?;
    let team_ids: Vec<i32> = user_teams.iter().map(|(t, _)| t.id).collect();
    list_presets_for_teams(&team_ids, conn).await
}

pub async fn list_presets_for_teams(
    team_ids: &[i32],
    conn: &mut AsyncPgConnection,
) -> anyhow::Result<Vec<PresetSummary>> {
    let results = review_presets::table
        .filter(review_presets::team_id.eq_any(team_ids))
        .select((review_presets::id, review_presets::name))
        .order_by(review_presets::name)
        .load::<(i32, String)>(conn)
        .await?;

    Ok(results
        .into_iter()
        .map(|(id, name)| PresetSummary { id, name })
        .collect())
}
