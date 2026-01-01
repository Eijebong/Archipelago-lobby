#!/bin/bash

set -e

apt update && apt install -y curl
curl -fsSL https://github.com/watchexec/watchexec/releases/download/cli-v1.20.5/watchexec-1.20.5-x86_64-unknown-linux-gnu.deb -o /tmp/watchexec.deb
dpkg -i /tmp/watchexec.deb
cat > /etc/apt/sources.list.d/debian-backports.sources <<EOF
Types: deb deb-src
URIs: http://deb.debian.org/debian
Suites: bookworm-backports
Components: main
Enabled: yes
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
EOF
apt update && apt install -y --no-install-recommends libpq-dev valkey python3 git libssl-dev pkg-config watchexec-cli liblzma-dev zlib1g-dev mold
apt autoremove -y
rm -rf /var/lib/apt/lists/*

rustup component add clippy rustfmt

# Add worker user
mkdir -p /builds
useradd -d /builds/worker -s /bin/bash -m worker
mkdir -p /builds/worker/artifacts
chown -R worker:worker /builds/worker
