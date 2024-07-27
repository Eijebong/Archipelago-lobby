Archipelago lobby
=================

This project provides a lobby to collect yaml files from players to be able to
host archipelagoes easily.

# Running this project


## Start the database

```
docker compose up -d
```

```
export DATABASE_URL="postgres:///postgres:postgres@127.0.0.1:25432/aplobby"
export ADMIN_TOKEN="theadmintoken"
export APWORLDS_INDEX_DIR="../apworlds_index"
export APWORLDS_INDEX_REPO_URL="https://github.com/Eijebong/Archipelago-index.git"
export APWORLDS_PATH="../apworlds_index/worlds"
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
## YAML checking

When uploading a YAML to a lobby, you can opt-in to validate YAMLs. It will use
this service, https://github.com/Eijebong/Archipelago-yaml-checker, just point `YAML_VALIDATOR_URL` to it.

`export YAML_VALIDATOR_URL="http://127.0.0.1:5000"`

## APWorlds list

You need to provide an `index.toml` so the project knows which apworlds to
display on the APWorlds page. This index should contain all apworlds you're
using in the yaml checking service.

You also need to provide it the apworlds folder you're using for yaml checking
so it can provide them for download.

This is done through three environment variables, `APWORLDS_INDEX_DIR`,
`APWORLDS_INDEX_REPO_URL` and `APWORLDS_PATH`.

`APWORLDS_INDEX_DIR` should point to the index folder (not the toml itself). It
will be created and initialized by the lobby.
`APWORLDS_PATH` should point to the apworlds folder. It will also get initialized by the lobby
`APWORLDS_INDEX_REPO_URL` needs to point to a git repository containing a valid index.

Index documentation and management tools can be found at https://github.com/Eijebong/apwm
You can find my own index at https://github.com/Eijebong/Archipelago-index

