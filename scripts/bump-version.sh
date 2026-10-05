#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Bump the workspace version in Cargo.toml, resync Cargo.lock and record the
# release in the AppStream metainfo.
# Invoked by semantic-release (@semantic-release/exec prepareCmd) with the
# next version as the sole argument. Requires `yq` and `cargo` on PATH.
set -euo pipefail

VERSION="${1:?usage: bump-version.sh <version>}"

yq -i ".workspace.package.version = \"${VERSION}\"" Cargo.toml
cargo update -p txbm-core -p txbm-gui

# The build jobs already did this on their checkout, but that is a throwaway
# workspace; this is the copy @semantic-release/git commits.
"$(dirname "${BASH_SOURCE[0]}")/set-metainfo-release.sh" "${VERSION}"
