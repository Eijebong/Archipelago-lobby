use anyhow::anyhow;
use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool as DieselPool;
use rocket::{State, routes, serde::json::Json};
use serde::Deserialize;
use uuid::Uuid;

use crate::Config;
use crate::auth::{AdminSession, LoggedInSession};
use crate::error;
use crate::review::Role;
use crate::review::db;

#[derive(Deserialize)]
struct CreateTeamRequest {
    name: String,
    guild_id: i64,
}

#[derive(Deserialize)]
struct LobbyRoomOwnership {
    author_id: i64,
}

#[rocket::get("/teams")]
async fn admin_list_teams(
    session: LoggedInSession,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::Team>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    if session.is_super_admin() {
        return Ok(Json(db::list_teams(&mut conn).await?));
    }
    let user_teams = db::get_user_teams(session.user_id(), &mut conn).await?;
    let teams: Vec<db::Team> = user_teams
        .into_iter()
        .filter(|(_, m)| m.role.parse::<Role>().ok() >= Some(Role::Admin))
        .map(|(t, _)| t)
        .collect();
    Ok(Json(teams))
}

#[rocket::post("/teams", data = "<body>")]
async fn admin_create_team(
    _session: AdminSession,
    body: Json<CreateTeamRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::Team>> {
    let req = body.into_inner();
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    let team = db::create_team(
        db::NewTeam {
            name: req.name,
            guild_id: req.guild_id,
        },
        &mut conn,
    )
    .await?;
    Ok(Json(team))
}

#[rocket::delete("/teams/<id>")]
async fn admin_delete_team(
    _session: AdminSession,
    id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    db::delete_team(id, &mut conn).await?;
    Ok(())
}

#[rocket::get("/teams/<team_id>/members")]
async fn admin_list_members(
    session: LoggedInSession,
    team_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::TeamMember>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    Ok(Json(db::list_team_members(team_id, &mut conn).await?))
}

#[derive(Deserialize)]
struct AddMemberRequest {
    user_id: i64,
    username: Option<String>,
    role: String,
}

#[rocket::post("/teams/<team_id>/members", data = "<body>")]
async fn admin_add_member(
    session: LoggedInSession,
    team_id: i32,
    body: Json<AddMemberRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::TeamMember>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    let req = body.into_inner();
    let role: Role = req
        .role
        .parse()
        .map_err(|_| error::bad_request(format!("Invalid role: {}", req.role)))?;
    if role >= Role::Admin && !session.is_super_admin() {
        return Err(error::forbidden(
            "Only super admins can assign the admin role",
        ));
    }
    let member = db::add_team_member(
        team_id,
        req.user_id,
        req.username.as_deref(),
        role.as_str(),
        &mut conn,
    )
    .await?;
    Ok(Json(member))
}

#[derive(Deserialize)]
struct UpdateMemberRoleRequest {
    role: String,
}

#[rocket::put("/teams/<team_id>/members/<user_id>", data = "<body>")]
async fn admin_update_member_role(
    session: LoggedInSession,
    team_id: i32,
    user_id: i64,
    body: Json<UpdateMemberRoleRequest>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::TeamMember>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    let req = body.into_inner();
    let new_role: Role = req
        .role
        .parse()
        .map_err(|_| error::bad_request(format!("Invalid role: {}", req.role)))?;
    if !session.is_super_admin() {
        if new_role >= Role::Admin {
            return Err(error::forbidden(
                "Only super admins can assign the admin role",
            ));
        }
        let target_role = db::get_user_role_for_team(user_id, team_id, &mut conn).await?;
        if target_role >= Some(Role::Admin) {
            return Err(error::forbidden(
                "Only super admins can modify admin members",
            ));
        }
    }
    let member =
        db::update_team_member_role(team_id, user_id, new_role.as_str(), &mut conn).await?;
    Ok(Json(member))
}

#[rocket::delete("/teams/<team_id>/members/<user_id>")]
async fn admin_remove_member(
    session: LoggedInSession,
    team_id: i32,
    user_id: i64,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    if !session.is_super_admin() {
        let target_role = db::get_user_role_for_team(user_id, team_id, &mut conn).await?;
        if target_role >= Some(Role::Admin) {
            return Err(error::forbidden(
                "Only super admins can remove admin members",
            ));
        }
    }
    db::remove_team_member(team_id, user_id, &mut conn).await?;
    Ok(())
}

#[rocket::get("/teams/<team_id>/rooms")]
async fn admin_list_rooms(
    session: LoggedInSession,
    team_id: i32,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<Vec<db::TeamRoom>>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    Ok(Json(db::list_team_rooms(team_id, &mut conn).await?))
}

#[derive(Deserialize)]
struct AddRoomRequest {
    room_id: Uuid,
}

#[rocket::post("/teams/<team_id>/rooms", data = "<body>")]
async fn admin_add_room(
    session: LoggedInSession,
    team_id: i32,
    body: Json<AddRoomRequest>,
    config: &State<Config>,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<Json<db::TeamRoom>> {
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    let req = body.into_inner();

    if !session.is_super_admin() {
        let client = reqwest::Client::new();
        let url = config
            .lobby_root_url
            .join(&format!("/api/room/{}", req.room_id))?;
        let resp = client
            .get(url)
            .header("x-api-key", &config.lobby_api_key)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(error::not_found("Room not found on the lobby"));
        }
        let room_info: LobbyRoomOwnership = resp.json().await?;
        if room_info.author_id != session.user_id() {
            return Err(error::forbidden(
                "You can only add rooms you own on the lobby",
            ));
        }
    }

    let room = db::add_team_room(team_id, req.room_id, &mut conn).await?;
    Ok(Json(room))
}

#[rocket::delete("/teams/<team_id>/rooms/<room_id>")]
async fn admin_remove_room(
    session: LoggedInSession,
    team_id: i32,
    room_id: &str,
    pool: &State<DieselPool<AsyncPgConnection>>,
) -> crate::error::Result<()> {
    let room_id: Uuid = room_id
        .parse()
        .map_err(|_| error::bad_request("Invalid room ID"))?;
    let mut conn = pool.get().await.map_err(|e| anyhow!(e))?;
    session
        .require_team_role(team_id, Role::Admin, &mut conn)
        .await?;
    db::remove_team_room(team_id, room_id, &mut conn).await?;
    Ok(())
}

pub fn routes() -> Vec<rocket::Route> {
    routes![
        admin_list_teams,
        admin_create_team,
        admin_delete_team,
        admin_list_members,
        admin_add_member,
        admin_update_member_role,
        admin_remove_member,
        admin_list_rooms,
        admin_add_room,
        admin_remove_room,
    ]
}
