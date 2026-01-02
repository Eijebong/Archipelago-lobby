use diesel::prelude::*;
use diesel::result::OptionalExtension;
use diesel::{Insertable, Queryable};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::error::Result;
use crate::schema::discord_users;

#[derive(Insertable, Queryable)]
#[diesel(table_name=discord_users)]
pub struct DiscordUser {
    pub id: i64,
    pub username: String,
}

#[tracing::instrument(skip(conn))]
pub async fn get_username(user_id: i64, conn: &mut AsyncPgConnection) -> Result<Option<String>> {
    let username = discord_users::table
        .filter(discord_users::id.eq(user_id))
        .select(discord_users::username)
        .first::<String>(conn)
        .await
        .optional()?;

    Ok(username)
}

#[tracing::instrument(skip(conn, discord_id), fields(%discord_id))]
pub async fn upsert_discord_user(
    discord_id: i64,
    username: &str,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let discord_user = DiscordUser {
        id: discord_id,
        username: username.to_string(),
    };

    diesel::insert_into(discord_users::table)
        .values(&discord_user)
        .on_conflict(discord_users::id)
        .do_update()
        .set(discord_users::username.eq(username))
        .execute(conn)
        .await?;

    Ok(())
}
