Archipelago lobby
=================

This project provides a lobby to collect yaml files from players to be able to
host archipelagoes easily.

# Running this project


```
docker compose build
./start.sh
```

The first start will require you to create a discord application for oauth2. Follow the instructions.
The first start will also download all apworlds in the index which might take a while.

## Discord oauth

The discord oauth is configured in `Rocket.toml` file

```toml
[default.oauth.discord]
provider = "Discord"
client_id="<your_client_id>"
client_secret="<your_client_secret>"
redirect_uri="http://127.0.0.1:8000/auth/oauth" # Switch this to your redirect URI
admins = [<discord_id_of_admin>, ...]
```

## Caveats

When working on the `ap-worker`, if you change the python dependencies, you
have to rerun `docker compose build` and restart everything.
