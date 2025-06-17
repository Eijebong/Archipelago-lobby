#!/bin/bash

set -e

printf "deb http://httpredir.debian.org/debian bookworm-backports main" > /etc/apt/sources.list.d/backports.list
apt update && apt install -y libpq-dev valkey python3 git
apt autoremove -y
rm -rf /var/lib/apt/lists/*

rustup component add clippy rustfmt

cargo install cargo-watch

rm -Rf /usr/local/cargo
# Add worker user
mkdir -p /builds
useradd -d /builds/worker -s /bin/bash -m worker
mkdir -p /builds/worker/artifacts
chown -R worker:worker /builds/worker
