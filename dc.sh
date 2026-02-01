#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

export USER_ID=$(id -u)
export GID=$(id -g)

COMPOSE_CMD=(docker compose)
if [[ -n "$EXTRA_COMPOSE" ]]; then
    COMPOSE_CMD+=(--file docker-compose.yml --file "$EXTRA_COMPOSE")
fi

"${COMPOSE_CMD[@]}" "$@"
