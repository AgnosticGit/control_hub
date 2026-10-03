#!/usr/bin/env bash
# One-time preparation of a ControlHub host (Ubuntu 22.04/24.04), run by the deploy workflow
# with `setup_hosts` or by hand: Docker with the Compose plugin from the Ubuntu archive, a swap
# file (1 GB hosts need it), and /opt/controlhub owned by the deploy user. Idempotent.
set -euo pipefail

DEPLOY_USER="${1:-$(id -un)}"
SWAP_SIZE="${SWAP_SIZE:-2G}"

if ! command -v docker >/dev/null || ! docker compose version >/dev/null 2>&1; then
  sudo apt-get update -y
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y docker.io docker-compose-v2
fi
sudo systemctl enable --now docker
sudo usermod -aG docker "$DEPLOY_USER"

if ! swapon --show=NAME --noheadings | grep -q .; then
  sudo fallocate -l "$SWAP_SIZE" /swapfile
  sudo chmod 600 /swapfile
  sudo mkswap /swapfile
  sudo swapon /swapfile
  grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab >/dev/null
fi
# Prefer RAM, swap only under pressure.
echo 'vm.swappiness=10' | sudo tee /etc/sysctl.d/90-controlhub.conf >/dev/null
sudo sysctl -q --system

sudo install -d -m 0750 -o "$DEPLOY_USER" -g "$DEPLOY_USER" /opt/controlhub
echo "host ready: $(docker --version), $(docker compose version), swap $(swapon --show=SIZE --noheadings | head -n1)"
