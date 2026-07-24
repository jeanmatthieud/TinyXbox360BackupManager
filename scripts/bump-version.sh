#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Bump the workspace version in Cargo.toml and resync Cargo.lock.
# Invoked by semantic-release (@semantic-release/exec prepareCmd) with the
# next version as the sole argument. Requires `yq` and `cargo` on PATH.
set -euo pipefail

VERSION="${1:?usage: bump-version.sh <version>}"

yq -i ".workspace.package.version = \"${VERSION}\"" Cargo.toml
cargo update -p txbm-core -p txbm-gui
