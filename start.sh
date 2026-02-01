#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

DC="$SCRIPT_DIR/dc.sh"

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

    cat > Rocket.community.toml <<EOF
[default.oauth.discord]
provider = "Discord"
client_id = "$_client_id"
client_secret = "$_client_secret"
redirect_uri = "http://127.0.0.1:8001/auth/oauth"
admins = [$_admin_id]
EOF

fi

COMMUNITY=false
ARGS=()

for arg in "$@"; do
    case "$arg" in
        --community)
            COMMUNITY=true
            ;;
        *)
            ARGS+=("$arg")
            ;;
    esac
done

if [[ "$COMMUNITY" == "true" ]]; then
    if [[ -z "$EXTRA_COMPOSE" ]]; then
        echo "error: --community requires EXTRA_COMPOSE to be set"
        exit 1
    fi

    echo "== Starting base services =="
    "$DC" up -d postgres valkey
    "$DC" up -d vol-setup
    echo "Waiting for postgres and valkey..."
    "$DC" exec postgres sh -c "until pg_isready -U postgres; do sleep 1; done" > /dev/null 2>&1
    "$DC" exec valkey sh -c "until valkey-cli ping | grep -q PONG; do sleep 1; done" > /dev/null 2>&1

    # Create APX database if it doesn't exist
    "$DC" exec postgres psql -U postgres -tc "SELECT 1 FROM pg_database WHERE datname = 'apx'" | grep -q 1 \
        || "$DC" exec postgres psql -U postgres -c "CREATE DATABASE apx"

    echo "== Starting lobby =="
    "$DC" up -d lobby
    echo "Waiting for lobby to be healthy..."
    until "$DC" exec lobby curl -sf http://127.0.0.1:8000/health > /dev/null 2>&1; do
        sleep 2
    done
    echo "Lobby is up."

    echo "== Starting workers =="
    "$DC" up -d yaml-checker generator option-generator

    echo "== Starting AP WebHost =="
    "$DC" up -d ap-webhost
    echo "Waiting for AP WebHost..."
    until "$DC" exec ap-webhost curl -sf http://127.0.0.1:9888/ > /dev/null 2>&1; do
        sleep 2
    done
    echo "AP WebHost is up at http://localhost:9888"

    if [[ ! -f ".env.community" ]]; then
        echo ""
        echo "== Community setup =="
        echo "1. Go to http://localhost:8000 and create a room in the lobby"
        echo "2. Upload YAMLs and generate the game"
        echo "3. Go to http://localhost:9888 and host the generated game"
        echo ""
        echo "Paste the lobby room ID:"
        read -r LOBBY_ROOM_ID
        echo "Paste the AP room ID:"
        read -r AP_ROOM_ID
        echo "Paste the AP room port:"
        read -r AP_ROOM_PORT
        echo "Paste the AP session cookie (with session=):"
        read -r AP_SESSION_COOKIE

        cat > .env.community <<-EOF
		LOBBY_ROOM_ID=$LOBBY_ROOM_ID
		AP_ROOM_ID=$AP_ROOM_ID
		AP_ROOM_PORT=$AP_ROOM_PORT
		AP_SESSION_COOKIE=$AP_SESSION_COOKIE
		EOF
        echo "Saved to .env.community"
    fi

    set -a
    . ./.env.community
    set +a

    echo "== Starting APX =="
    "$DC" up -d apx

    echo "== Starting community-ap-tools =="
    "$DC" up "${ARGS[@]}" community-ap-tools
else
    "$DC" up "${ARGS[@]}"
fi
