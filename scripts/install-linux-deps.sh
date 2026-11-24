#!/usr/bin/env bash
# Installs the system packages Tauri 2 needs to build and bundle the app on
# Debian or Ubuntu. Used by CI and the release workflow; you can run it
# locally too.
#
# Based on https://v2.tauri.app/start/prerequisites/ (Linux, Debian), minus
# libssl-dev because reqwest uses rustls, plus the packages the AppImage
# bundler needs (patchelf, xdg-utils).
set -euo pipefail

packages=(
  build-essential
  curl
  wget
  file
  libwebkit2gtk-4.1-dev
  libxdo-dev
  libayatana-appindicator3-dev
  librsvg2-dev
  patchelf
  xdg-utils
)

sudo apt-get update
sudo apt-get install -y --no-install-recommends "${packages[@]}"
