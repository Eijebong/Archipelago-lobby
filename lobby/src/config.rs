use crate::error::Result;
use anyhow::anyhow;
use rocket::figment::{value::Value, Figment, Profile, Provider};

pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    pub admins: Vec<Value>,
    pub banned_users: Vec<i64>,
}

impl DiscordConfig {
    pub fn from_figment(config: &Figment) -> Result<Self> {
        let config = config.data()?;
        let discord_config = config
            .get(&Profile::Default)
            .ok_or(anyhow!("No default profile in config"))?
            .get("oauth")
            .ok_or(anyhow!("No oauth section in default profile"))?
            .as_dict()
            .ok_or(anyhow!("oauth section isn't a map"))?
            .get("discord")
            .ok_or(anyhow!("no discord section in oauth"))?
            .as_dict()
            .ok_or(anyhow!("discord section isn't a dict"))?;
        let client_id = discord_config
            .get("client_id")
            .ok_or(anyhow!("client id not present in discord config"))?
            .as_str()
            .ok_or(anyhow!("client id isn't a string"))?;
        let client_secret = discord_config
            .get("client_secret")
            .ok_or(anyhow!("client secret not present in discord config"))?
            .as_str()
            .ok_or(anyhow!("client secret isn't a string"))?;

        let admins = discord_config
            .get("admins")
            .ok_or(anyhow!("no admins in discord section"))?
            .as_array()
            .ok_or(anyhow!("admins isn't an array"))?;

        let banned_users: Vec<i64> = discord_config
            .get("banned_users")
            .cloned()
            .unwrap_or_else(|| <Value as From<Vec<i64>>>::from(vec![]))
            .deserialize()?;

        Ok(Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            admins: admins.iter().cloned().collect::<Vec<_>>(),
            banned_users,
        })
    }
}
