#!/bin/bash

set -e

apt update && apt install -y curl
curl -fsSL https://apt.cli.rs/pubkey.asc | tee -a /usr/share/keyrings/rust-tools.asc
curl -fsSL https://apt.cli.rs/rust-tools.list -o /etc/apt/sources.list.d/rust-tools.list
printf "deb http://httpredir.debian.org/debian bookworm-backports main" > /etc/apt/sources.list.d/backports.list
apt update && apt install -y libpq-dev valkey python3 git libssl-dev pkg-config watchexec-cli liblzma-dev zlib1g-dev
apt autoremove -y
rm -rf /var/lib/apt/lists/*

rustup component add clippy rustfmt

# Add worker user
mkdir -p /builds
useradd -d /builds/worker -s /bin/bash -m worker
mkdir -p /builds/worker/artifacts
chown -R worker:worker /builds/worker
