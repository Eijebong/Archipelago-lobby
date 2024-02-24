Archipelago lobby
=================

This project provides a lobby to collect yaml files from players to be able to
host archipelagoes easily.

# Running this project

```
export DATABASE_URL="sqlite:///path/to/your/db.sqlite"
export ADMIN_TOKEN="theadmintoken"
cargo run
```

If you want to run this in a production environment, make sure to set the following too:

```
ROCKET_ENV=production
ROCKET_SECRET_KEY="yoursecretkeyhere" # openssl rand -base64 32
```

## Discord oauth

To configure the discord oauth, create a `Rocket.toml` file in the same directory as the binary and include the following content:

```toml
[default.oauth.discord]
provider = "Discord"
client_id="<your_client_id>"
client_secret="<your_client_secret>"
redirect_uri="http://127.0.0.1:8000/auth/oauth" # Switch this to your redirect URI
admins = [<discord_id_of_admin>, ...]
```
