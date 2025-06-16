#!/bin/bash

export USER_ID=$(id -u)
export GID=$(id -g)

if [[ ! -f "./Rocket.toml" ]]; then
    echo "== FIRST SETUP =="
    echo "Go to https://discord.com/developers/applications"
    echo "Create an application (or select an existing one)"
    echo "In oauth2 add a redirect to \"http://127.0.0.1:8000/auth/oauth\""
    echo "Click on \"save changes\" then \"Reset Secret\""

    echo "Paste the client ID:"
    read -r _client_id
    echo "Paste the client secret:"
    read -r _client_secret
    echo "Paste your discord user ID:"
    read -r _admin_id

    cat > Rocket.toml <<EOF
[default.oauth.discord]
provider = "Discord"
client_id = "$_client_id"
client_secret = "$_client_secret"
redirect_uri = "http://127.0.0.1:8000/auth/oauth"
admins = [$_admin_id]
EOF

fi

docker compose up $@
